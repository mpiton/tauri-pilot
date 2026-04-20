use super::Client;
use anyhow::{Context, Result};
use std::path::Path;
use tokio::io::BufReader;
use tokio::net::windows::named_pipe::ClientOptions;

pub async fn connect(path: &Path) -> Result<Client> {
    let client = ClientOptions::new()
        .open(path)
        .with_context(|| format!("Cannot connect to named pipe: {}", path.display()))?;
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
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::windows::named_pipe::ServerOptions;

    async fn mock_server(path: &str) -> tokio::task::JoinHandle<()> {
        let server = ServerOptions::new()
            .create(path)
            .expect("create named pipe server");
        tokio::spawn(async move {
            server.connect().await.expect("server accept");
            let (reader, mut writer) = tokio::io::split(server);
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
        let pipe = r"\\.\pipe\tauri-pilot-test-t05a";
        let handle = mock_server(pipe).await;

        let mut client = connect_with_retry(Path::new(pipe)).await;
        let result = client.call("ping", None).await.unwrap();
        assert_eq!(result, serde_json::json!({"status": "ok"}));

        handle.abort();
    }

    #[tokio::test]
    async fn test_client_null_result_is_success() {
        let pipe = r"\\.\pipe\tauri-pilot-test-t05c";
        let server = ServerOptions::new()
            .create(pipe)
            .expect("create named pipe server");
        let handle = tokio::spawn(async move {
            server.connect().await.expect("server accept");
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let req: Request = serde_json::from_str(line.trim()).unwrap();
            let raw = format!(r#"{{"jsonrpc":"2.0","id":{},"result":null}}"#, req.id);
            writer.write_all(raw.as_bytes()).await.unwrap();
            writer.write_all(b"\n").await.unwrap();
            writer.flush().await.unwrap();
        });

        let mut client = connect_with_retry(Path::new(pipe)).await;
        let result = client.call("eval", None).await.unwrap();
        assert_eq!(result, serde_json::Value::Null);

        handle.abort();
    }

    #[tokio::test]
    async fn test_client_missing_result_is_success() {
        let pipe = r"\\.\pipe\tauri-pilot-test-t05d";
        let server = ServerOptions::new()
            .create(pipe)
            .expect("create named pipe server");
        let handle = tokio::spawn(async move {
            server.connect().await.expect("server accept");
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let req: Request = serde_json::from_str(line.trim()).unwrap();
            let raw = format!(r#"{{"jsonrpc":"2.0","id":{}}}"#, req.id);
            writer.write_all(raw.as_bytes()).await.unwrap();
            writer.write_all(b"\n").await.unwrap();
            writer.flush().await.unwrap();
        });

        let mut client = connect_with_retry(Path::new(pipe)).await;
        let result = client.call("eval", None).await.unwrap();
        assert_eq!(result, serde_json::Value::Null);

        handle.abort();
    }

    #[tokio::test]
    async fn test_client_unknown_method_returns_error() {
        let pipe = r"\\.\pipe\tauri-pilot-test-t05b";
        let handle = mock_server(pipe).await;

        let mut client = connect_with_retry(Path::new(pipe)).await;
        let result = client.call("nonexistent", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("-32601"));

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
