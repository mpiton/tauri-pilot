use crate::protocol::{Request, Response};

use anyhow::{Result, anyhow, bail};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Longest request line the plugin reads, trailing newline included.
///
/// Mirrors `MAX_LINE_LENGTH` in the plugin's `server/mod.rs`, which answers a
/// longer line with an error and closes the connection, often while the CLI
/// is still writing. The crates ship separately, so change both together.
pub(crate) const MAX_REQUEST_LEN: usize = 1_048_576;

/// How long a call waits for the app's answer unless `--rpc-timeout` says otherwise.
///
/// Above the plugin's longest fixed bound, 30 s for `screenshot`, so a slow
/// but live app still answers. `wait`, `watch` and a tuned `drag` add the
/// time they spend in the webview on top. Lower and a slow `navigate` fails
/// while the app is still working; higher and a wedged app takes longer to
/// report (#241).
pub(crate) const DEFAULT_RPC_TIMEOUT: Duration = Duration::from_secs(35);

/// Deadline set once from `--rpc-timeout`, read by every `Client` the process opens.
static RPC_TIMEOUT: OnceLock<Duration> = OnceLock::new();

/// Sets the deadline for every `Client` connected afterwards.
///
/// Only the first call takes effect: the CLI sets it once, from its flags,
/// before it opens any connection.
pub(crate) fn set_rpc_timeout(timeout: Duration) {
    let _ = RPC_TIMEOUT.set(timeout);
}

fn rpc_timeout() -> Duration {
    RPC_TIMEOUT.get().copied().unwrap_or(DEFAULT_RPC_TIMEOUT)
}

/// JSON-RPC client over a platform-specific transport (Unix socket or Named Pipe).
pub(crate) struct Client {
    #[cfg(unix)]
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    #[cfg(unix)]
    writer: tokio::net::unix::OwnedWriteHalf,
    #[cfg(windows)]
    reader: BufReader<tokio::io::ReadHalf<tokio::net::windows::named_pipe::NamedPipeClient>>,
    #[cfg(windows)]
    writer: tokio::io::WriteHalf<tokio::net::windows::named_pipe::NamedPipeClient>,
    next_id: u64,
    /// Socket or pipe path, named when the app does not answer.
    endpoint: PathBuf,
    /// How long a call waits for its answer, before `rpc_budget` extends it.
    rpc_timeout: Duration,
    /// Set while a request awaits its answer; still set after a call gave up
    /// or its connection failed.
    in_flight: bool,
}

impl Client {
    /// Connect to the tauri-pilot transport.
    pub async fn connect(path: &Path) -> Result<Self> {
        #[cfg(unix)]
        let (reader, writer) = unix::connect(path).await?;
        #[cfg(windows)]
        let (reader, writer) = windows::connect(path).await?;
        Ok(Self {
            reader: BufReader::new(reader),
            writer,
            next_id: 1,
            endpoint: path.to_owned(),
            rpc_timeout: rpc_timeout(),
            in_flight: false,
        })
    }

    /// Replaces a connection an unfinished or failed call left unusable.
    ///
    /// A no-op while the connection is usable. A scenario calls this before
    /// each step, so a step cut off by its `timeout_ms` or by `--rpc-timeout`,
    /// or one whose connection the app closed, does not fail every step after
    /// it (#241).
    ///
    /// # Errors
    ///
    /// Returns the connection error when the endpoint no longer accepts.
    pub(crate) async fn resync(&mut self) -> Result<()> {
        if self.in_flight {
            let rpc_timeout = self.rpc_timeout;
            *self = Self::connect(&self.endpoint).await?;
            self.rpc_timeout = rpc_timeout;
        }
        Ok(())
    }

    /// Send a JSON-RPC request and return the result value.
    pub async fn call(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        if self.in_flight {
            bail!("Connection is out of sync: an earlier request on it did not complete");
        }
        let id = self.next_id;
        self.next_id += 1;

        let budget = rpc_budget(method, self.rpc_timeout, params.as_ref());
        let bytes = encode(id, method, params)?;
        if bytes.len() > MAX_REQUEST_LEN {
            bail!(
                "{method} request is {} bytes; the plugin accepts at most {MAX_REQUEST_LEN} bytes",
                bytes.len()
            );
        }
        // A call dropped mid-exchange, by this deadline or a caller's, leaves
        // its answer on the way or half read, and the next call would take
        // that as its own. The flag stays set then, and later calls refuse.
        // An I/O error leaves it set too: that connection is dead.
        self.in_flight = true;
        let exchanged = tokio::time::timeout(budget, self.exchange(&bytes))
            .await
            .map_err(|_elapsed| {
                anyhow!(
                    "No response from the app after {budget:?}: {} accepted the connection \
                     but did not answer. Raise --rpc-timeout if the command needs longer.",
                    self.endpoint.display()
                )
            })?;
        let line = exchanged?;
        self.in_flight = false;

        let response: Response = serde_json::from_str(line.trim())?;

        // JSON-RPC 2.0 answers with `"id": null` when it could not read the
        // request id: an oversized line or a parse error. Only one request is
        // in flight, so that error is ours; report it, not a mismatch (#214).
        let unreadable_id = response.id.is_null() && response.error.is_some();
        if !unreadable_id && response.id != serde_json::Value::Number(id.into()) {
            bail!("Response ID mismatch: expected {id}, got {}", response.id);
        }

        // Kept typed, not formatted: the CLI prints it through `Display`, the
        // MCP server reads its fields back out (#242).
        if let Some(err) = response.error {
            return Err(err.into());
        }

        // A missing `result` field (or explicit `"result": null`) means the
        // server-side script completed successfully but produced no value —
        // e.g., `element.click()` or any void expression. Treat this as
        // success with Value::Null rather than an error so bash `&&` chains
        // and `set -e` keep working. See #48.
        Ok(response.result.unwrap_or(serde_json::Value::Null))
    }

    /// Write one request line and read one answer line.
    async fn exchange(&mut self, bytes: &[u8]) -> Result<String> {
        self.writer.write_all(bytes).await?;
        self.writer.flush().await?;

        let mut line = String::new();
        let n = self.reader.read_line(&mut line).await?;
        if n == 0 {
            bail!("Server closed the connection");
        }
        Ok(line)
    }

    /// Bytes the next `call` would write for this request, newline included.
    pub(crate) fn request_len(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<usize> {
        Ok(encode(self.next_id, method, params)?.len())
    }
}

/// Time a call may wait: `rpc_timeout`, plus the time in ms `method` asks
/// the webview to spend before the plugin can answer.
///
/// That is the `timeout` of a `wait` or `watch`, or the gesture of a `drag`:
/// up to 60 moves spaced by `stepDelayMs`, then `settleMs`. The plugin
/// stretches its own bound for the same methods, in `bridge_eval_timeout`
/// and `drag_eval_timeout` of its `handler.rs`; change both together. At the
/// default `rpc_timeout` the client never cuts before the plugin; a lower
/// `--rpc-timeout` is the caller asking to give up sooner.
fn rpc_budget(method: &str, rpc_timeout: Duration, params: Option<&serde_json::Value>) -> Duration {
    let ms = |key: &str| {
        params
            .and_then(|p| p.get(key))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    };
    let in_webview = match method {
        "wait" | "watch" => ms("timeout"),
        // ponytail: counts the 60-move clamp, not `steps`; over-waits only when the app is wedged.
        "drag" => ms("stepDelayMs")
            .saturating_mul(60)
            .saturating_add(ms("settleMs")),
        _ => 0,
    };
    rpc_timeout.saturating_add(Duration::from_millis(in_webview))
}

/// Serialize a request as one line, trailing newline included.
fn encode(id: u64, method: &str, params: Option<serde_json::Value>) -> Result<Vec<u8>> {
    let request = Request {
        jsonrpc: "2.0".to_owned(),
        id,
        method: method.to_owned(),
        params,
    };
    let mut bytes = serde_json::to_vec(&request)?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(unix)]
pub mod unix;
#[cfg(windows)]
pub mod windows;

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// Build a unique socket path per test invocation so parallel `cargo test`
    /// runs (same process, different tests) don't clobber each other's sockets
    /// or leak a previous test's bind into the next one.
    fn unique_socket_path(tag: &str) -> PathBuf {
        let n = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        PathBuf::from(format!(
            "/tmp/tauri-pilot-test-{}-{}-{}.sock",
            tag,
            std::process::id(),
            n
        ))
    }

    /// Accept one connection and answer each request line with `reply(line)`.
    fn mock_server_with(
        path: &Path,
        mut reply: impl FnMut(&str) -> String + Send + 'static,
    ) -> tokio::task::JoinHandle<()> {
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path).expect("bind mock socket");
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            while reader.read_line(&mut line).await.expect("read line") > 0 {
                let mut bytes = reply(line.trim()).into_bytes();
                bytes.push(b'\n');
                writer.write_all(&bytes).await.expect("write bytes");
                writer.flush().await.expect("flush");
                line.clear();
            }
        })
    }

    fn mock_server(path: &Path) -> tokio::task::JoinHandle<()> {
        mock_server_with(path, |line| {
            let req: Request = serde_json::from_str(line).expect("parse request");
            let resp = if req.method == "ping" {
                Response::success(req.id, serde_json::json!({"status": "ok"}))
            } else {
                Response::error(
                    serde_json::Value::Number(req.id.into()),
                    -32601,
                    "Method not found",
                )
            };
            serde_json::to_string(&resp).expect("serialize response")
        })
    }

    /// Connect with retry to avoid race with server bind.
    async fn connect_with_retry(path: &Path) -> Client {
        for _ in 0..20 {
            match Client::connect(path).await {
                Ok(c) => return c,
                Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
        Client::connect(path)
            .await
            .expect("Failed to connect after retries")
    }

    #[tokio::test]
    async fn test_client_ping_returns_ok() {
        let socket = unique_socket_path("t05a");
        let handle = mock_server(&socket);

        let mut client = connect_with_retry(&socket).await;
        let result = client.call("ping", None).await.expect("ping call");
        assert_eq!(result, serde_json::json!({"status": "ok"}));

        handle.abort();
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn test_client_null_result_is_success() {
        // A JSON-RPC response with explicit `"result": null` must be treated as
        // success with Value::Null — not a protocol error. This happens when an
        // eval'd JS expression legitimately returns `undefined` (e.g.,
        // `element.click()`, void functions). Regression test for #48.
        let socket = unique_socket_path("t05c");
        let handle = mock_server_with(&socket, |line| {
            let req: Request = serde_json::from_str(line).expect("parse request");
            // Write `{"result": null}` explicitly to simulate a void JS expr.
            format!(r#"{{"jsonrpc":"2.0","id":{},"result":null}}"#, req.id)
        });

        let mut client = connect_with_retry(&socket).await;
        let result = client.call("eval", None).await.expect("eval call");
        assert_eq!(result, serde_json::Value::Null);

        handle.abort();
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn test_client_missing_result_is_success() {
        // Defensive coverage: a response with neither `result` nor `error` is
        // technically a JSON-RPC protocol edge case. The #48 path proper is
        // covered by `test_client_null_result_is_success` above (explicit
        // `"result": null`); this test pins down the companion shape where
        // the field is omitted entirely. Both end up as `Value::Null` via
        // `unwrap_or`.
        let socket = unique_socket_path("t05d");
        let handle = mock_server_with(&socket, |line| {
            let req: Request = serde_json::from_str(line).expect("parse request");
            // Neither `result` nor `error` present
            format!(r#"{{"jsonrpc":"2.0","id":{}}}"#, req.id)
        });

        let mut client = connect_with_retry(&socket).await;
        let result = client.call("eval", None).await.expect("eval call");
        assert_eq!(result, serde_json::Value::Null);

        handle.abort();
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn test_client_null_id_error_is_reported_as_the_error() {
        // JSON-RPC 2.0 answers with `"id": null` when the request id could not
        // be read, as the plugin does for an oversized line. That error is the
        // failure to report, not an id mismatch. Regression test for #214.
        let socket = unique_socket_path("t05e");
        let handle = mock_server_with(&socket, |_| {
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Request line exceeds maximum length"}}"#.to_owned()
        });

        let mut client = connect_with_retry(&socket).await;
        let err = client.call("eval", None).await.expect_err("server error");
        assert_eq!(
            err.to_string(),
            "RPC error (-32700): Request line exceeds maximum length"
        );

        handle.abort();
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn test_client_null_id_exemption_needs_an_error() {
        // Only an error may come back with `"id": null` (#214): a null-id
        // result, or an error for another id, is still a mismatch.
        let socket = unique_socket_path("t05h");
        let mut replies = [
            r#"{"jsonrpc":"2.0","id":null,"result":42}"#,
            r#"{"jsonrpc":"2.0","id":7,"error":{"code":-32601,"message":"Method not found"}}"#,
        ]
        .into_iter();
        let handle = mock_server_with(&socket, move |_| {
            replies.next().expect("one reply per call").to_owned()
        });

        let mut client = connect_with_retry(&socket).await;
        let err = client.call("eval", None).await.expect_err("null-id result");
        assert_eq!(
            err.to_string(),
            "Response ID mismatch: expected 1, got null"
        );
        let err = client.call("eval", None).await.expect_err("other id");
        assert_eq!(err.to_string(), "Response ID mismatch: expected 2, got 7");

        handle.abort();
        let _ = std::fs::remove_file(&socket);
    }

    /// Bytes the first `call("eval", …)` of a connection sends around the
    /// script, trailing newline included.
    const EVAL_ENVELOPE: usize =
        r#"{"jsonrpc":"2.0","id":1,"method":"eval","params":{"script":""}}"#.len() + 1;

    #[tokio::test]
    async fn test_client_refuses_request_over_plugin_limit_before_sending() {
        // The plugin reads at most 1 MiB per line, newline included. Past that
        // it answers with an error and hangs up, often while the CLI is still
        // writing ("Broken pipe"). Refuse such a request up front (#214).
        let socket = unique_socket_path("t05f");
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).expect("bind mock socket");
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut received = Vec::new();
            stream
                .read_to_end(&mut received)
                .await
                .expect("read to end");
            received.len()
        });

        let mut client = connect_with_retry(&socket).await;
        let script = "a".repeat(1_048_577 - EVAL_ENVELOPE);
        let call = client.call("eval", Some(serde_json::json!({ "script": script })));
        let err = tokio::time::timeout(Duration::from_secs(5), call)
            .await
            .expect("refused without waiting for the plugin")
            .expect_err("oversized request");
        assert_eq!(
            err.to_string(),
            "eval request is 1048577 bytes; the plugin accepts at most 1048576 bytes"
        );

        drop(client);
        let received = handle.await.expect("mock server task");
        assert_eq!(
            received, 0,
            "an oversized request must not reach the plugin"
        );
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn test_client_sends_request_at_exactly_plugin_limit() {
        // The plugin still reads a line of exactly 1 MiB, newline included,
        // so that request must be sent: the mock answers `eval` with -32601.
        let socket = unique_socket_path("t05g");
        let handle = mock_server(&socket);

        let mut client = connect_with_retry(&socket).await;
        let script = "a".repeat(1_048_576 - EVAL_ENVELOPE);
        let err = client
            .call("eval", Some(serde_json::json!({ "script": script })))
            .await
            .expect_err("mock answers eval with Method not found");
        assert!(
            err.to_string().contains("-32601"),
            "a 1048576-byte request must reach the plugin, got: {err}"
        );

        handle.abort();
        let _ = std::fs::remove_file(&socket);
    }

    /// Accept one connection and never answer, like a wedged app or another
    /// process squatting the socket path.
    fn silent_server(path: &Path) -> tokio::task::JoinHandle<()> {
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path).expect("bind mock socket");
        tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.expect("accept");
            std::future::pending::<()>().await;
        })
    }

    #[tokio::test]
    async fn test_client_gives_up_when_the_app_never_answers() {
        // Something that accepts the connection but never writes a line used
        // to block every command forever, with no output (#241).
        let socket = unique_socket_path("t05i");
        let handle = silent_server(&socket);

        let mut client = connect_with_retry(&socket).await;
        client.rpc_timeout = Duration::from_millis(100);
        let err = tokio::time::timeout(Duration::from_secs(5), client.call("ping", None))
            .await
            .expect("bounded by the client deadline")
            .expect_err("silent server");
        assert_eq!(
            err.to_string(),
            format!(
                "No response from the app after 100ms: {} accepted the connection but did not \
                 answer. Raise --rpc-timeout if the command needs longer.",
                socket.display()
            )
        );
        // The unanswered ping may still arrive, so the next call must not
        // read it as its own answer, nor wait a second full deadline.
        let err = tokio::time::timeout(Duration::from_secs(5), client.call("screenshot", None))
            .await
            .expect("refused without waiting for the app")
            .expect_err("connection out of sync");
        assert_eq!(
            err.to_string(),
            "Connection is out of sync: an earlier request on it did not complete"
        );

        handle.abort();
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn test_client_refuses_reuse_after_an_unfinished_call() {
        // A caller's own deadline, like a scenario step `timeout_ms`, drops the
        // call mid-read. The late answer would then be read as the answer to
        // the next request, so that request must fail at once (#241).
        let socket = unique_socket_path("t05j");
        let handle = silent_server(&socket);

        let mut client = connect_with_retry(&socket).await;
        let dropped =
            tokio::time::timeout(Duration::from_millis(50), client.call("click", None)).await;
        assert!(dropped.is_err(), "the silent server cannot answer");
        let err = tokio::time::timeout(Duration::from_secs(5), client.call("screenshot", None))
            .await
            .expect("refused without waiting for the app")
            .expect_err("connection out of sync");
        assert_eq!(
            err.to_string(),
            "Connection is out of sync: an earlier request on it did not complete"
        );

        handle.abort();
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn test_resync_replaces_a_connection_the_app_closed() {
        // An app that restarts mid-run hangs up without answering. The next
        // step must reconnect instead of failing on the dead connection.
        let socket = unique_socket_path("t05l");
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).expect("bind mock socket");
        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let mut line = String::new();
            BufReader::new(stream)
                .read_line(&mut line)
                .await
                .expect("read line");
            let (stream, _) = listener.accept().await.expect("accept again");
            let (reader, mut writer) = stream.into_split();
            line.clear();
            BufReader::new(reader)
                .read_line(&mut line)
                .await
                .expect("read line");
            let req: Request = serde_json::from_str(line.trim()).expect("parse request");
            let resp = Response::success(req.id, serde_json::json!({"status": "ok"}));
            let mut bytes = serde_json::to_vec(&resp).expect("serialize response");
            bytes.push(b'\n');
            writer.write_all(&bytes).await.expect("write response");
        });

        let mut client = Client::connect(&socket).await.expect("connect");
        let err = client
            .call("click", None)
            .await
            .expect_err("the app hung up");
        assert_eq!(err.to_string(), "Server closed the connection");
        client.resync().await.expect("reconnect");
        let result = client.call("ping", None).await.expect("ping");
        assert_eq!(result, serde_json::json!({"status": "ok"}));

        handle.abort();
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn test_client_waits_out_the_wait_timeout() {
        // `call` must pass the extended budget to its timer, not the bare
        // `rpc_timeout`: a lower bound on the elapsed time never flakes.
        let socket = unique_socket_path("t05k");
        let handle = silent_server(&socket);

        let mut client = connect_with_retry(&socket).await;
        client.rpc_timeout = Duration::from_millis(50);
        let started = std::time::Instant::now();
        let params = serde_json::json!({"selector": "#a", "timeout": 200});
        let err = tokio::time::timeout(Duration::from_secs(5), client.call("wait", Some(params)))
            .await
            .expect("bounded by the client deadline")
            .expect_err("silent server");
        assert!(
            started.elapsed() >= Duration::from_millis(250),
            "gave up after {:?}, before the wait timeout ran out: {err}",
            started.elapsed()
        );

        handle.abort();
        let _ = std::fs::remove_file(&socket);
    }

    #[test]
    fn test_rpc_budget_adds_the_wait_timeout() {
        // `wait` and `watch` run for their own `timeout` in the webview, so
        // the client deadline only covers the time around it.
        let base = Duration::from_secs(35);
        let wait = serde_json::json!({"selector": "#a", "timeout": 60_000});
        assert_eq!(
            rpc_budget("wait", base, Some(&wait)),
            Duration::from_secs(95)
        );
        assert_eq!(
            rpc_budget("watch", base, Some(&wait)),
            Duration::from_secs(95)
        );
        assert_eq!(rpc_budget("wait", base, None), base);
    }

    #[test]
    fn test_rpc_budget_adds_a_tuned_drag_gesture() {
        // The plugin waits `steps × stepDelayMs + settleMs` for a drag, so a
        // slow gesture over MCP must not hit the client deadline first.
        let base = Duration::from_secs(35);
        let drag = serde_json::json!({"steps": 60, "stepDelayMs": 1_000, "settleMs": 5_000});
        assert_eq!(
            rpc_budget("drag", base, Some(&drag)),
            Duration::from_secs(100)
        );
    }

    #[test]
    fn test_rpc_budget_ignores_timing_keys_of_other_methods() {
        // Only the methods the plugin stretches its own bound for get more
        // time; a `timeout` key elsewhere must not lengthen the deadline.
        let base = Duration::from_secs(35);
        let params = serde_json::json!({"timeout": 60_000, "stepDelayMs": 1_000});
        assert_eq!(rpc_budget("click", base, Some(&params)), base);
        assert_eq!(rpc_budget("ping", base, None), base);
    }

    #[tokio::test]
    async fn test_client_unknown_method_returns_error() {
        let socket = unique_socket_path("t05b");
        let handle = mock_server(&socket);

        let mut client = connect_with_retry(&socket).await;
        let result = client.call("nonexistent", None).await;
        assert!(result.is_err());
        assert!(
            result
                .expect_err("call returns error")
                .to_string()
                .contains("-32601")
        );

        handle.abort();
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn test_client_connect_failure() {
        let err = Client::connect(Path::new("/tmp/tauri-pilot-nonexistent.sock"))
            .await
            .map(|_| ())
            .expect_err("should fail to connect");
        assert!(err.to_string().contains("Cannot connect"));
    }
}
