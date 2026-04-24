use std::{
    io::IsTerminal,
    path::Path,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use anyhow::Result;
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResult, ErrorCode, Implementation, JsonObject,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
        ToolAnnotations,
    },
    service::{MaybeSendFuture, RequestContext, RoleServer},
    transport::stdio,
};
use serde_json::{Map, Value, json};

use crate::{
    client::Client, export_replay_file, resolve_socket, run_drop_command, run_replay_command,
    target_params, with_window,
};

#[derive(Debug, Clone)]
pub(crate) struct PilotMcpServer {
    socket: Option<PathBuf>,
    window: Option<String>,
    resolved_socket: Arc<OnceLock<PathBuf>>,
}

pub(crate) async fn run_mcp_server(socket: Option<PathBuf>, window: Option<String>) -> Result<()> {
    print_startup_banner(socket.as_deref(), window.as_deref());
    let service = PilotMcpServer::new(socket, window)
        .serve(stdio())
        .await
        .map_err(|e| anyhow::anyhow!("failed to initialize MCP server: {e}"))?;
    service
        .waiting()
        .await
        .map_err(|e| anyhow::anyhow!("MCP server failed: {e}"))?;
    Ok(())
}

fn print_startup_banner(socket: Option<&Path>, window: Option<&str>) {
    if std::io::stdin().is_terminal() && std::io::stderr().is_terminal() {
        eprintln!("{}", startup_banner(socket, window));
    }
}

fn startup_banner(socket: Option<&Path>, window: Option<&str>) -> String {
    let socket = socket.map_or_else(
        || "auto-detect on first tool call".to_owned(),
        |path| path.display().to_string(),
    );
    let window = window.unwrap_or("default app window");

    format!(
        r"
tauri-pilot MCP server

Status : listening on stdio
Socket : {socket}
Window : {window}

stdout is reserved for MCP JSON-RPC.
Configure your MCP client to launch this command instead of typing requests here.
"
    )
}

impl PilotMcpServer {
    fn new(socket: Option<PathBuf>, window: Option<String>) -> Self {
        Self {
            socket,
            window,
            resolved_socket: Arc::new(OnceLock::new()),
        }
    }

    async fn connect_client(&self) -> Result<Client> {
        if let Some(socket) = &self.socket {
            return Client::connect(socket).await;
        }

        if let Some(socket) = self.resolved_socket.get() {
            return Client::connect(socket).await;
        }

        let socket = resolve_socket(None)?;
        let client = Client::connect(&socket).await?;
        let _ = self.resolved_socket.set(socket);
        Ok(client)
    }

    async fn call_app(
        &self,
        method: &'static str,
        params: Option<Value>,
        window: Option<String>,
    ) -> Result<Value> {
        let mut client = self.connect_client().await?;
        client
            .call(method, with_window(params, window.as_deref()))
            .await
    }

    async fn call_app_tool(
        &self,
        method: &'static str,
        params: Option<Value>,
        window: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        Ok(match self.call_app(method, params, window).await {
            Ok(result) => tool_success(result),
            Err(err) => tool_error(err),
        })
    }

    #[allow(clippy::too_many_lines)]
    async fn call_tool_by_name(
        &self,
        name: &str,
        args: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let window = self.window_arg(&args)?;
        match name {
            "ping" => self.call_app_tool("ping", None, window).await,
            "windows" => self.call_app_tool("windows.list", None, None).await,
            "state" => self.call_app_tool("state", None, window).await,
            "snapshot" => self
                .call_app_tool(
                    "snapshot",
                    Some(json!({
                        "interactive": optional_bool(&args, "interactive")?.unwrap_or(false),
                        "selector": optional_string(&args, "selector")?,
                        "depth": optional_u8(&args, "depth")?,
                    })),
                    window,
                )
                .await,
            "diff" => {
                let mut params = json!({
                    "interactive": optional_bool(&args, "interactive")?.unwrap_or(false),
                    "selector": optional_string(&args, "selector")?,
                    "depth": optional_u8(&args, "depth")?,
                });
                if let Some(reference) = args.get("reference") {
                    params["reference"] = reference.clone();
                }
                self.call_app_tool("diff", Some(params), window).await
            }
            "click" => self.target_call("click", &args, window).await,
            "fill" => {
                let mut params = target_params(&required_string(&args, "target")?);
                params["value"] = json!(required_string(&args, "value")?);
                self.call_app_tool("fill", Some(params), window).await
            }
            "type" => {
                let mut params = target_params(&required_string(&args, "target")?);
                params["text"] = json!(required_string(&args, "text")?);
                self.call_app_tool("type", Some(params), window).await
            }
            "press" => {
                self.call_app_tool(
                    "press",
                    Some(json!({"key": required_string(&args, "key")?})),
                    window,
                )
                .await
            }
            "select" => {
                let mut params = target_params(&required_string(&args, "target")?);
                params["value"] = json!(required_string(&args, "value")?);
                self.call_app_tool("select", Some(params), window).await
            }
            "check" => self.target_call("check", &args, window).await,
            "scroll" => {
                self.call_app_tool(
                    "scroll",
                    Some(json!({
                        "direction": optional_string(&args, "direction")?.unwrap_or_else(|| "down".to_owned()),
                        "amount": optional_i32(&args, "amount")?,
                        "ref": optional_ref(&args)?,
                    })),
                    window,
                )
                .await
            }
            "drag" => {
                let source = required_string(&args, "source")?;
                let mut params = json!({"source": target_params(&source)});
                let target = optional_string(&args, "target")?;
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
                self.call_app_tool("drag", Some(params), window).await
            }
            "drop" => self.call_drop_tool(args, window).await,
            "text" => self.target_call("text", &args, window).await,
            "html" => {
                let params =
                    optional_string(&args, "target")?.map(|target| target_params(&target));
                self.call_app_tool("html", params, window).await
            }
            "value" => self.target_call("value", &args, window).await,
            "attrs" => self.target_call("attrs", &args, window).await,
            "eval" => {
                self.call_app_tool(
                    "eval",
                    Some(json!({"script": required_string(&args, "script")?})),
                    window,
                )
                .await
            }
            "ipc" => {
                self.call_app_tool(
                    "ipc",
                    Some(json!({
                        "command": required_string(&args, "command")?,
                        "args": args.get("args").cloned(),
                    })),
                    window,
                )
                .await
            }
            "screenshot" => {
                self.call_app_tool(
                    "screenshot",
                    Some(json!({"selector": optional_string(&args, "selector")?})),
                    window,
                )
                .await
            }
            "navigate" => {
                self.call_app_tool(
                    "navigate",
                    Some(json!({"url": required_string(&args, "url")?})),
                    window,
                )
                .await
            }
            "url" => self.call_app_tool("url", None, window).await,
            "title" => self.call_app_tool("title", None, window).await,
            "wait" => {
                self.call_app_tool(
                    "wait",
                    Some(json!({
                        "target": optional_string(&args, "target")?,
                        "selector": optional_string(&args, "selector")?,
                        "gone": optional_bool(&args, "gone")?.unwrap_or(false),
                        "timeout": optional_u64(&args, "timeout")?.unwrap_or(10_000),
                    })),
                    window,
                )
                .await
            }
            "watch" => {
                let mut watch_params = json!({
                    "selector": optional_string(&args, "selector")?,
                    "timeout": optional_u64(&args, "timeout")?.unwrap_or(10_000),
                    "stable": optional_u64(&args, "stable")?.unwrap_or(300),
                });
                if optional_bool(&args, "require_mutation")?.unwrap_or(false) {
                    watch_params["requireMutation"] = json!(true);
                }
                self.call_app_tool("watch", Some(watch_params), window).await
            }
            "logs" => self.call_logs_tool(&args, window).await,
            "network" => self.call_network_tool(&args, window).await,
            "storage_get" => {
                self.call_app_tool(
                    "storage.get",
                    Some(json!({
                        "key": required_string(&args, "key")?,
                        "session": optional_bool(&args, "session")?.unwrap_or(false),
                    })),
                    window,
                )
                .await
            }
            "storage_set" => {
                self.call_app_tool(
                    "storage.set",
                    Some(json!({
                        "key": required_string(&args, "key")?,
                        "value": required_string(&args, "value")?,
                        "session": optional_bool(&args, "session")?.unwrap_or(false),
                    })),
                    window,
                )
                .await
            }
            "storage_list" => {
                self.call_app_tool(
                    "storage.list",
                    Some(json!({"session": optional_bool(&args, "session")?.unwrap_or(false)})),
                    window,
                )
                .await
            }
            "storage_clear" => {
                self.call_app_tool(
                    "storage.clear",
                    Some(json!({"session": optional_bool(&args, "session")?.unwrap_or(false)})),
                    window,
                )
                .await
            }
            "forms" => {
                let params =
                    optional_string(&args, "selector")?.map(|selector| json!({ "selector": selector }));
                self.call_app_tool("forms.dump", params, window).await
            }
            "assert_text" => self.assert_text(args, window, false).await,
            "assert_contains" => self.assert_text(args, window, true).await,
            "assert_visible" => self.assert_bool("visible", args, window, true).await,
            "assert_hidden" => self.assert_bool("visible", args, window, false).await,
            "assert_value" => self.assert_value(args, window).await,
            "assert_count" => self.assert_count(args, window).await,
            "assert_checked" => self.assert_bool("checked", args, window, true).await,
            "assert_url" => self.assert_url(args, window).await,
            "record_start" => self.call_app_tool("record.start", None, window).await,
            "record_stop" => self.call_app_tool("record.stop", None, window).await,
            "record_status" => self.call_app_tool("record.status", None, window).await,
            "replay" => self.call_replay_tool(args, window).await,
            _ => Err(McpError::new(
                ErrorCode::METHOD_NOT_FOUND,
                format!("unknown tool: {name}"),
                None,
            )),
        }
    }

    async fn target_call(
        &self,
        method: &'static str,
        args: &JsonObject,
        window: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        let target = required_string(args, "target")?;
        self.call_app_tool(method, Some(target_params(&target)), window)
            .await
    }

    async fn call_logs_tool(
        &self,
        args: &JsonObject,
        window: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        if optional_bool(args, "clear")?.unwrap_or(false) {
            return self.call_app_tool("console.clear", None, window).await;
        }
        let mut params = Map::new();
        insert_optional_string(&mut params, args, "level")?;
        insert_optional_usize(&mut params, args, "last")?;
        self.call_app_tool("console.getLogs", Some(Value::Object(params)), window)
            .await
    }

    async fn call_network_tool(
        &self,
        args: &JsonObject,
        window: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        if optional_bool(args, "clear")?.unwrap_or(false) {
            return self.call_app_tool("network.clear", None, window).await;
        }
        let mut params = Map::new();
        insert_optional_string(&mut params, args, "filter")?;
        insert_optional_usize(&mut params, args, "last")?;
        if optional_bool(args, "failed")?.unwrap_or(false) {
            params.insert("failedOnly".into(), json!(true));
        }
        self.call_app_tool("network.getRequests", Some(Value::Object(params)), window)
            .await
    }

    async fn call_drop_tool(
        &self,
        args: JsonObject,
        window: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        let target = required_string(&args, "target")?;
        let files: Vec<PathBuf> = required_string_array(&args, "files")?
            .into_iter()
            .map(PathBuf::from)
            .collect();
        if files.is_empty() {
            return Err(invalid_params("'files' must contain at least one path"));
        }
        let mut client = match self.connect_client().await {
            Ok(client) => client,
            Err(err) => return Ok(tool_error(err)),
        };
        Ok(
            match run_drop_command(&mut client, &target, files, window.as_deref()).await {
                Ok(result) => tool_success(result),
                Err(err) => tool_error(err),
            },
        )
    }

    async fn call_replay_tool(
        &self,
        args: JsonObject,
        window: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        let path = PathBuf::from(required_string(&args, "path")?);
        let export = optional_string(&args, "export")?;
        if let Some(export) = export.as_deref() {
            return Ok(match export_replay_file(&path, export) {
                Ok(result) => tool_success(result),
                Err(err) => tool_error(err),
            });
        }
        let mut client = match self.connect_client().await {
            Ok(client) => client,
            Err(err) => return Ok(tool_error(err)),
        };
        Ok(
            match run_replay_command(&mut client, &path, None, window.as_deref()).await {
                Ok(result) => tool_success(result),
                Err(err) => tool_error(err),
            },
        )
    }

    async fn assert_text(
        &self,
        args: JsonObject,
        window: Option<String>,
        contains: bool,
    ) -> Result<CallToolResult, McpError> {
        let expected = required_string(&args, "expected")?;
        let target = required_string(&args, "target")?;
        let actual = match self
            .call_app("text", Some(target_params(&target)), window)
            .await
        {
            Ok(Value::String(actual)) => actual,
            Ok(other) => {
                return Ok(tool_error_msg(format!(
                    "expected string response, got {other}"
                )));
            }
            Err(err) => return Ok(tool_error(err)),
        };
        let passed = if contains {
            actual.contains(&expected)
        } else {
            actual == expected
        };
        if passed {
            Ok(tool_success(json!({"ok": true})))
        } else {
            let message = if contains {
                format!("text does not contain \"{expected}\", got \"{actual}\"")
            } else {
                format!("expected text \"{expected}\", got \"{actual}\"")
            };
            Ok(tool_error_msg(message))
        }
    }

    async fn assert_value(
        &self,
        args: JsonObject,
        window: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        let expected = required_string(&args, "expected")?;
        let target = required_string(&args, "target")?;
        let actual = match self
            .call_app("value", Some(target_params(&target)), window)
            .await
        {
            Ok(Value::String(actual)) => actual,
            Ok(other) => {
                return Ok(tool_error_msg(format!(
                    "expected string response, got {other}"
                )));
            }
            Err(err) => return Ok(tool_error(err)),
        };
        if actual == expected {
            Ok(tool_success(json!({"ok": true})))
        } else {
            Ok(tool_error_msg(format!(
                "expected value \"{expected}\", got \"{actual}\""
            )))
        }
    }

    async fn assert_bool(
        &self,
        method: &'static str,
        args: JsonObject,
        window: Option<String>,
        expected: bool,
    ) -> Result<CallToolResult, McpError> {
        let target = required_string(&args, "target")?;
        let field = method;
        let actual = match self
            .call_app(method, Some(target_params(&target)), window)
            .await
        {
            Ok(result) => match result.get(field).and_then(Value::as_bool) {
                Some(value) => value,
                None => return Ok(tool_error_msg(format!("missing boolean field '{field}'"))),
            },
            Err(err) => return Ok(tool_error(err)),
        };
        if actual == expected {
            Ok(tool_success(json!({"ok": true})))
        } else if method == "visible" && expected {
            Ok(tool_error_msg("element is not visible"))
        } else if method == "visible" {
            Ok(tool_error_msg("element is visible"))
        } else if method == "checked" && expected {
            Ok(tool_error_msg("element is not checked"))
        } else if method == "checked" {
            Ok(tool_error_msg("element is checked"))
        } else {
            Ok(tool_error_msg(format!(
                "element '{method}' state mismatch: expected {expected}"
            )))
        }
    }

    async fn assert_count(
        &self,
        args: JsonObject,
        window: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        let selector = required_string(&args, "selector")?;
        let expected = required_u64(&args, "expected")?;
        let actual = match self
            .call_app("count", Some(json!({"selector": selector})), window)
            .await
        {
            Ok(result) => match result.get("count").and_then(Value::as_u64) {
                Some(value) => value,
                None => return Ok(tool_error_msg("missing 'count' field")),
            },
            Err(err) => return Ok(tool_error(err)),
        };
        if actual == expected {
            Ok(tool_success(json!({"ok": true})))
        } else {
            Ok(tool_error_msg(format!(
                "expected {expected} elements, found {actual}"
            )))
        }
    }

    async fn assert_url(
        &self,
        args: JsonObject,
        window: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        let expected = required_string(&args, "expected")?;
        let actual = match self.call_app("url", None, window).await {
            Ok(Value::String(actual)) => actual,
            Ok(other) => {
                return Ok(tool_error_msg(format!(
                    "expected string response, got {other}"
                )));
            }
            Err(err) => return Ok(tool_error(err)),
        };
        if actual.contains(&expected) {
            Ok(tool_success(json!({"ok": true})))
        } else {
            Ok(tool_error_msg(format!(
                "URL does not contain \"{expected}\", got \"{actual}\""
            )))
        }
    }

    fn window_arg(&self, args: &JsonObject) -> Result<Option<String>, McpError> {
        optional_string(args, "window").map(|window| window.or_else(|| self.window.clone()))
    }
}

impl ServerHandler for PilotMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("tauri-pilot", env!("CARGO_PKG_VERSION"))
                    .with_title("tauri-pilot")
                    .with_description("MCP server for testing Tauri apps through tauri-pilot"),
            )
            .with_instructions(
                "Use these tools to inspect and control a running Tauri app through tauri-pilot.",
            )
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + MaybeSendFuture + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(tools())))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, McpError>> + MaybeSendFuture + '_ {
        let name = request.name.to_string();
        let args = request.arguments.unwrap_or_default();
        async move { self.call_tool_by_name(&name, args).await }
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        cached_tools()
            .iter()
            .find(|tool| tool.name == name)
            .cloned()
    }
}

struct ToolSpec {
    name: &'static str,
    description: &'static str,
    schema: fn() -> Arc<JsonObject>,
    read_only: bool,
    destructive: bool,
    idempotent: bool,
}

#[allow(clippy::too_many_lines)]
fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "attrs",
            description: "Get all HTML attributes for an element target.",
            schema: target_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "check",
            description: "Toggle a checkbox or radio element.",
            schema: target_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "click",
            description: "Click an element target by ref, selector, or coordinates.",
            schema: target_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "diff",
            description: "Compare the current page to the previous or supplied snapshot.",
            schema: diff_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "drag",
            description: "Drag an element to another target or by an offset.",
            schema: drag_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "drop",
            description: "Drop one or more local files on an element target.",
            schema: drop_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "eval",
            description: "Evaluate JavaScript in the WebView context.",
            schema: eval_schema,
            read_only: false,
            destructive: true,
            idempotent: false,
        },
        ToolSpec {
            name: "fill",
            description: "Clear and fill an input target with a value.",
            schema: fill_schema,
            read_only: false,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "forms",
            description: "Dump all form fields on the page or inside a selector.",
            schema: selector_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "html",
            description: "Get inner HTML for an element target, or the full page if target is omitted.",
            schema: optional_target_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "ipc",
            description: "Invoke a Tauri IPC command with optional JSON arguments.",
            schema: ipc_schema,
            read_only: false,
            destructive: true,
            idempotent: false,
        },
        ToolSpec {
            name: "logs",
            description: "Read or clear captured console logs.",
            schema: logs_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "navigate",
            description: "Navigate the WebView to a URL.",
            schema: navigate_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "network",
            description: "Read or clear captured network requests.",
            schema: network_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "ping",
            description: "Check connectivity with the running Tauri app.",
            schema: empty_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "press",
            description: "Press a keyboard key.",
            schema: press_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "record_start",
            description: "Start recording app interactions.",
            schema: empty_schema,
            read_only: false,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "record_status",
            description: "Get recorder status.",
            schema: empty_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "record_stop",
            description: "Stop recording and return recorded entries.",
            schema: empty_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "replay",
            description: "Replay or export a recorded tauri-pilot session file.",
            schema: replay_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "screenshot",
            description: "Capture the WebView or an element selector as a PNG data URL.",
            schema: selector_schema,
            read_only: true,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "scroll",
            description: "Scroll the page or an element ref.",
            schema: scroll_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "select",
            description: "Select an option in a select element.",
            schema: fill_schema,
            read_only: false,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "snapshot",
            description: "Capture an accessibility snapshot of the WebView.",
            schema: snapshot_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "state",
            description: "Get page URL, title, viewport, and scroll state.",
            schema: empty_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "storage_clear",
            description: "Clear localStorage or sessionStorage.",
            schema: session_schema,
            read_only: false,
            destructive: true,
            idempotent: true,
        },
        ToolSpec {
            name: "storage_get",
            description: "Read a localStorage or sessionStorage key.",
            schema: storage_get_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "storage_list",
            description: "List localStorage or sessionStorage entries.",
            schema: session_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "storage_set",
            description: "Set a localStorage or sessionStorage key.",
            schema: storage_set_schema,
            read_only: false,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "text",
            description: "Get text content for an element target.",
            schema: target_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "title",
            description: "Get the current page title.",
            schema: empty_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "type",
            description: "Type text into an element target without clearing it first.",
            schema: type_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "url",
            description: "Get the current page URL.",
            schema: empty_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "value",
            description: "Get an input, textarea, or select value.",
            schema: target_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "wait",
            description: "Wait for an element or condition.",
            schema: wait_schema,
            read_only: true,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "watch",
            description: "Watch for DOM mutations until the page is stable.",
            schema: watch_schema,
            read_only: true,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "windows",
            description: "List all open Tauri windows.",
            schema: global_empty_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "assert_checked",
            description: "Assert that a checkbox or radio target is checked.",
            schema: target_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "assert_contains",
            description: "Assert that target text contains a substring.",
            schema: expected_target_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "assert_count",
            description: "Assert the number of elements matching a selector.",
            schema: assert_count_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "assert_hidden",
            description: "Assert that an element target is hidden.",
            schema: target_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "assert_text",
            description: "Assert exact text content for an element target.",
            schema: expected_target_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "assert_url",
            description: "Assert that the current URL contains a substring.",
            schema: expected_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "assert_value",
            description: "Assert an input, textarea, or select value.",
            schema: expected_target_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "assert_visible",
            description: "Assert that an element target is visible.",
            schema: target_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
    ]
}

fn tools() -> Vec<Tool> {
    cached_tools().clone()
}

fn cached_tools() -> &'static Vec<Tool> {
    static TOOLS: OnceLock<Vec<Tool>> = OnceLock::new();
    TOOLS.get_or_init(build_tools)
}

fn build_tools() -> Vec<Tool> {
    let mut specs = tool_specs();
    specs.sort_by_key(|spec| spec.name);
    specs
        .into_iter()
        .map(|spec| {
            Tool::new(spec.name, spec.description, (spec.schema)()).with_annotations(
                ToolAnnotations::new()
                    .read_only(spec.read_only)
                    .destructive(spec.destructive)
                    .idempotent(spec.idempotent)
                    .open_world(false),
            )
        })
        .collect()
}

fn tool_success(result: Value) -> CallToolResult {
    let mut payload = Map::new();
    payload.insert("result".to_owned(), result);
    CallToolResult::structured(Value::Object(payload))
}

fn tool_error(err: impl std::fmt::Display) -> CallToolResult {
    tool_error_msg(err.to_string())
}

fn tool_error_msg(message: impl Into<String>) -> CallToolResult {
    CallToolResult::structured_error(json!({ "error": message.into() }))
}

fn invalid_params(message: impl Into<String>) -> McpError {
    McpError::invalid_params(message.into(), None)
}

fn required_string(args: &JsonObject, name: &str) -> Result<String, McpError> {
    args.get(name)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| invalid_params(format!("'{name}' is required and must be a string")))
}

fn optional_string(args: &JsonObject, name: &str) -> Result<Option<String>, McpError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        _ => Err(invalid_params(format!("'{name}' must be a string"))),
    }
}

fn required_u64(args: &JsonObject, name: &str) -> Result<u64, McpError> {
    args.get(name)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_params(format!("'{name}' is required and must be an integer")))
}

fn optional_u64(args: &JsonObject, name: &str) -> Result<Option<u64>, McpError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| invalid_params(format!("'{name}' must be an integer"))),
    }
}

fn optional_usize(args: &JsonObject, name: &str) -> Result<Option<usize>, McpError> {
    optional_u64(args, name)?
        .map(|value| {
            usize::try_from(value)
                .map_err(|_| invalid_params(format!("'{name}' is out of range for usize")))
        })
        .transpose()
}

fn optional_i32(args: &JsonObject, name: &str) -> Result<Option<i32>, McpError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let parsed = value
                .as_i64()
                .ok_or_else(|| invalid_params(format!("'{name}' must be an integer")))?;
            i32::try_from(parsed)
                .map(Some)
                .map_err(|_| invalid_params(format!("'{name}' is out of range for i32")))
        }
    }
}

fn optional_u8(args: &JsonObject, name: &str) -> Result<Option<u8>, McpError> {
    match optional_u64(args, name)? {
        Some(value) => u8::try_from(value)
            .map(Some)
            .map_err(|_| invalid_params(format!("'{name}' is out of range for u8"))),
        None => Ok(None),
    }
}

fn optional_bool(args: &JsonObject, name: &str) -> Result<Option<bool>, McpError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| invalid_params(format!("'{name}' must be a boolean"))),
    }
}

fn optional_ref(args: &JsonObject) -> Result<Option<String>, McpError> {
    match optional_string(args, "ref")? {
        Some(value) => Ok(Some(value.trim_start_matches('@').to_owned())),
        None => Ok(None),
    }
}

fn required_string_array(args: &JsonObject, name: &str) -> Result<Vec<String>, McpError> {
    let values = args
        .get(name)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_params(format!("'{name}' is required and must be an array")))?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| invalid_params(format!("'{name}' must contain only strings")))
        })
        .collect()
}

fn insert_optional_string(
    params: &mut Map<String, Value>,
    args: &JsonObject,
    name: &str,
) -> Result<(), McpError> {
    if let Some(value) = optional_string(args, name)? {
        params.insert(name.to_owned(), json!(value));
    }
    Ok(())
}

fn insert_optional_usize(
    params: &mut Map<String, Value>,
    args: &JsonObject,
    name: &str,
) -> Result<(), McpError> {
    if let Some(value) = optional_usize(args, name)? {
        params.insert(name.to_owned(), json!(value));
    }
    Ok(())
}

fn empty_schema() -> Arc<JsonObject> {
    object_schema(Map::new(), &[])
}

fn global_empty_schema() -> Arc<JsonObject> {
    let mut schema = Map::new();
    schema.insert("type".to_owned(), json!("object"));
    schema.insert("properties".to_owned(), Value::Object(Map::new()));
    schema.insert("additionalProperties".to_owned(), json!(false));
    Arc::new(schema)
}

fn target_schema() -> Arc<JsonObject> {
    object_schema(
        props([(
            "target",
            string_prop("Element ref, CSS selector, or x,y coordinates."),
        )]),
        &["target"],
    )
}

fn optional_target_schema() -> Arc<JsonObject> {
    object_schema(
        props([(
            "target",
            string_prop("Optional element ref, CSS selector, or x,y coordinates."),
        )]),
        &[],
    )
}

fn expected_schema() -> Arc<JsonObject> {
    object_schema(
        props([("expected", string_prop("Expected value or substring."))]),
        &["expected"],
    )
}

fn expected_target_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "target",
                string_prop("Element ref, CSS selector, or x,y coordinates."),
            ),
            ("expected", string_prop("Expected value or substring.")),
        ]),
        &["target", "expected"],
    )
}

fn snapshot_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "interactive",
                bool_prop("Only include interactive elements."),
            ),
            (
                "selector",
                string_prop("CSS selector to scope the snapshot."),
            ),
            ("depth", integer_prop("Maximum traversal depth.")),
        ]),
        &[],
    )
}

fn diff_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "interactive",
                bool_prop("Only include interactive elements."),
            ),
            (
                "selector",
                string_prop("CSS selector to scope the new snapshot."),
            ),
            ("depth", integer_prop("Maximum traversal depth.")),
            (
                "reference",
                any_prop("Optional prior snapshot object to compare against."),
            ),
        ]),
        &[],
    )
}

fn fill_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "target",
                string_prop("Element ref, CSS selector, or x,y coordinates."),
            ),
            ("value", string_prop("Value to set.")),
        ]),
        &["target", "value"],
    )
}

fn type_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "target",
                string_prop("Element ref, CSS selector, or x,y coordinates."),
            ),
            ("text", string_prop("Text to type.")),
        ]),
        &["target", "text"],
    )
}

fn press_schema() -> Arc<JsonObject> {
    object_schema(
        props([("key", string_prop("Keyboard key to press."))]),
        &["key"],
    )
}

fn scroll_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "direction",
                enum_prop("Direction to scroll.", &["up", "down", "left", "right"]),
            ),
            ("amount", integer_prop("Pixel amount to scroll.")),
            (
                "ref",
                string_prop("Optional element ref, with or without @."),
            ),
        ]),
        &[],
    )
}

fn drag_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "source",
                string_prop("Source element ref, selector, or coordinates."),
            ),
            (
                "target",
                string_prop("Destination element ref, selector, or coordinates."),
            ),
            (
                "offset",
                any_prop("Optional offset object such as {\"x\": 0, \"y\": 100}."),
            ),
        ]),
        &["source"],
    )
}

fn drop_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "target",
                string_prop("Drop target ref, selector, or coordinates."),
            ),
            ("files", array_string_prop("Local file paths to drop.")),
        ]),
        &["target", "files"],
    )
}

fn eval_schema() -> Arc<JsonObject> {
    object_schema(
        props([("script", string_prop("JavaScript to evaluate."))]),
        &["script"],
    )
}

fn ipc_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            ("command", string_prop("Tauri IPC command name.")),
            (
                "args",
                any_prop("Optional JSON object of command arguments."),
            ),
        ]),
        &["command"],
    )
}

fn selector_schema() -> Arc<JsonObject> {
    object_schema(
        props([("selector", string_prop("Optional CSS selector."))]),
        &[],
    )
}

fn navigate_schema() -> Arc<JsonObject> {
    object_schema(
        props([("url", string_prop("URL to navigate to."))]),
        &["url"],
    )
}

fn wait_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "target",
                string_prop("Element ref, CSS selector, or x,y coordinates."),
            ),
            ("selector", string_prop("CSS selector to wait for.")),
            ("gone", bool_prop("Wait for the element to disappear.")),
            ("timeout", integer_prop("Timeout in milliseconds.")),
        ]),
        &[],
    )
}

fn watch_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "selector",
                string_prop("CSS selector to scope observation."),
            ),
            ("timeout", integer_prop("Timeout in milliseconds.")),
            ("stable", integer_prop("Stability window in milliseconds.")),
            (
                "require_mutation",
                bool_prop(
                    "Defer the stability timer until at least one DOM mutation occurs. \
                     Rejects on timeout when nothing changed.",
                ),
            ),
        ]),
        &[],
    )
}

fn logs_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "level",
                enum_prop(
                    "Optional log level filter.",
                    &["log", "info", "warn", "error"],
                ),
            ),
            ("last", integer_prop("Return only the last N log entries.")),
            (
                "clear",
                bool_prop("Clear the log buffer instead of reading it."),
            ),
        ]),
        &[],
    )
}

fn network_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            ("filter", string_prop("Optional URL substring filter.")),
            ("failed", bool_prop("Only return failed requests.")),
            ("last", integer_prop("Return only the last N requests.")),
            (
                "clear",
                bool_prop("Clear the request buffer instead of reading it."),
            ),
        ]),
        &[],
    )
}

fn session_schema() -> Arc<JsonObject> {
    object_schema(
        props([(
            "session",
            bool_prop("Use sessionStorage instead of localStorage."),
        )]),
        &[],
    )
}

fn storage_get_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            ("key", string_prop("Storage key.")),
            (
                "session",
                bool_prop("Use sessionStorage instead of localStorage."),
            ),
        ]),
        &["key"],
    )
}

fn storage_set_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            ("key", string_prop("Storage key.")),
            ("value", string_prop("Storage value.")),
            (
                "session",
                bool_prop("Use sessionStorage instead of localStorage."),
            ),
        ]),
        &["key", "value"],
    )
}

fn assert_count_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            ("selector", string_prop("CSS selector to count.")),
            ("expected", integer_prop("Expected element count.")),
        ]),
        &["selector", "expected"],
    )
}

fn replay_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            ("path", string_prop("Path to a recording JSON file.")),
            (
                "export",
                enum_prop("Export format instead of replaying.", &["sh"]),
            ),
        ]),
        &["path"],
    )
}

fn object_schema(mut properties: Map<String, Value>, required: &[&str]) -> Arc<JsonObject> {
    properties.insert(
        "window".to_owned(),
        string_prop("Optional Tauri window label overriding the MCP server default."),
    );
    let mut schema = Map::new();
    schema.insert("type".to_owned(), json!("object"));
    schema.insert("properties".to_owned(), Value::Object(properties));
    if !required.is_empty() {
        schema.insert("required".to_owned(), json!(required));
    }
    schema.insert("additionalProperties".to_owned(), json!(false));
    Arc::new(schema)
}

fn props<const N: usize>(properties: [(&str, Value); N]) -> Map<String, Value> {
    properties
        .into_iter()
        .map(|(name, schema)| (name.to_owned(), schema))
        .collect()
}

fn string_prop(description: &str) -> Value {
    json!({"type": "string", "description": description})
}

fn bool_prop(description: &str) -> Value {
    json!({"type": "boolean", "description": description})
}

fn integer_prop(description: &str) -> Value {
    json!({"type": "integer", "description": description})
}

fn array_string_prop(description: &str) -> Value {
    json!({"type": "array", "items": {"type": "string"}, "description": description})
}

fn any_prop(description: &str) -> Value {
    json!({"description": description})
}

fn enum_prop(description: &str, values: &[&str]) -> Value {
    json!({"type": "string", "enum": values, "description": description})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::protocol::{Request, Response};
    #[cfg(unix)]
    use serial_test::serial;
    #[cfg(unix)]
    use tokio::net::UnixListener;
    #[cfg(unix)]
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        task::JoinHandle,
    };

    #[test]
    fn tool_list_matches_cli_command_surface() {
        let tools = tools();
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
        let expected = vec![
            "assert_checked",
            "assert_contains",
            "assert_count",
            "assert_hidden",
            "assert_text",
            "assert_url",
            "assert_value",
            "assert_visible",
            "attrs",
            "check",
            "click",
            "diff",
            "drag",
            "drop",
            "eval",
            "fill",
            "forms",
            "html",
            "ipc",
            "logs",
            "navigate",
            "network",
            "ping",
            "press",
            "record_start",
            "record_status",
            "record_stop",
            "replay",
            "screenshot",
            "scroll",
            "select",
            "snapshot",
            "state",
            "storage_clear",
            "storage_get",
            "storage_list",
            "storage_set",
            "text",
            "title",
            "type",
            "url",
            "value",
            "wait",
            "watch",
            "windows",
        ];
        assert_eq!(names, expected);
    }

    #[test]
    fn schemas_include_window_override() {
        let schema = target_schema();
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("schema has properties");
        assert!(properties.contains_key("target"));
        assert!(properties.contains_key("window"));
    }

    #[test]
    fn windows_schema_omits_window_override() {
        let schema = global_empty_schema();
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("schema has properties");
        assert!(!properties.contains_key("window"));
    }

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

        let result = pilot
            .call_tool_by_name("replay", args)
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
        let result = pilot
            .call_tool_by_name("click", args)
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
        pilot
            .call_tool_by_name("click", args)
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

    /// Every tool registered in `tools()` has a unique name.
    #[test]
    fn test_tools_registry_has_unique_names() {
        let tools = tools();
        let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        names.sort_unstable();
        let total = names.len();
        names.dedup();
        assert_eq!(
            total,
            names.len(),
            "duplicate tool names: total={total}, unique={}",
            names.len()
        );
    }

    /// All known tool names (canonical list at PR1 baseline). Must stay stable
    /// across the upcoming Task 4–8 module split — additions/removals require
    /// updating BOTH this list and the `tools()` registry in lock-step.
    #[test]
    fn test_tools_registry_contains_baseline_tools() {
        let tools = tools();
        let names: std::collections::HashSet<&str> =
            tools.iter().map(|t| t.name.as_ref()).collect();

        let expected = [
            // core
            "ping",
            "windows",
            "state",
            "snapshot",
            "diff",
            "screenshot",
            "navigate",
            "url",
            "title",
            "wait",
            // interact
            "click",
            "fill",
            "type",
            "press",
            "select",
            "check",
            "scroll",
            "drag",
            "drop",
            // inspect
            "text",
            "html",
            "value",
            "attrs",
            // eval
            "eval",
            "ipc",
            // observe
            "watch",
            "logs",
            "network",
            // storage
            "storage_get",
            "storage_set",
            "storage_list",
            "storage_clear",
            "forms",
            // assert
            "assert_text",
            "assert_contains",
            "assert_visible",
            "assert_hidden",
            "assert_value",
            "assert_count",
            "assert_checked",
            "assert_url",
            // record
            "record_start",
            "record_stop",
            "record_status",
            "replay",
        ];

        for name in expected {
            assert!(
                names.contains(name),
                "expected tool '{name}' missing from registry"
            );
        }
    }

    /// Tool count matches baseline. If you add a tool, bump this number AND
    /// add it to `test_tools_registry_contains_baseline_tools`.
    #[test]
    fn test_tools_registry_count_baseline() {
        let count = tools().len();
        // Snapshot of pre-split registry. If you intentionally add a tool, update.
        assert_eq!(count, 45, "tool count drifted from baseline");
    }
}
