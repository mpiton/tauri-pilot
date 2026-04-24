//! Integration tests for `PilotMcpServer` socket pinning and call helpers.

use serde_json::{Map, Value, json};
use serial_test::serial;
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::task::JoinHandle;

use crate::protocol::{Request, Response};
use rmcp::model::CallToolResult;

use super::super::handlers::call_tool_by_name;
use super::PilotMcpServer;

#[tokio::test]
#[serial]
async fn auto_detected_socket_is_pinned_after_first_connection() {
    let dir = std::env::temp_dir().join(format!("tauri-pilot-mcp-pin-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create socket dir");
    let old_socket = dir.join("tauri-pilot-old.sock");
    let new_socket = dir.join("tauri-pilot-new.sock");
    let _ = std::fs::remove_file(&old_socket);
    let _ = std::fs::remove_file(&new_socket);

    let old_server = spawn_click_server(&old_socket, "old", 2);

    // SAFETY: serial attribute serializes tests that touch XDG_RUNTIME_DIR.
    unsafe { std::env::set_var("XDG_RUNTIME_DIR", &dir) };

    let pilot = PilotMcpServer::new(None, None);
    let first = call_click(&pilot).await;
    assert_eq!(tool_result_source(&first), Some("old"));

    let new_server = spawn_click_server(&new_socket, "new", 1);
    let second = call_click(&pilot).await;
    assert_eq!(tool_result_source(&second), Some("old"));

    unsafe { std::env::remove_var("XDG_RUNTIME_DIR") };
    old_server.await.expect("old mock server task");
    new_server.abort();
    let _ = std::fs::remove_file(&old_socket);
    let _ = std::fs::remove_file(&new_socket);
    let _ = std::fs::remove_dir(&dir);
}

async fn call_click(pilot: &PilotMcpServer) -> CallToolResult {
    let mut args = Map::new();
    args.insert("target".to_owned(), json!("@e3"));
    call_tool_by_name(pilot, "click", args)
        .await
        .expect("tool call succeeds")
}

fn tool_result_source(result: &CallToolResult) -> Option<&str> {
    result
        .structured_content
        .as_ref()
        .and_then(|content| content.get("result"))
        .and_then(|result| result.get("source"))
        .and_then(Value::as_str)
}

fn spawn_click_server(socket: &Path, source: &'static str, requests: usize) -> JoinHandle<()> {
    let listener = UnixListener::bind(socket).expect("bind mock socket");
    tokio::spawn(async move {
        for _ in 0..requests {
            let (stream, _) = listener.accept().await.expect("accept");
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            reader.read_line(&mut line).await.expect("read request");
            let request: Request = serde_json::from_str(line.trim()).expect("parse request");
            assert_eq!(request.method, "click");
            let mut response =
                serde_json::to_vec(&Response::success(request.id, json!({"source": source})))
                    .expect("serialize response");
            response.push(b'\n');
            writer.write_all(&response).await.expect("write response");
        }
    })
}
