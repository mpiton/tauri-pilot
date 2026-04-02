use crate::error::Error;
use crate::handler;
use crate::protocol::{Request, Response};

use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

/// Start the socket server. Logs errors internally — never returns `Err` to callers.
pub(crate) async fn start(socket_path: PathBuf) {
    if let Err(e) = run(&socket_path).await {
        tracing::error!(path = %socket_path.display(), "socket server error: {e}");
    }
}

async fn run(socket_path: &PathBuf) -> Result<(), Error> {
    let listener = UnixListener::bind(socket_path)?;
    tracing::info!(path = %socket_path.display(), "tauri-pilot socket listening");

    loop {
        let (stream, _addr) = listener.accept().await?;
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream).await {
                tracing::warn!("connection error: {e}");
            }
        });
    }
}

async fn handle_connection(stream: UnixStream) -> Result<(), Error> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Request>(trimmed) {
            Ok(req) => dispatch(&req),
            Err(e) => Response::error(0, -32700, format!("Parse error: {e}")),
        };

        let mut resp_bytes = serde_json::to_vec(&response)?;
        resp_bytes.push(b'\n');
        writer.write_all(&resp_bytes).await?;
        writer.flush().await?;
    }

    Ok(())
}

fn dispatch(req: &Request) -> Response {
    match handler::dispatch(&req.method, req.params.as_ref()) {
        Ok(result) => Response::success(req.id, result),
        Err(rpc_err) => Response {
            jsonrpc: "2.0".to_owned(),
            id: req.id,
            result: None,
            error: Some(rpc_err),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    async fn start_test_server(path: &PathBuf) -> tokio::task::JoinHandle<()> {
        let _ = std::fs::remove_file(path);
        let p = path.clone();
        let handle = tokio::spawn(async move { start(p).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        handle
    }

    #[tokio::test]
    async fn test_server_responds_ping_ok() {
        let socket = PathBuf::from("/tmp/tauri-pilot-test-t04a.sock");
        let handle = start_test_server(&socket).await;

        let stream = UnixStream::connect(&socket).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        writer
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n")
            .await
            .unwrap();
        writer.flush().await.unwrap();

        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let resp: Response = serde_json::from_str(&line).unwrap();

        assert_eq!(resp.id, 1);
        assert!(resp.error.is_none());
        assert_eq!(resp.result, Some(serde_json::json!({"status": "ok"})));

        handle.abort();
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn test_server_handles_invalid_json() {
        let socket = PathBuf::from("/tmp/tauri-pilot-test-t03b.sock");
        let handle = start_test_server(&socket).await;

        let stream = UnixStream::connect(&socket).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        writer.write_all(b"not json\n").await.unwrap();
        writer.flush().await.unwrap();

        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let resp: Response = serde_json::from_str(&line).unwrap();

        assert_eq!(resp.id, 0);
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32700);

        handle.abort();
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn test_server_handles_multiple_requests() {
        let socket = PathBuf::from("/tmp/tauri-pilot-test-t03c.sock");
        let handle = start_test_server(&socket).await;

        let stream = UnixStream::connect(&socket).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        for i in 1..=3 {
            let req = format!("{{\"jsonrpc\":\"2.0\",\"id\":{i},\"method\":\"test\"}}\n");
            writer.write_all(req.as_bytes()).await.unwrap();
            writer.flush().await.unwrap();

            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let resp: Response = serde_json::from_str(&line).unwrap();
            assert_eq!(resp.id, i);
        }

        handle.abort();
        let _ = std::fs::remove_file(&socket);
    }
}
