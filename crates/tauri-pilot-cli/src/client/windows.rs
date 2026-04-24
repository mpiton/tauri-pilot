use super::Client;
use anyhow::{Context, Result};
use std::path::Path;
use std::time::Duration;
use tokio::io::BufReader;
use tokio::net::windows::named_pipe::ClientOptions;
use windows::Win32::Foundation::ERROR_PIPE_BUSY;

/// Maximum time spent retrying when all server pipe instances are busy.
/// Matches typical Unix socket `connect()` latency under contention — longer
/// and the CLI feels hung; shorter and transient bursts falsely fail.
const CONNECT_DEADLINE: Duration = Duration::from_secs(5);

/// Delay between `ERROR_PIPE_BUSY` retries. Keeps CPU idle while the server
/// accepts pending connections.
const RETRY_INTERVAL: Duration = Duration::from_millis(20);

/// Connect to a Named Pipe, retrying briefly on `ERROR_PIPE_BUSY`.
///
/// Named Pipe `open()` is synchronous and fails immediately with
/// `ERROR_PIPE_BUSY` when all server instances are in use. The caller expects
/// an async contract, so we mirror the Unix socket behavior by yielding
/// between retries, giving up after `CONNECT_DEADLINE` so a stuck server
/// never hangs the CLI.
pub async fn connect(path: &Path) -> Result<Client> {
    let deadline = tokio::time::Instant::now() + CONNECT_DEADLINE;
    let client = loop {
        match ClientOptions::new().open(path) {
            Ok(c) => break c,
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY.0.cast_signed()) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(e).with_context(|| {
                        format!(
                            "Named pipe {} remained busy for {}s",
                            path.display(),
                            CONNECT_DEADLINE.as_secs()
                        )
                    });
                }
                tokio::time::sleep(RETRY_INTERVAL).await;
            }
            Err(e) => {
                return Err(e).with_context(|| {
                    format!("Cannot connect to named pipe: {}", path.display())
                });
            }
        }
    };
    let (reader, writer) = tokio::io::split(client);
    Ok(Client {
        reader: BufReader::new(reader),
        writer,
        next_id: 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Request, Response};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::windows::named_pipe::ServerOptions;

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn unique_pipe_path() -> String {
        let n = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        format!(r"\\.\pipe\tauri-pilot-test-{}-{}", std::process::id(), n)
    }

    fn mock_server(path: &str) -> tokio::task::JoinHandle<()> {
        let server = ServerOptions::new()
            .create(path)
            .expect("create named pipe server");
        tokio::spawn(async move {
            server.connect().await.expect("server accept");
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            while reader.read_line(&mut line).await.expect("read line") > 0 {
                let req: Request = serde_json::from_str(line.trim()).expect("parse request");
                let resp = if req.method == "ping" {
                    Response::success(req.id, serde_json::json!({"status": "ok"}))
                } else {
                    Response::error(
                        serde_json::Value::Number(req.id.into()),
                        -32601,
                        "Method not found",
                    )
                };
                let mut bytes = serde_json::to_vec(&resp).expect("serialize response");
                bytes.push(b'\n');
                writer.write_all(&bytes).await.expect("write bytes");
                writer.flush().await.expect("flush");
                line.clear();
            }
        })
    }

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
        let pipe = unique_pipe_path();
        let handle = mock_server(&pipe);

        let mut client = connect_with_retry(Path::new(&pipe)).await;
        let result = client.call("ping", None).await.expect("ping call");
        assert_eq!(result, serde_json::json!({"status": "ok"}));

        handle.abort();
    }

    #[tokio::test]
    async fn test_client_null_result_is_success() {
        let pipe = unique_pipe_path();
        let server = ServerOptions::new()
            .create(&pipe)
            .expect("create named pipe server");
        let handle = tokio::spawn(async move {
            server.connect().await.expect("server accept");
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            reader.read_line(&mut line).await.expect("read line");
            let req: Request = serde_json::from_str(line.trim()).expect("parse request");
            let raw = format!(r#"{{"jsonrpc":"2.0","id":{},"result":null}}"#, req.id);
            writer.write_all(raw.as_bytes()).await.expect("write raw");
            writer.write_all(b"\n").await.expect("write newline");
            writer.flush().await.expect("flush");
        });

        let mut client = connect_with_retry(Path::new(&pipe)).await;
        let result = client.call("eval", None).await.expect("eval call");
        assert_eq!(result, serde_json::Value::Null);

        handle.abort();
    }

    #[tokio::test]
    async fn test_client_missing_result_is_success() {
        let pipe = unique_pipe_path();
        let server = ServerOptions::new()
            .create(&pipe)
            .expect("create named pipe server");
        let handle = tokio::spawn(async move {
            server.connect().await.expect("server accept");
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            reader.read_line(&mut line).await.expect("read line");
            let req: Request = serde_json::from_str(line.trim()).expect("parse request");
            let raw = format!(r#"{{"jsonrpc":"2.0","id":{}}}"#, req.id);
            writer.write_all(raw.as_bytes()).await.expect("write raw");
            writer.write_all(b"\n").await.expect("write newline");
            writer.flush().await.expect("flush");
        });

        let mut client = connect_with_retry(Path::new(&pipe)).await;
        let result = client.call("eval", None).await.expect("eval call");
        assert_eq!(result, serde_json::Value::Null);

        handle.abort();
    }

    #[tokio::test]
    async fn test_client_unknown_method_returns_error() {
        let pipe = unique_pipe_path();
        let handle = mock_server(&pipe);

        let mut client = connect_with_retry(Path::new(&pipe)).await;
        let result = client.call("nonexistent", None).await;
        assert!(result.is_err());
        assert!(result.expect_err("call returns error").to_string().contains("-32601"));

        handle.abort();
    }

    #[tokio::test]
    async fn test_client_connect_failure() {
        let err = Client::connect(Path::new(r"\\.\pipe\tauri-pilot-nonexistent"))
            .await
            .map(|_| ())
            .expect_err("should fail to connect");
        assert!(err.to_string().contains("Cannot connect"));
    }
}
