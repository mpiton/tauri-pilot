use crate::protocol::{Request, Response};

use anyhow::{Context, Result, bail};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

/// JSON-RPC client over a Unix socket.
pub(crate) struct Client {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    next_id: u64,
}

impl Client {
    /// Connect to the tauri-pilot socket.
    pub async fn connect(path: &Path) -> Result<Self> {
        let stream = UnixStream::connect(path)
            .await
            .with_context(|| format!("Cannot connect to socket: {}", path.display()))?;

        let (reader, writer) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(reader),
            writer,
            next_id: 1,
        })
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

        response
            .result
            .context("Server returned empty result without error")
    }
}

#[cfg(test)]
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
