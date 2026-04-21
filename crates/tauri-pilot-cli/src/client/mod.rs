use crate::protocol::{Request, Response};

use anyhow::{Result, bail};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

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
}

impl Client {
    /// Connect to the tauri-pilot transport.
    pub async fn connect(path: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            unix::connect(path).await
        }
        #[cfg(windows)]
        {
            windows::connect(path).await
        }
    }

    /// Send a JSON-RPC request and return the result value.
    pub async fn call(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let id = self.next_id;
        self.next_id += 1;

        let request = Request {
            jsonrpc: "2.0".to_owned(),
            id,
            method: method.to_owned(),
            params,
        };

        let mut bytes = serde_json::to_vec(&request)?;
        bytes.push(b'\n');
        self.writer.write_all(&bytes).await?;
        self.writer.flush().await?;

        let mut line = String::new();
        let n = self.reader.read_line(&mut line).await?;
        if n == 0 {
            bail!("Server closed the connection");
        }

        let response: Response = serde_json::from_str(line.trim())?;

        if response.id != id {
            bail!("Response ID mismatch: expected {id}, got {}", response.id);
        }

        if let Some(err) = response.error {
            bail!("RPC error ({}): {}", err.code, err.message);
        }

        // A missing `result` field (or explicit `"result": null`) means the
        // server-side script completed successfully but produced no value —
        // e.g., `element.click()` or any void expression. Treat this as
        // success with Value::Null rather than an error so bash `&&` chains
        // and `set -e` keep working. See #48.
        Ok(response.result.unwrap_or(serde_json::Value::Null))
    }
}

#[cfg(unix)]
pub mod unix;
#[cfg(windows)]
pub mod windows;

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    async fn mock_server(path: &PathBuf) -> tokio::task::JoinHandle<()> {
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path).unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            while reader.read_line(&mut line).await.unwrap() > 0 {
                let req: Request = serde_json::from_str(line.trim()).unwrap();
                let resp = if req.method == "ping" {
                    Response::success(req.id, serde_json::json!({"status": "ok"}))
                } else {
                    Response::error(req.id, -32601, "Method not found")
                };
                let mut bytes = serde_json::to_vec(&resp).unwrap();
                bytes.push(b'\n');
                writer.write_all(&bytes).await.unwrap();
                writer.flush().await.unwrap();
                line.clear();
            }
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
        let socket = PathBuf::from("/tmp/tauri-pilot-test-t05a.sock");
        let handle = mock_server(&socket).await;

        let mut client = connect_with_retry(&socket).await;
        let result = client.call("ping", None).await.unwrap();
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
        let socket = PathBuf::from("/tmp/tauri-pilot-test-t05c.sock");
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).unwrap();
        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let req: Request = serde_json::from_str(line.trim()).unwrap();
            // Write `{"result": null}` explicitly to simulate a void JS expr.
            let raw = format!(r#"{{"jsonrpc":"2.0","id":{},"result":null}}"#, req.id);
            writer.write_all(raw.as_bytes()).await.unwrap();
            writer.write_all(b"\n").await.unwrap();
            writer.flush().await.unwrap();
        });

        let mut client = connect_with_retry(&socket).await;
        let result = client.call("eval", None).await.unwrap();
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
        let socket = PathBuf::from("/tmp/tauri-pilot-test-t05d.sock");
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).unwrap();
        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let req: Request = serde_json::from_str(line.trim()).unwrap();
            // Neither `result` nor `error` present
            let raw = format!(r#"{{"jsonrpc":"2.0","id":{}}}"#, req.id);
            writer.write_all(raw.as_bytes()).await.unwrap();
            writer.write_all(b"\n").await.unwrap();
            writer.flush().await.unwrap();
        });

        let mut client = connect_with_retry(&socket).await;
        let result = client.call("eval", None).await.unwrap();
        assert_eq!(result, serde_json::Value::Null);

        handle.abort();
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn test_client_unknown_method_returns_error() {
        let socket = PathBuf::from("/tmp/tauri-pilot-test-t05b.sock");
        let handle = mock_server(&socket).await;

        let mut client = connect_with_retry(&socket).await;
        let result = client.call("nonexistent", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("-32601"));

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
