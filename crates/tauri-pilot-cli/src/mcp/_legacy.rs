//! TEMPORARY — shrinks step-by-step toward zero (PR1 Tasks 5-8).
//!
//! Removed in Task 5: `PilotMcpServer` struct, `run_mcp_server`, impl blocks.
//! Removed in Task 6: `tools()` registry, schema builders, `ToolSpec` struct.
//! Remaining for Task 8: delete this file.

#[cfg(test)]
mod tests {
    use super::super::banner::startup_banner;
    use super::super::server::PilotMcpServer;
    #[cfg(unix)]
    use crate::protocol::{Request, Response};
    use rmcp::model::CallToolResult;
    use serde_json::{Map, Value, json};
    #[cfg(unix)]
    use serial_test::serial;
    #[cfg(unix)]
    use std::path::Path;
    #[cfg(unix)]
    use tokio::net::UnixListener;
    #[cfg(unix)]
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        task::JoinHandle,
    };

    #[test]
    fn startup_banner_explains_stdio_server() {
        let banner = startup_banner(None, Some("main"));

        assert!(banner.contains("tauri-pilot MCP server"));
        assert!(banner.contains("listening on stdio"));
        assert!(banner.contains("auto-detect on first tool call"));
        assert!(banner.contains("main"));
        assert!(banner.contains("stdout is reserved for MCP JSON-RPC"));
    }

    #[tokio::test]
    async fn replay_export_does_not_connect_to_socket() {
        let recording = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-replay-test-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &recording,
            r#"[{"action":"click","timestamp":0,"ref":"e1"}]"#,
        )
        .expect("write recording");

        let missing_socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-missing-{}.sock",
            std::process::id()
        ));
        let pilot = PilotMcpServer::new(Some(missing_socket), None);
        let mut args = Map::new();
        args.insert("path".to_owned(), json!(recording.display().to_string()));
        args.insert("export".to_owned(), json!("sh"));

        let result = super::super::handlers::call_tool_by_name(&pilot, "replay", args)
            .await
            .expect("tool call succeeds");

        assert_eq!(result.is_error, Some(false));
        let script = result
            .structured_content
            .as_ref()
            .and_then(|content| content.get("result"))
            .and_then(Value::as_str)
            .expect("script result");
        assert!(script.starts_with("#!/bin/bash"));
        assert!(script.contains("tauri-pilot click @e1"));

        let _ = std::fs::remove_file(&recording);
    }

    #[tokio::test]
    #[serial]
    #[cfg(unix)]
    async fn auto_detected_socket_is_pinned_after_first_connection() {
        let dir =
            std::env::temp_dir().join(format!("tauri-pilot-mcp-pin-test-{}", std::process::id()));
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

    #[tokio::test]
    #[cfg(unix)]
    async fn click_tool_sends_json_rpc_request() {
        let socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-click-test-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).expect("bind mock socket");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            reader.read_line(&mut line).await.expect("read request");
            let request: Request = serde_json::from_str(line.trim()).expect("parse request");
            assert_eq!(request.method, "click");
            assert_eq!(request.params, Some(json!({"ref": "e3"})));
            let mut response =
                serde_json::to_vec(&Response::success(request.id, json!({"ok": true})))
                    .expect("serialize response");
            response.push(b'\n');
            writer.write_all(&response).await.expect("write response");
        });

        let pilot = PilotMcpServer::new(Some(socket.clone()), None);
        let mut args = Map::new();
        args.insert("target".to_owned(), json!("@e3"));
        let result = super::super::handlers::call_tool_by_name(&pilot, "click", args)
            .await
            .expect("tool call succeeds");
        assert_eq!(result.is_error, Some(false));
        assert_eq!(
            result.structured_content,
            Some(json!({"result": {"ok": true}}))
        );

        server.await.expect("mock server task");
        let _ = std::fs::remove_file(&socket);
    }

    #[cfg(unix)]
    async fn call_click(pilot: &PilotMcpServer) -> CallToolResult {
        let mut args = Map::new();
        args.insert("target".to_owned(), json!("@e3"));
        super::super::handlers::call_tool_by_name(pilot, "click", args)
            .await
            .expect("tool call succeeds")
    }

    #[cfg(unix)]
    fn tool_result_source(result: &CallToolResult) -> Option<&str> {
        result
            .structured_content
            .as_ref()
            .and_then(|content| content.get("result"))
            .and_then(|result| result.get("source"))
            .and_then(Value::as_str)
    }

    #[cfg(unix)]
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
}
