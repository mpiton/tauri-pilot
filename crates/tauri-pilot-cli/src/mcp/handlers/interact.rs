//! Handlers for the `interact` tool group: click, fill, type, press, select,
//! check, scroll, drag, drop. Tests in `interact_tests.rs`.

use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject},
};
use serde_json::json;

use super::super::args::{optional_i32, optional_ref, optional_string, required_string};
use super::super::responses::invalid_params;
use super::super::server::PilotMcpServer;
use crate::target_params;

pub(super) async fn dispatch(
    server: &PilotMcpServer,
    name: &str,
    args: &JsonObject,
    window: Option<String>,
) -> Result<CallToolResult, McpError> {
    match name {
        "click" => server.target_call("click", args, window).await,
        "fill" => {
            let mut params = target_params(&required_string(args, "target")?);
            params["value"] = json!(required_string(args, "value")?);
            server.call_app_tool("fill", Some(params), window).await
        }
        "type" => {
            let mut params = target_params(&required_string(args, "target")?);
            params["text"] = json!(required_string(args, "text")?);
            server.call_app_tool("type", Some(params), window).await
        }
        "press" => {
            server
                .call_app_tool(
                    "press",
                    Some(json!({"key": required_string(args, "key")?})),
                    window,
                )
                .await
        }
        "select" => {
            let mut params = target_params(&required_string(args, "target")?);
            params["value"] = json!(required_string(args, "value")?);
            server.call_app_tool("select", Some(params), window).await
        }
        "check" => server.target_call("check", args, window).await,
        "scroll" => {
            server
                .call_app_tool(
                    "scroll",
                    Some(json!({
                        "direction": optional_string(args, "direction")?.unwrap_or_else(|| "down".to_owned()),
                        "amount": optional_i32(args, "amount")?,
                        "ref": optional_ref(args)?,
                    })),
                    window,
                )
                .await
        }
        "drag" => {
            let source = required_string(args, "source")?;
            let mut params = json!({"source": target_params(&source)});
            let target = optional_string(args, "target")?;
            let offset = args.get("offset").cloned();
            match (target, offset) {
                (Some(_), Some(_)) => {
                    return Err(invalid_params(
                        "drag accepts either 'target' or 'offset', not both",
                    ));
                }
                (None, None) => {
                    return Err(invalid_params("drag requires either 'target' or 'offset'"));
                }
                (Some(target), None) => {
                    params["target"] = target_params(&target);
                }
                (None, Some(offset)) => {
                    params["offset"] = offset;
                }
            }
            server.call_app_tool("drag", Some(params), window).await
        }
        "drop" => server.call_drop_tool(args.clone(), window).await,
        _ => unreachable!("handlers/mod.rs guarantees prefix match: {name}"),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use serde_json::{Map, json};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    use super::super::super::server::PilotMcpServer;
    use super::super::call_tool_by_name;
    use crate::protocol::{Request, Response};

    #[tokio::test]
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
        let result = call_tool_by_name(&pilot, "click", args)
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
}
