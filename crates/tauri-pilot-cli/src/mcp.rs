use std::{
    io::IsTerminal,
    path::Path,
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::Duration,
};

use anyhow::Result;
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ErrorCode, Implementation,
        JsonObject, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
        Tool, ToolAnnotations,
    },
    service::{MaybeSendFuture, RequestContext, RoleServer},
    transport::stdio,
};
use serde_json::{Map, Value, json};

use crate::{
    build_scroll_params, build_wait_params, client::Client, export_replay_file, protocol::RpcError,
    resolve_socket, run_drop_command, run_replay_command, scenario, target_params, with_window,
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
            Err(err) => tool_error(&err),
        })
    }

    #[allow(clippy::too_many_lines)]
    async fn call_tool_by_name(
        &self,
        name: &str,
        args: JsonObject,
    ) -> Result<CallToolResult, McpError> {
        let name = normalize_tool_name(name);
        if DANGEROUS_MCP_TOOLS.contains(&name) && !dangerous_mcp_tools_enabled() {
            return Err(invalid_params(format!(
                "tool '{name}' is disabled by default. Set {ENABLE_DANGEROUS_MCP_TOOLS_ENV}=1 to enable dangerous MCP tools."
            )));
        }
        let window = self.window_arg(&args)?;
        match name {
            "ping" => self.call_app_tool("ping", None, window).await,
            "windows" => self.call_app_tool("windows.list", None, None).await,
            "state" => self.call_app_tool("state", None, window).await,
            "snapshot" => {
                self.call_app_tool(
                    "snapshot",
                    Some(json!({
                        "interactive": optional_bool(&args, "interactive")?.unwrap_or(false),
                        "selector": optional_string(&args, "selector")?,
                        "depth": optional_u8(&args, "depth")?,
                    })),
                    window,
                )
                .await
            }
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
                // `additionalProperties: false` is advertised in `tools/list`
                // but never enforced here, so a client still sending the old
                // `{"ref": "e12"}` would scroll the page instead of the
                // element. Fail loudly rather than silently (#216).
                if args.contains_key("ref") {
                    return Err(invalid_params(
                        "scroll no longer takes 'ref'; pass the element through 'target' (a bare 'e12' is accepted)",
                    ));
                }
                let target = optional_string(&args, "target")?;
                self.call_app_tool(
                    "scroll",
                    Some(build_scroll_params(
                        &optional_string(&args, "direction")?.unwrap_or_else(|| "down".to_owned()),
                        optional_i32(&args, "amount")?,
                        target.as_deref(),
                    )),
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
                // Gesture tunables are optional; the bridge applies its own
                // defaults and clamps when they are absent.
                for key in ["steps", "stepDelayMs", "settleMs"] {
                    if let Some(value) = optional_i32(&args, key)? {
                        params[key] = json!(value);
                    }
                }
                self.call_app_tool("drag", Some(params), window).await
            }
            "drop" => self.call_drop_tool(args, window).await,
            "text" => self.target_call("text", &args, window).await,
            "html" => {
                let params = optional_string(&args, "target")?.map(|target| target_params(&target));
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
            "screenshot_native" => {
                let mut payload = json!({
                    "window_id": required_u32(&args, "window_id")?,
                    "output_path": required_string(&args, "output_path")?,
                });
                if let Some(format) = optional_string(&args, "format")? {
                    payload["format"] = json!(format);
                }
                self.call_app_tool("screenshot_native", Some(payload), window)
                    .await
            }
            "navigate" => {
                let url = required_string(&args, "url")?;
                validate_navigate_url(&url)?;
                self.call_app_tool("navigate", Some(json!({ "url": url })), window)
                    .await
            }
            "url" => self.call_app_tool("url", None, window).await,
            "title" => self.call_app_tool("title", None, window).await,
            "wait" => {
                let target = optional_string(&args, "target")?;
                let selector = optional_string(&args, "selector")?;
                let gone = optional_bool(&args, "gone")?.unwrap_or(false);
                let timeout = optional_u64(&args, "timeout")?.unwrap_or(10_000);
                let params =
                    build_wait_params(target.as_deref(), selector.as_deref(), gone, timeout);
                self.call_app_tool("wait", Some(params), window).await
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
                self.call_app_tool("watch", Some(watch_params), window)
                    .await
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
                let params = optional_string(&args, "selector")?
                    .map(|selector| json!({ "selector": selector }));
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
            "run" => self.call_run_tool(args, window).await,
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
            Err(err) => return Ok(tool_error(&err)),
        };
        Ok(
            match run_drop_command(&mut client, &target, files, window.as_deref()).await {
                Ok(result) => tool_success(result),
                Err(err) => tool_error(&err),
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
                Err(err) => tool_error(&err),
            });
        }
        let mut client = match self.connect_client().await {
            Ok(client) => client,
            Err(err) => return Ok(tool_error(&err)),
        };
        Ok(
            match run_replay_command(&mut client, &path, None, window.as_deref()).await {
                Ok(result) => tool_success(result),
                Err(err) => tool_error(&err),
            },
        )
    }

    async fn call_run_tool(
        &self,
        args: JsonObject,
        window: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        let path = optional_string(&args, "path")?;
        let content = optional_string(&args, "content")?;
        let fail_fast = optional_bool(&args, "fail_fast")?;
        // An empty string is "not set", not "the server's working directory":
        // `Path::new("").join(name)` is a bare filename (#215).
        let screenshots_dir = optional_string(&args, "screenshots_dir")?
            .filter(|dir| !dir.trim().is_empty())
            .map_or_else(default_screenshots_dir, PathBuf::from);
        let scenario = match (path, content) {
            (Some(_), Some(_)) => {
                return Err(invalid_params(
                    "run accepts either 'path' or 'content', not both",
                ));
            }
            (None, None) => {
                return Err(invalid_params("run requires either 'path' or 'content'"));
            }
            (Some(path), None) => match scenario::load_scenario(Path::new(&path)) {
                Ok(scenario) => scenario,
                Err(err) => return Ok(tool_error_msg(format!("{err:#}"))),
            },
            (None, Some(content)) => match scenario::parse_scenario(&content) {
                Ok(scenario) => scenario,
                Err(err) => return Ok(tool_error_msg(format!("{err:#}"))),
            },
        };
        if scenario.step.is_empty() {
            return Err(invalid_params("run requires at least one [[step]] entry"));
        }
        validate_run_scenario_steps(&scenario)?;
        let mut client = match self.connect_for_run(&scenario).await {
            Ok(client) => client,
            Err(err) => return Ok(tool_error_msg(format!("{err:#}"))),
        };
        Ok(
            match scenario::run_scenario(
                &mut client,
                &scenario,
                window.as_deref(),
                fail_fast,
                &screenshots_dir,
            )
            .await
            {
                Ok(report) => tool_success(scenario::report_to_json(&report)),
                Err(err) => tool_error_msg(format!("{err:#}")),
            },
        )
    }

    async fn connect_for_run(&self, scenario: &scenario::Scenario) -> Result<Client> {
        let connect = scenario.connect.as_ref();
        let timeout_ms = connect.and_then(|c| c.timeout_ms);
        let connect_fut = async {
            match (
                self.socket.as_ref(),
                connect.and_then(|c| c.socket.as_ref()),
            ) {
                (None, Some(socket)) => Client::connect(socket).await,
                _ => self.connect_client().await,
            }
        };
        match timeout_ms {
            Some(ms) => tokio::time::timeout(Duration::from_millis(ms), connect_fut)
                .await
                .map_err(|_elapsed| anyhow::anyhow!("connection timed out after {ms}ms"))?,
            None => connect_fut.await,
        }
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
            Err(err) => return Ok(tool_error(&err)),
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
            Err(err) => return Ok(tool_error(&err)),
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
            Err(err) => return Ok(tool_error(&err)),
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
            Err(err) => return Ok(tool_error(&err)),
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
            Err(err) => return Ok(tool_error(&err)),
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
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
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
    ) -> impl Future<Output = Result<CallToolResponse, McpError>> + MaybeSendFuture + '_ {
        let name = request.name.to_string();
        let args = request.arguments.unwrap_or_default();
        async move { self.call_tool_by_name(&name, args).await.map(Into::into) }
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        let name = namespaced_tool_name(name);
        cached_tools()
            .iter()
            .find(|tool| tool.name == name.as_str())
            .cloned()
    }
}

const PILOT_PREFIX: &str = "pilot.";
const ENABLE_DANGEROUS_MCP_TOOLS_ENV: &str = "TAURI_PILOT_MCP_ENABLE_DANGEROUS_TOOLS";
const DANGEROUS_MCP_TOOLS: &[&str] = &["drop", "eval", "ipc"];

fn normalize_tool_name(name: &str) -> &str {
    name.strip_prefix(PILOT_PREFIX).unwrap_or(name)
}

fn namespaced_tool_name(name: &str) -> String {
    if name.starts_with(PILOT_PREFIX) {
        name.to_owned()
    } else {
        format!("{PILOT_PREFIX}{name}")
    }
}

fn dangerous_mcp_tools_enabled() -> bool {
    std::env::var(ENABLE_DANGEROUS_MCP_TOOLS_ENV).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn validate_run_scenario_steps(scenario: &scenario::Scenario) -> Result<(), McpError> {
    let dangerous_enabled = dangerous_mcp_tools_enabled();
    for step in &scenario.step {
        if !dangerous_enabled && DANGEROUS_MCP_TOOLS.contains(&step.action.as_str()) {
            return Err(invalid_params(format!(
                "scenario step action '{}' is disabled by default. Set {ENABLE_DANGEROUS_MCP_TOOLS_ENV}=1 to enable dangerous MCP tools.",
                step.action
            )));
        }
        if step.action == "navigate"
            && let Some(url) = step.url.as_deref()
        {
            validate_navigate_url(url)?;
        }
        if step.action == "screenshot" && step.path.is_some() {
            return Err(invalid_params("run screenshot steps cannot set 'path'"));
        }
    }
    Ok(())
}

fn validate_navigate_url(url: &str) -> Result<(), McpError> {
    // Mirror the normalization a browser's URL parser applies before it
    // resolves the scheme, so a crafted string can't smuggle a `javascript:`
    // URL past this filter and reach `window.location.href` in the bridge:
    //   * ASCII tab/LF/CR are stripped from anywhere in the input, so
    //     `java\tscript:` and `java\nscript:` collapse to `javascript:`.
    //   * Leading C0 controls and spaces (scalar value <= U+0020) are removed,
    //     so `\u{0}javascript:` collapses to `javascript:`.
    let stripped: String = url
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    if stripped
        .trim_start_matches(|c| c <= '\u{20}')
        .to_ascii_lowercase()
        .starts_with("javascript:")
    {
        return Err(invalid_params(
            "navigate does not allow javascript: URLs for security reasons",
        ));
    }
    Ok(())
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
            description: "Toggle an <input type=checkbox>, or select an <input type=radio> (already selected stays selected). Errors on any other element.",
            schema: target_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "click",
            description: "Click an element.",
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
            description: "Clear and fill an input, textarea, select, or contenteditable target. Errors if the target cannot take a value.",
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
            description: "Stop recording and return recorded entries. Errors if no recording is in progress.",
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
            name: "run",
            description: "Execute a declarative TOML scenario and return a JSON report (`ok`, counts, `summary`, `steps`). A finished run including failed steps is a successful tool result with `ok` false; only parse, step-key, I/O, connect, and timeout failures are tool errors.",
            schema: run_schema,
            read_only: false,
            destructive: true,
            idempotent: false,
        },
        ToolSpec {
            name: "screenshot",
            description: "Capture the full page or an element selector as a PNG data URL.",
            schema: selector_schema,
            read_only: true,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "screenshot_native",
            description: "Capture a native window by `window_id` and write a PNG to `output_path`. Returns path + metadata; never inlines bytes.",
            schema: pilot_screenshot_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "scroll",
            description: "Scroll the page or an element.",
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
            description: "Type text into an input, textarea, or contenteditable target without clearing it first. Errors on <select> and on any target that cannot take a value.",
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
    build_tools_with_flag(dangerous_mcp_tools_enabled())
}

fn build_tools_with_flag(enable_dangerous_tools: bool) -> Vec<Tool> {
    let mut specs = tool_specs();
    if !enable_dangerous_tools {
        specs.retain(|spec| !DANGEROUS_MCP_TOOLS.contains(&spec.name));
    }
    specs.sort_by_key(|spec| spec.name);
    specs
        .into_iter()
        .map(|spec| {
            Tool::new(
                namespaced_tool_name(spec.name),
                spec.description,
                (spec.schema)(),
            )
            .with_annotations(
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

/// Turns a failed call into a tool error, keeping an app error's fields.
///
/// An [`RpcError`] anywhere in the chain comes out as its `data` object plus
/// `message` and `rpc_code`, so a client reads `error: WINDOW_NOT_FOUND` and
/// `available_windows` without parsing text (#242). `error` falls back to the
/// message when the app sent no string domain code. A distinct `data` value
/// under one of those keys moves to `data_<key>` (prefixed again while that
/// key is taken) rather than being dropped: the plugin and CLI ship
/// separately. Anything else is its text.
fn tool_error(err: &anyhow::Error) -> CallToolResult {
    let Some(rpc) = err
        .chain()
        .find_map(|cause| cause.downcast_ref::<RpcError>())
    else {
        return tool_error_msg(err.to_string());
    };
    let mut fields = match &rpc.data {
        Some(Value::Object(data)) => data.clone(),
        Some(data) if !data.is_null() => Map::from_iter([("data".to_owned(), data.clone())]),
        _ => Map::new(),
    };
    let has_code = fields.get("error").is_some_and(Value::is_string);
    let mut set = |key: &str, value: Value| {
        if let Some(old) = fields.insert(key.to_owned(), value.clone())
            && old != value
            && !old.is_null()
        {
            let mut backup = format!("data_{key}");
            while fields.contains_key(&backup) {
                backup.insert_str(0, "data_");
            }
            fields.insert(backup, old);
        }
    };
    let message = Value::String(rpc.message.clone());
    if !has_code {
        set("error", message.clone());
    }
    set("message", message);
    set("rpc_code", json!(rpc.code));
    CallToolResult::structured_error(Value::Object(fields))
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

fn required_u32(args: &JsonObject, name: &str) -> Result<u32, McpError> {
    let value = required_u64(args, name)?;
    u32::try_from(value).map_err(|_| invalid_params(format!("'{name}' is out of range for u32")))
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
        props([("target", target_prop("Element to act on"))]),
        &["target"],
    )
}

fn optional_target_schema() -> Arc<JsonObject> {
    object_schema(
        props([(
            "target",
            target_prop_no_coords("Optional element to act on"),
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
            ("target", target_prop("Element to act on")),
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
            ("target", target_prop("Element to act on")),
            ("value", string_prop("Value to set.")),
        ]),
        &["target", "value"],
    )
}

fn type_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            ("target", target_prop("Element to act on")),
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
                enum_prop(
                    "Direction to scroll.",
                    &["up", "down", "left", "right", "top", "bottom"],
                ),
            ),
            ("amount", integer_prop("Pixel amount to scroll.")),
            (
                "target",
                target_prop("Optional element to scroll, defaulting to the page"),
            ),
        ]),
        &[],
    )
}

fn drag_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            ("source", target_prop("Element to drag")),
            ("target", target_prop("Element to drop onto")),
            (
                "offset",
                any_prop("Optional offset object such as {\"x\": 0, \"y\": 100}."),
            ),
            (
                "steps",
                integer_prop("Move events to emit, 1-60 (default 12)."),
            ),
            (
                "stepDelayMs",
                integer_prop("Pause between move events in ms (default 16)."),
            ),
            (
                "settleMs",
                integer_prop(
                    "Wait after the release in ms, for async state updates to land (default 250).",
                ),
            ),
        ]),
        &["source"],
    )
}

fn drop_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            ("target", target_prop("Element to drop the files onto")),
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

fn pilot_screenshot_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "window_id",
                json!({
                    "type": "integer",
                    "minimum": 0,
                    "maximum": u32::MAX,
                    "description": "Native window id (e.g. `CGWindowID` on macOS) to capture. Must fit in u32.",
                }),
            ),
            (
                "output_path",
                string_prop(
                    "Absolute path where the PNG is written. The parent directory must already exist; the file is written atomically via a temp + rename.",
                ),
            ),
            (
                "format",
                enum_prop("Image format. v1 accepts only \"png\".", &["png"]),
            ),
        ]),
        &["window_id", "output_path"],
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
            ("target", target_prop_no_coords("Element to act on")),
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

/// Failure screenshot directory used when the caller passes none.
///
/// The MCP server inherits its working directory from the client that spawned
/// it, so the CLI default (`./tauri-pilot-failures`) would land anywhere.
///
/// A fixed name in a world-writable temp directory is no better: the first
/// user to create it owns it and everyone else gets `EACCES`, and a local
/// user can pre-create the predictable name as a symlink and collect
/// screenshots that routinely show login screens. On Unix the directory is
/// therefore per-user and created `0700`, under `$XDG_RUNTIME_DIR` when that
/// is private, falling back to the temp directory. Windows needs none of this:
/// `temp_dir()` is already per-user there.
fn default_screenshots_dir() -> PathBuf {
    #[cfg(unix)]
    {
        private_screenshots_dir()
    }
    #[cfg(not(unix))]
    {
        std::env::temp_dir().join(scenario::DEFAULT_SCREENSHOT_DIR)
    }
}

/// Returns true if `path` is a directory owned by us with no group/world bits.
#[cfg(unix)]
fn is_private_dir(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    // `symlink_metadata` so a symlink pointing at a private directory of ours
    // is rejected instead of vouching for the link's target.
    match std::fs::symlink_metadata(path) {
        // SAFETY: getuid() has no preconditions.
        Ok(m) => {
            m.is_dir() && m.uid() == unsafe { libc::getuid() } && m.mode().trailing_zeros() >= 6
        }
        Err(_) => false,
    }
}

/// Creates (or reuses) an owner-only per-user screenshot directory.
///
/// Returns the path even when it could not be secured, so the caller reports
/// the write failure as `screenshot_error` rather than silently writing
/// somewhere world-readable.
#[cfg(unix)]
fn private_screenshots_dir() -> PathBuf {
    use std::os::unix::fs::DirBuilderExt;

    // SAFETY: getuid() has no preconditions.
    let uid = unsafe { libc::getuid() };
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .filter(|dir| is_private_dir(dir))
        .unwrap_or_else(std::env::temp_dir);

    // The uid keeps two users on the same host off each other's directory;
    // the pid suffix is the escape hatch when the plain name is squatted.
    let names = [
        format!("{}-{uid}", scenario::DEFAULT_SCREENSHOT_DIR),
        format!(
            "{}-{uid}-{}",
            scenario::DEFAULT_SCREENSHOT_DIR,
            std::process::id()
        ),
    ];
    let mut last = base.join(names[0].as_str());
    for name in &names {
        let candidate = base.join(name.as_str());
        let _ = std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&candidate);
        if is_private_dir(&candidate) {
            return candidate;
        }
        last = candidate;
    }
    last
}

fn run_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            ("path", string_prop("Path to a scenario TOML file.")),
            (
                "content",
                string_prop("Inline scenario TOML. Mutually exclusive with path."),
            ),
            (
                "fail_fast",
                bool_prop(
                    "Override the scenario file fail_fast setting. When omitted, the TOML value is used (default true).",
                ),
            ),
            (
                "screenshots_dir",
                string_prop(
                    "Directory for failure screenshots. Defaults to an owner-only per-user tauri-pilot-failures directory under $XDG_RUNTIME_DIR or the system temp directory. Failed steps report the file as 'screenshot', or the reason as 'screenshot_error'.",
                ),
            ),
        ]),
        &[],
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

/// Wording shared by every element-targeting property (#216).
///
/// The bare `e12` form is spelled out because `snapshot` prints `[ref=e12]`
/// and agents copy that id verbatim; it used to reach `click` and `value` as
/// a CSS selector and fail with `No element matches selector: e12`.
const TARGET_SHAPES: &str = "snapshot ref (e12 or @e12), CSS selector, or x,y coordinates";

/// Wording for the tools that resolve a target without hit-testing (#216).
///
/// `wait` sends `x,y` on to `querySelector`, which throws, and `html` ignores
/// it and returns the whole page, so neither may advertise coordinates.
const TARGET_SHAPES_NO_COORDS: &str = "snapshot ref (e12 or @e12) or CSS selector";

/// Build the description of an element-targeting property.
fn target_prop(role: &str) -> Value {
    string_prop(&format!("{role}: {TARGET_SHAPES}."))
}

/// Build the description of a target that does not accept coordinates.
fn target_prop_no_coords(role: &str) -> Value {
    string_prop(&format!("{role}: {TARGET_SHAPES_NO_COORDS}."))
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
        // Validate the complete tool surface; runtime gating of dangerous tools
        // is covered by `dangerous_tools_hidden_by_default` / `dangerous_tools_can_be_enabled`.
        let tools = build_tools_with_flag(true);
        assert!(
            tools
                .iter()
                .all(|tool| tool.name.as_ref().starts_with(PILOT_PREFIX))
        );
        let names: Vec<&str> = tools
            .iter()
            .map(|tool| normalize_tool_name(tool.name.as_ref()))
            .collect();
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
            "run",
            "screenshot",
            "screenshot_native",
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
    fn tool_name_helpers_are_round_trip_symmetric() {
        assert_eq!(normalize_tool_name("click"), "click");
        assert_eq!(normalize_tool_name("pilot.click"), "click");
        assert_eq!(namespaced_tool_name("click"), "pilot.click");
        assert_eq!(namespaced_tool_name("pilot.click"), "pilot.click");
    }

    #[test]
    fn tool_name_helpers_handle_corner_cases() {
        // normalize_tool_name: empty, bare prefix, double prefix
        assert_eq!(normalize_tool_name(""), "");
        assert_eq!(normalize_tool_name(PILOT_PREFIX), "");
        // Single-strip is intentional: a double prefix loses only the outer one.
        assert_eq!(normalize_tool_name("pilot.pilot.click"), "pilot.click");

        // namespaced_tool_name: empty, bare prefix, already-namespaced
        assert_eq!(namespaced_tool_name(""), PILOT_PREFIX);
        assert_eq!(namespaced_tool_name(PILOT_PREFIX), PILOT_PREFIX);
        assert_eq!(namespaced_tool_name("pilot.click"), "pilot.click");
    }

    #[test]
    fn get_tool_resolves_bare_and_prefixed_names_to_same_tool() {
        let pilot = PilotMcpServer::new(None, None);
        let bare = pilot.get_tool("click").expect("bare name resolves");
        let prefixed = pilot
            .get_tool("pilot.click")
            .expect("prefixed name resolves");
        assert_eq!(bare.name, prefixed.name);
        assert_eq!(bare.description, prefixed.description);
        assert_eq!(bare.name.as_ref(), "pilot.click");
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
    fn run_schema_properties_include_run_options_and_window() {
        let schema = run_schema();
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("schema has properties");
        for key in ["path", "content", "fail_fast", "screenshots_dir", "window"] {
            assert!(
                properties.contains_key(key),
                "run schema must advertise `{key}`"
            );
        }
        match schema.get("required") {
            None => {}
            Some(Value::Array(required)) => {
                assert!(
                    required.is_empty(),
                    "run schema must not require any fields, got {required:?}"
                );
            }
            other => panic!("unexpected required field: {other:?}"),
        }
    }

    #[test]
    fn scroll_schema_accepts_top_and_bottom_directions() {
        let schema = scroll_schema();
        let direction = schema
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|p| p.get("direction"))
            .and_then(Value::as_object)
            .expect("scroll schema has direction property");
        let values: Vec<&str> = direction
            .get("enum")
            .and_then(Value::as_array)
            .expect("direction has enum")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(
            values.len(),
            6,
            "scroll direction enum must have exactly 6 string variants, got {values:?}"
        );
        for expected in ["up", "down", "left", "right", "top", "bottom"] {
            assert!(
                values.contains(&expected),
                "scroll direction enum must include {expected}"
            );
        }
    }

    /// Every element-targeting property spells out the same shapes (#216).
    #[test]
    fn target_properties_share_one_wording() {
        for spec in tool_specs() {
            let schema = (spec.schema)();
            let properties = schema
                .get("properties")
                .and_then(Value::as_object)
                .expect("schema has properties");
            for name in ["target", "source"] {
                let Some(prop) = properties.get(name) else {
                    continue;
                };
                let description = prop
                    .get("description")
                    .and_then(Value::as_str)
                    .expect("property has a description");
                assert!(
                    description.contains(TARGET_SHAPES)
                        || description.contains(TARGET_SHAPES_NO_COORDS),
                    "pilot.{} `{name}` must describe the accepted shapes, got: {description}",
                    spec.name
                );
            }
        }
    }

    /// The shared wording is pinned, not just shared (#216).
    ///
    /// `target_properties_share_one_wording` compares descriptions against the
    /// same constant that builds them, so it stays green whatever the constant
    /// says. These two assertions are what fails when the shapes change.
    #[test]
    fn target_descriptions_name_the_accepted_shapes() {
        assert_eq!(
            target_description(&target_schema()),
            "Element to act on: snapshot ref (e12 or @e12), CSS selector, or x,y coordinates."
        );
        assert_eq!(
            target_description(&wait_schema()),
            "Element to act on: snapshot ref (e12 or @e12) or CSS selector."
        );
    }

    /// Read the `target` property description out of a tool schema.
    fn target_description(schema: &JsonObject) -> &str {
        schema
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|props| props.get("target"))
            .and_then(|prop| prop.get("description"))
            .and_then(Value::as_str)
            .expect("schema has a target description")
    }

    /// `ref` is not a second name for `target` on any tool (#216).
    ///
    /// A bare `e12` now parses as a ref through `target`, so the alias only
    /// gave agents two spellings to choose between.
    #[test]
    fn no_tool_advertises_a_ref_alias() {
        for spec in tool_specs() {
            let schema = (spec.schema)();
            let properties = schema
                .get("properties")
                .and_then(Value::as_object)
                .expect("schema has properties");
            assert!(
                !properties.contains_key("ref"),
                "pilot.{} must take the element through `target`, not `ref`",
                spec.name
            );
        }
    }

    #[test]
    fn scroll_schema_advertises_target_only() {
        let schema = scroll_schema();
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("schema has properties");
        assert!(
            properties.contains_key("target"),
            "scroll schema must advertise `target`"
        );
    }

    /// `scroll` routes a bare snapshot id to a ref like every other tool.
    #[test]
    fn scroll_target_accepts_a_bare_snapshot_ref() {
        assert_eq!(
            build_scroll_params("down", Some(50), Some("e1")),
            json!({"ref": "e1", "direction": "down", "amount": 50})
        );
        assert_eq!(
            build_scroll_params("down", Some(50), Some("#log")),
            json!({"selector": "#log", "direction": "down", "amount": 50})
        );
    }

    /// The dropped `ref` argument fails instead of scrolling the page (#216).
    ///
    /// `additionalProperties: false` is only advertised, never enforced, so
    /// the handler itself has to reject the pre-#216 shape.
    #[tokio::test]
    async fn scroll_rejects_the_legacy_ref_argument() {
        let pilot = PilotMcpServer::new(Some(PathBuf::from("/nonexistent.sock")), None);
        let mut args = Map::new();
        args.insert("ref".to_owned(), json!("e12"));
        let err = pilot
            .call_tool_by_name("scroll", args)
            .await
            .expect_err("scroll must reject `ref`");
        assert!(
            err.to_string().contains("'target'"),
            "error must point callers at `target`, got: {err}"
        );
    }

    #[tokio::test]
    async fn run_rejects_path_and_content_together() {
        let mut args = Map::new();
        args.insert("path".to_owned(), json!("scenario.toml"));
        args.insert("content".to_owned(), json!("[scenario]\nname = \"x\""));
        let err = PilotMcpServer::new(None, None)
            .call_tool_by_name("run", args)
            .await
            .expect_err("both set");
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        assert!(
            err.message.contains("not both"),
            "unexpected error: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn run_rejects_missing_path_and_content() {
        let err = PilotMcpServer::new(None, None)
            .call_tool_by_name("run", Map::new())
            .await
            .expect_err("neither set");
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        assert!(
            err.message.contains("requires either"),
            "unexpected error: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn run_invalid_toml_content_is_tool_error() {
        let mut args = Map::new();
        args.insert("content".to_owned(), json!("[[[not toml"));
        let missing_socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-missing-parse-{}.sock",
            std::process::id()
        ));
        let result = PilotMcpServer::new(Some(missing_socket), None)
            .call_tool_by_name("run", args)
            .await
            .expect("tool call returns");
        assert_eq!(result.is_error, Some(true));
        let error = tool_error_text(&result);
        assert!(
            error.contains("Failed to parse scenario TOML"),
            "unexpected error: {error}"
        );
        assert!(
            error.contains("TOML parse error") || error.contains("line"),
            "cause chain missing from error: {error}"
        );
    }

    #[tokio::test]
    async fn run_unknown_top_level_key_is_tool_error() {
        let mut args = Map::new();
        args.insert("content".to_owned(), json!("[[steps]]\naction = \"click\""));
        let missing_socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-missing-unknown-{}.sock",
            std::process::id()
        ));
        let result = PilotMcpServer::new(Some(missing_socket), None)
            .call_tool_by_name("run", args)
            .await
            .expect("tool call returns");
        assert_eq!(result.is_error, Some(true));
        let error = tool_error_text(&result);
        assert!(
            error.contains("Failed to parse scenario TOML"),
            "unexpected error: {error}"
        );
        assert!(
            error.contains("unknown field") && error.contains("steps"),
            "cause chain missing from error: {error}"
        );
    }

    #[tokio::test]
    async fn run_missing_file_is_tool_error() {
        let path = std::env::temp_dir().join(format!(
            "tauri-pilot-missing-scenario-{}-{}.toml",
            std::process::id(),
            "run-missing"
        ));
        let _ = std::fs::remove_file(&path);
        let missing_socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-missing-load-{}.sock",
            std::process::id()
        ));
        let mut args = Map::new();
        args.insert("path".to_owned(), json!(path.display().to_string()));
        let result = PilotMcpServer::new(Some(missing_socket), None)
            .call_tool_by_name("run", args)
            .await
            .expect("tool call returns");
        assert_eq!(result.is_error, Some(true));
        let error = tool_error_text(&result);
        assert!(
            error.contains("Failed to read scenario file"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn run_connect_failure_is_tool_error() {
        let missing_socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-missing-connect-{}.sock",
            std::process::id()
        ));
        let mut args = Map::new();
        args.insert(
            "content".to_owned(),
            json!(
                r##"
[[step]]
action = "click"
target = "#btn"
"##
            ),
        );
        let result = PilotMcpServer::new(Some(missing_socket), None)
            .call_tool_by_name("run", args)
            .await
            .expect("tool call returns");
        assert_eq!(result.is_error, Some(true));
        let error = tool_error_text(&result);
        assert!(
            error.contains("Cannot connect to socket")
                || error.contains("Cannot connect to named pipe"),
            "unexpected error: {error}"
        );
    }

    /// A step key the action does not read fails before connecting (#243).
    #[tokio::test]
    async fn run_invalid_step_key_is_tool_error_before_connect() {
        let missing_socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-missing-keys-{}.sock",
            std::process::id()
        ));
        let mut args = Map::new();
        args.insert(
            "content".to_owned(),
            json!("[[step]]\naction = \"assert-exists\"\nselector = \"#x\"\n"),
        );
        let result = PilotMcpServer::new(Some(missing_socket), None)
            .call_tool_by_name("run", args)
            .await
            .expect("tool call returns");
        assert_eq!(result.is_error, Some(true));
        let error = tool_error_text(&result);
        assert!(
            error.contains("step 'assert-exists' does not accept 'selector'; use 'target'")
                && !error.contains("Cannot connect"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn run_rejects_empty_steps() {
        let err = PilotMcpServer::new(None, None)
            .call_tool_by_name("run", {
                let mut args = Map::new();
                args.insert("content".to_owned(), json!(""));
                args
            })
            .await
            .expect_err("empty scenario");
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        assert!(
            err.message.contains("at least one [[step]]"),
            "unexpected error: {}",
            err.message
        );
    }

    struct RestoreEnv {
        key: &'static str,
        previous: Option<String>,
    }

    impl RestoreEnv {
        fn unset(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            // SAFETY: callers use `#[serial]` around tests that mutate this env var.
            unsafe { std::env::remove_var(key) };
            Self { key, previous }
        }
    }

    impl Drop for RestoreEnv {
        fn drop(&mut self) {
            // SAFETY: paired with `unset`; restores the pre-test value even if the test panics.
            unsafe {
                match &self.previous {
                    Some(value) => std::env::set_var(self.key, value),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    #[tokio::test]
    #[serial]
    async fn run_rejects_eval_step_when_dangerous_tools_disabled() {
        let _guard = RestoreEnv::unset(ENABLE_DANGEROUS_MCP_TOOLS_ENV);

        let mut args = Map::new();
        args.insert(
            "content".to_owned(),
            json!(
                r#"
[[step]]
action = "eval"
script = "1+1"
"#
            ),
        );
        let err = PilotMcpServer::new(None, None)
            .call_tool_by_name("run", args)
            .await
            .expect_err("eval gated");
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        assert!(
            err.message.contains("eval") && err.message.contains("disabled by default"),
            "unexpected error: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn run_rejects_javascript_navigate_urls() {
        let contents = [
            "[[step]]\naction = \"navigate\"\nurl = \" javascript:alert(1)\"",
            "[[step]]\naction = \"navigate\"\nurl = \"JaVaScRiPt:alert(1)\"",
            "[[step]]\naction = \"navigate\"\nurl = \"java\\tscript:alert(1)\"",
        ];
        for content in contents {
            let mut args = Map::new();
            args.insert("content".to_owned(), json!(content));
            let err = PilotMcpServer::new(None, None)
                .call_tool_by_name("run", args)
                .await
                .expect_err(&format!("javascript URL should be rejected: {content}"));
            assert_eq!(err.code, ErrorCode::INVALID_PARAMS, "content: {content}");
            assert!(
                err.message.contains("does not allow javascript: URLs"),
                "unexpected error message for {content}: {}",
                err.message
            );
        }
    }

    #[tokio::test]
    async fn run_rejects_screenshot_step_with_path() {
        let mut args = Map::new();
        args.insert(
            "content".to_owned(),
            json!(
                r#"
[[step]]
action = "screenshot"
path = "/tmp/out.png"
"#
            ),
        );
        let err = PilotMcpServer::new(None, None)
            .call_tool_by_name("run", args)
            .await
            .expect_err("screenshot path gated");
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        assert!(
            err.message.contains("cannot set 'path'"),
            "unexpected error: {}",
            err.message
        );
    }

    #[test]
    fn pilot_screenshot_tool_advertises_path_only_contract() {
        let tool = cached_tools()
            .iter()
            .find(|t| t.name == "pilot.screenshot_native")
            .expect("pilot.screenshot_native tool registered");
        let props = tool
            .input_schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("schema has properties");
        for required in ["window_id", "output_path", "format"] {
            assert!(
                props.contains_key(required),
                "pilot.screenshot_native must advertise `{required}` in its schema"
            );
        }
        let required = tool
            .input_schema
            .get("required")
            .and_then(Value::as_array)
            .expect("required list present");
        let required: Vec<&str> = required.iter().filter_map(Value::as_str).collect();
        assert!(
            required.contains(&"window_id"),
            "pilot.screenshot_native must require `window_id`"
        );
        assert!(
            required.contains(&"output_path"),
            "pilot.screenshot_native must require `output_path`"
        );
        // The path-only contract forbids inline byte / base64 fields on either
        // surface — guard against a future revision silently growing one.
        for forbidden in ["bytes", "base64", "data"] {
            assert!(
                !props.contains_key(forbidden),
                "pilot.screenshot_native must not advertise `{forbidden}` (path-only contract)"
            );
        }
    }

    #[test]
    fn pilot_screenshot_tool_routes_native_method() {
        // The native contract lives under a distinct name (`screenshot_native`
        // advertised as `pilot.screenshot_native`) so the existing bridge
        // `screenshot` (html-to-image, base64) keeps working for current
        // CLI/scenario callers. This test pins the surface so a future
        // refactor cannot silently fold the two tools together and resurrect
        // the bytes-inline payload shape.
        let bridge = cached_tools()
            .iter()
            .find(|t| t.name == "pilot.screenshot")
            .expect("bridge pilot.screenshot tool still registered");
        let native = cached_tools()
            .iter()
            .find(|t| t.name == "pilot.screenshot_native")
            .expect("pilot.screenshot_native tool registered");
        assert_ne!(
            bridge.input_schema, native.input_schema,
            "bridge `pilot.screenshot` and native `pilot.screenshot_native` advertise different schemas"
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn pilot_screenshot_forwards_native_params_to_jsonrpc() {
        let socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-screenshot-native-{}.sock",
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
            assert_eq!(request.method, "screenshot_native");
            let params = request.params.expect("native screenshot params present");
            assert_eq!(params["window_id"], json!(42_u64));
            assert_eq!(params["output_path"], json!("/tmp/out.png"));
            assert_eq!(params["format"], json!("png"));
            let response = Response::success(
                request.id,
                json!({
                    "output_path": "/tmp/out.png",
                    "window_id": 42_u32,
                    "width": 100_u32,
                    "height": 50_u32,
                    "scale_factor": 2.0_f32,
                    "byte_size": 1234_u64,
                    "backend": "screencapture",
                    "tcc_denied": false,
                }),
            );
            let mut bytes = serde_json::to_vec(&response).expect("serialize response");
            bytes.push(b'\n');
            writer.write_all(&bytes).await.expect("write response");
        });

        let pilot = PilotMcpServer::new(Some(socket.clone()), None);
        let mut args = Map::new();
        args.insert("window_id".to_owned(), json!(42_u32));
        args.insert("output_path".to_owned(), json!("/tmp/out.png"));
        args.insert("format".to_owned(), json!("png"));
        let result = pilot
            .call_tool_by_name("screenshot_native", args)
            .await
            .expect("tool call succeeds");
        assert_eq!(result.is_error, Some(false));

        server.await.expect("mock server task");
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn drag_forwards_gesture_tunables_to_the_bridge() {
        // The docs advertise steps/stepDelayMs/settleMs over MCP; they only work
        // if the rebuilt params object carries them through to the bridge.
        let socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-drag-tunables-{}.sock",
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
            assert_eq!(request.method, "drag");
            let params = request.params.expect("drag params present");
            assert_eq!(params["steps"], json!(30));
            assert_eq!(params["stepDelayMs"], json!(40));
            assert_eq!(params["settleMs"], json!(1_500));
            let response = Response::success(request.id, json!({"ok": true}));
            let mut bytes = serde_json::to_vec(&response).expect("serialize response");
            bytes.push(b'\n');
            writer.write_all(&bytes).await.expect("write response");
        });

        let pilot = PilotMcpServer::new(Some(socket.clone()), None);
        let mut args = Map::new();
        args.insert("source".to_owned(), json!("@e5"));
        args.insert("target".to_owned(), json!("@e8"));
        args.insert("steps".to_owned(), json!(30));
        args.insert("stepDelayMs".to_owned(), json!(40));
        args.insert("settleMs".to_owned(), json!(1_500));
        let result = pilot
            .call_tool_by_name("drag", args)
            .await
            .expect("tool call succeeds");
        assert_eq!(result.is_error, Some(false));

        server.await.expect("mock server task");
        let _ = std::fs::remove_file(&socket);
    }

    #[test]
    fn drag_schema_declares_the_documented_tunables() {
        let schema = drag_schema();
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("schema has properties");
        for key in ["steps", "stepDelayMs", "settleMs"] {
            assert!(properties.contains_key(key), "drag schema is missing {key}");
        }
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

    #[test]
    fn dangerous_tools_hidden_by_default() {
        let tools = build_tools_with_flag(false);
        for dangerous in DANGEROUS_MCP_TOOLS {
            let namespaced = namespaced_tool_name(dangerous);
            assert!(
                !tools.iter().any(|tool| tool.name == namespaced),
                "dangerous tool '{dangerous}' must not be listed unless explicitly enabled"
            );
        }
    }

    #[test]
    fn dangerous_tools_can_be_enabled() {
        let tools = build_tools_with_flag(true);
        for dangerous in DANGEROUS_MCP_TOOLS {
            let namespaced = namespaced_tool_name(dangerous);
            assert!(
                tools.iter().any(|tool| tool.name == namespaced),
                "dangerous tool '{dangerous}' should be listed when explicitly enabled"
            );
        }
    }

    #[tokio::test]
    async fn navigate_rejects_javascript_urls() {
        // Each payload normalizes to `javascript:` once a browser's URL parser
        // strips tab/LF/CR and leading C0 controls/spaces, so the MCP filter
        // must reject them before they reach `window.location.href`.
        let payloads = [
            " javascript:alert(1)",          // leading space
            "JaVaScRiPt:alert(1)",           // mixed case
            "java\tscript:alert(1)",         // embedded tab
            "java\nscript:alert(1)",         // embedded newline
            "java\rscript:alert(1)",         // embedded carriage return
            "\u{0}javascript:alert(1)",      // leading NUL (C0 control)
            "\u{1}\u{2}javascript:alert(1)", // leading C0 controls
        ];

        for payload in payloads {
            let pilot = PilotMcpServer::new(None, None);
            let mut args = Map::new();
            args.insert("url".to_owned(), json!(payload));

            // Validation rejects before any socket call, so this surfaces an
            // `Err(McpError)` rather than reaching the app tool.
            let err = pilot
                .call_tool_by_name("navigate", args)
                .await
                .expect_err(&format!("javascript URL should be rejected: {payload:?}"));
            assert_eq!(err.code, ErrorCode::INVALID_PARAMS, "payload: {payload:?}");
            assert!(
                err.message.contains("does not allow javascript: URLs"),
                "unexpected error message for {payload:?}: {}",
                err.message
            );
        }
    }

    #[test]
    fn validate_navigate_url_allows_safe_urls() {
        // Legitimate schemes and paths that merely contain the substring must
        // not be over-blocked by the prefix check.
        for url in [
            "https://example.com/app",
            "/relative/path",
            "https://example.com/javascript:not-a-scheme",
            "about:blank",
        ] {
            validate_navigate_url(url).unwrap_or_else(|err| {
                panic!("safe url wrongly rejected: {url:?} ({})", err.message)
            });
        }
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
        assert!(script.contains("tauri-pilot click '@e1'"));

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

    /// `error.data` reaches the MCP client as fields, not as JSON printed
    /// inside the error text (#242).
    #[tokio::test]
    #[cfg(unix)]
    async fn rpc_error_data_stays_structured_in_the_tool_error() {
        let socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-rpc-error-test-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).expect("bind mock socket");
        let windows = json!([{"label": "main", "title": "Main", "url": "tauri://localhost"}]);
        let data = json!({
            "error": "WINDOW_NOT_FOUND",
            "message": "Window 'nope' not found",
            "available_windows": windows,
        });
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            reader.read_line(&mut line).await.expect("read request");
            let request: Request = serde_json::from_str(line.trim()).expect("parse request");
            let mut response =
                Response::error(json!(request.id), -32602, "Window 'nope' not found");
            response.error.as_mut().expect("error response").data = Some(data);
            let mut bytes = serde_json::to_vec(&response).expect("serialize response");
            bytes.push(b'\n');
            writer.write_all(&bytes).await.expect("write response");
        });

        let pilot = PilotMcpServer::new(Some(socket.clone()), None);
        let mut args = Map::new();
        args.insert("window".to_owned(), json!("nope"));
        let result = pilot
            .call_tool_by_name("state", args)
            .await
            .expect("tool call returns");
        assert_eq!(result.is_error, Some(true));
        assert_eq!(
            result.structured_content,
            Some(json!({
                "error": "WINDOW_NOT_FOUND",
                "message": "Window 'nope' not found",
                "rpc_code": -32602,
                "available_windows": windows,
            }))
        );

        server.await.expect("mock server task");
        let _ = std::fs::remove_file(&socket);
    }

    /// Without a string domain code, `error` carries the message; the RPC
    /// error is found under added context, and a distinct `data` value under
    /// a key `tool_error` sets moves to `data_<key>`.
    #[test]
    fn rpc_error_without_domain_code_uses_its_message() {
        let bare = anyhow::Error::from(RpcError {
            code: -32603,
            message: "boom".to_owned(),
            data: None,
        })
        .context("while clicking");
        assert_eq!(
            tool_error(&bare).structured_content,
            Some(json!({"error": "boom", "message": "boom", "rpc_code": -32603}))
        );

        let distinct = anyhow::Error::from(RpcError {
            code: -32000,
            message: "top".to_owned(),
            data: Some(json!({"message": "inner", "hint": 1, "rpc_code": 7})),
        });
        assert_eq!(
            tool_error(&distinct).structured_content,
            Some(json!({
                "error": "top",
                "message": "top",
                "rpc_code": -32000,
                "hint": 1,
                "data_message": "inner",
                "data_rpc_code": 7,
            }))
        );

        let top = |data| {
            tool_error(&anyhow::Error::from(RpcError {
                code: -32000,
                message: "top".to_owned(),
                data,
            }))
            .structured_content
        };
        assert_eq!(
            top(Some(json!({"error": null}))),
            Some(json!({"error": "top", "message": "top", "rpc_code": -32000}))
        );
        assert_eq!(
            top(Some(json!({"error": 5}))),
            Some(json!({"error": "top", "message": "top", "rpc_code": -32000, "data_error": 5}))
        );
        assert_eq!(
            top(Some(json!({"message": "inner", "data_message": "detail"}))),
            Some(json!({
                "error": "top",
                "message": "top",
                "rpc_code": -32000,
                "data_message": "detail",
                "data_data_message": "inner",
            }))
        );
        assert_eq!(
            top(Some(json!(["a", "b"]))),
            Some(json!({"error": "top", "message": "top", "rpc_code": -32000, "data": ["a", "b"]}))
        );
        assert_eq!(
            top(Some(Value::Null)),
            Some(json!({"error": "top", "message": "top", "rpc_code": -32000}))
        );

        assert_eq!(
            tool_error(&anyhow::anyhow!("boom")).structured_content,
            Some(json!({"error": "boom"}))
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn run_tool_executes_inline_click_scenario() {
        let socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-run-test-{}.sock",
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
            let mut response =
                serde_json::to_vec(&Response::success(request.id, json!({"ok": true})))
                    .expect("serialize response");
            response.push(b'\n');
            writer.write_all(&response).await.expect("write response");
        });

        let pilot = PilotMcpServer::new(Some(socket.clone()), None);
        let mut args = Map::new();
        args.insert(
            "content".to_owned(),
            json!(
                r##"
[scenario]
name = "mcp-run"
[[step]]
name = "click submit"
action = "click"
target = "#btn"
"##
            ),
        );
        let result = pilot
            .call_tool_by_name("run", args)
            .await
            .expect("tool call succeeds");
        assert_eq!(result.is_error, Some(false));
        let report = result
            .structured_content
            .as_ref()
            .and_then(|content| content.get("result"))
            .expect("structured result");
        assert_eq!(report["ok"], true);
        assert_eq!(report["name"], "mcp-run");
        assert_eq!(report["steps"].as_array().map(Vec::len), Some(1));
        assert_eq!(report["steps"][0]["status"], "passed");
        let summary = report["summary"].as_str().expect("summary");
        assert!(
            summary.contains("1 passed"),
            "unexpected summary: {summary}"
        );

        server.await.expect("mock server task");
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn run_tool_executes_path_click_scenario() {
        let socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-run-path-{}.sock",
            std::process::id()
        ));
        let scenario_path = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-run-path-{}.toml",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&socket);
        let _ = std::fs::remove_file(&scenario_path);
        std::fs::write(
            &scenario_path,
            r##"
[scenario]
name = "mcp-run-path"
[[step]]
name = "click submit"
action = "click"
target = "#btn"
"##,
        )
        .expect("write scenario");
        let listener = UnixListener::bind(&socket).expect("bind mock socket");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            reader.read_line(&mut line).await.expect("read request");
            let request: Request = serde_json::from_str(line.trim()).expect("parse request");
            assert_eq!(request.method, "click");
            let mut response =
                serde_json::to_vec(&Response::success(request.id, json!({"ok": true})))
                    .expect("serialize response");
            response.push(b'\n');
            writer.write_all(&response).await.expect("write response");
        });

        let pilot = PilotMcpServer::new(Some(socket.clone()), None);
        let mut args = Map::new();
        args.insert(
            "path".to_owned(),
            json!(scenario_path.display().to_string()),
        );
        let result = pilot
            .call_tool_by_name("run", args)
            .await
            .expect("tool call succeeds");
        assert_eq!(result.is_error, Some(false));
        let report = result
            .structured_content
            .as_ref()
            .and_then(|content| content.get("result"))
            .expect("structured result");
        assert_eq!(report["ok"], true);
        assert_eq!(report["name"], "mcp-run-path");
        assert_eq!(report["steps"].as_array().map(Vec::len), Some(1));
        assert_eq!(report["steps"][0]["status"], "passed");

        server.await.expect("mock server task");
        let _ = std::fs::remove_file(&socket);
        let _ = std::fs::remove_file(&scenario_path);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn run_global_timeout_is_tool_error() {
        let socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-run-timeout-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).expect("bind mock socket");
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.expect("accept");
            std::future::pending::<()>().await;
        });

        let pilot = PilotMcpServer::new(Some(socket.clone()), None);
        let mut args = Map::new();
        args.insert(
            "content".to_owned(),
            json!(
                r##"
[scenario]
name = "timeout"
global_timeout_ms = 200
[[step]]
action = "click"
target = "#btn"
"##
            ),
        );
        let result = pilot
            .call_tool_by_name("run", args)
            .await
            .expect("tool call returns");
        assert_eq!(result.is_error, Some(true));
        let error = tool_error_text(&result);
        assert!(
            error.contains("scenario exceeded global timeout"),
            "unexpected error: {error}"
        );
        server.abort();
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn run_honors_connect_socket_when_server_has_none() {
        let socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-run-connect-toml-{}.sock",
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
            let mut response =
                serde_json::to_vec(&Response::success(request.id, json!({"ok": true})))
                    .expect("serialize response");
            response.push(b'\n');
            writer.write_all(&response).await.expect("write response");
        });

        let content = format!(
            r##"
[connect]
socket = "{}"
[[step]]
action = "click"
target = "#btn"
"##,
            socket.display()
        );
        let mut args = Map::new();
        args.insert("content".to_owned(), json!(content));
        let result = PilotMcpServer::new(None, None)
            .call_tool_by_name("run", args)
            .await
            .expect("tool call succeeds");
        assert_eq!(result.is_error, Some(false));
        let report = result
            .structured_content
            .as_ref()
            .and_then(|content| content.get("result"))
            .expect("structured result");
        assert_eq!(report["ok"], true);
        assert_eq!(report["steps"][0]["status"], "passed");

        server.await.expect("mock server task");
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn run_keeps_server_socket_when_connect_socket_is_set() {
        let socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-run-connect-server-{}.sock",
            std::process::id()
        ));
        let missing = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-run-connect-ignored-{}.sock",
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
            let mut response =
                serde_json::to_vec(&Response::success(request.id, json!({"ok": true})))
                    .expect("serialize response");
            response.push(b'\n');
            writer.write_all(&response).await.expect("write response");
        });

        let content = format!(
            r##"
[connect]
socket = "{}"
[[step]]
action = "click"
target = "#btn"
"##,
            missing.display()
        );
        let mut args = Map::new();
        args.insert("content".to_owned(), json!(content));
        let result = PilotMcpServer::new(Some(socket.clone()), None)
            .call_tool_by_name("run", args)
            .await
            .expect("tool call succeeds");
        assert_eq!(result.is_error, Some(false));
        let report = result
            .structured_content
            .as_ref()
            .and_then(|content| content.get("result"))
            .expect("structured result");
        assert_eq!(report["ok"], true);

        server.await.expect("mock server task");
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn run_tool_fail_fast_skips_remaining_steps() {
        let socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-run-failfast-{}.sock",
            std::process::id()
        ));
        let methods = spawn_failing_click_server(&socket);
        let shots = tempfile::tempdir().expect("temp dir");
        let report = call_run_two_clicks(&socket, None, Some(shots.path())).await;
        assert_eq!(report["ok"], false);
        assert_eq!(report["steps"][0]["status"], "failed");
        assert_eq!(report["steps"][1]["status"], "skipped");
        let recorded = methods.lock().expect("methods lock");
        assert_eq!(
            recorded.iter().filter(|method| *method == "click").count(),
            1
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn run_tool_fail_fast_false_runs_remaining_steps() {
        let socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-run-nofailfast-{}.sock",
            std::process::id()
        ));
        let methods = spawn_failing_click_server(&socket);
        let shots = tempfile::tempdir().expect("temp dir");
        let report = call_run_two_clicks(&socket, Some(false), Some(shots.path())).await;
        assert_eq!(report["ok"], false);
        assert_eq!(report["passed"], 1);
        assert_eq!(report["failed"], 1);
        assert_eq!(report["skipped"], 0);
        assert_eq!(report["steps"][0]["status"], "failed");
        assert_eq!(report["steps"][1]["status"], "passed");
        let recorded = methods.lock().expect("methods lock");
        assert_eq!(
            recorded.iter().filter(|method| *method == "click").count(),
            2
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn run_tool_reports_failure_screenshot_path() {
        let socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-run-shot-{}.sock",
            std::process::id()
        ));
        let _methods = spawn_failing_click_server(&socket);
        let shots = tempfile::tempdir().expect("temp dir");
        let report = call_run_two_clicks(&socket, None, Some(shots.path())).await;
        let saved = report["steps"][0]["screenshot"]
            .as_str()
            .expect("failed step reports a screenshot path");
        assert!(
            Path::new(saved).starts_with(shots.path()),
            "screenshot {saved} is not under the requested directory"
        );
        assert!(Path::new(saved).is_file(), "screenshot {saved} is missing");
        assert!(report["steps"][0].get("screenshot_error").is_none());
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn run_tool_reports_why_screenshot_was_not_saved() {
        let socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-run-noshot-{}.sock",
            std::process::id()
        ));
        let _methods = spawn_failing_click_server(&socket);
        let dir = tempfile::tempdir().expect("temp dir");
        // A regular file where the directory should be: create_dir_all fails.
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, b"not a directory").expect("write blocker");
        let report = call_run_two_clicks(&socket, None, Some(blocked.as_path())).await;
        let reason = report["steps"][0]["screenshot_error"]
            .as_str()
            .expect("failed step reports why the screenshot is missing");
        assert!(
            reason.contains("failed to save screenshot to"),
            "unexpected reason: {reason}"
        );
        assert!(report["steps"][0].get("screenshot").is_none());
    }

    /// Calls `pilot.run` on a two-click scenario, omitting `screenshots_dir`
    /// when it is `None` so the server default is exercised too.
    #[cfg(unix)]
    async fn call_run_two_clicks(
        socket: &Path,
        fail_fast: Option<bool>,
        screenshots_dir: Option<&Path>,
    ) -> Value {
        let pilot = PilotMcpServer::new(Some(socket.to_path_buf()), None);
        let mut args = Map::new();
        args.insert(
            "content".to_owned(),
            json!(
                r##"
[scenario]
name = "mcp-run-fail-fast"
[[step]]
name = "first"
action = "click"
target = "#btn"
[[step]]
name = "second"
action = "click"
target = "#btn"
"##
            ),
        );
        if let Some(fail_fast) = fail_fast {
            args.insert("fail_fast".to_owned(), json!(fail_fast));
        }
        if let Some(dir) = screenshots_dir {
            args.insert(
                "screenshots_dir".to_owned(),
                json!(dir.display().to_string()),
            );
        }
        let result = pilot
            .call_tool_by_name("run", args)
            .await
            .expect("tool call succeeds");
        assert_eq!(result.is_error, Some(false));
        let _ = std::fs::remove_file(socket);
        result
            .structured_content
            .as_ref()
            .and_then(|content| content.get("result"))
            .cloned()
            .expect("structured result")
    }

    /// Mock whose first `click` fails and whose `screenshot` succeeds.
    #[cfg(unix)]
    fn spawn_failing_click_server(socket: &Path) -> Arc<std::sync::Mutex<Vec<String>>> {
        spawn_failing_click_server_with(socket, ScreenshotMock::Png)
    }

    /// How the mock answers the failure-capture `screenshot` call.
    #[cfg(unix)]
    #[derive(Copy, Clone)]
    enum ScreenshotMock {
        Png,
        RpcError,
    }

    #[cfg(unix)]
    fn spawn_failing_click_server_with(
        socket: &Path,
        screenshot: ScreenshotMock,
    ) -> Arc<std::sync::Mutex<Vec<String>>> {
        let _ = std::fs::remove_file(socket);
        let listener = UnixListener::bind(socket).expect("bind mock socket");
        let methods = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = Arc::clone(&methods);
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            let mut click_count = 0_u8;
            while reader.read_line(&mut line).await.expect("read line") > 0 {
                let request: Request = serde_json::from_str(line.trim()).expect("parse request");
                recorded
                    .lock()
                    .expect("methods lock")
                    .push(request.method.clone());
                let resp = match request.method.as_str() {
                    "click" => {
                        click_count += 1;
                        if click_count == 1 {
                            Response::error(
                                serde_json::Value::Number(request.id.into()),
                                -32_000,
                                "click failed",
                            )
                        } else {
                            Response::success(request.id, json!({"ok": true}))
                        }
                    }
                    "screenshot" => match screenshot {
                        // 1x1 transparent PNG, enough for the save path to run.
                        ScreenshotMock::Png => Response::success(
                            request.id,
                            json!(
                                "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
                            ),
                        ),
                        ScreenshotMock::RpcError => Response::error(
                            serde_json::Value::Number(request.id.into()),
                            -32_000,
                            "window is gone",
                        ),
                    },
                    _ => Response::error(
                        serde_json::Value::Number(request.id.into()),
                        -32_601,
                        "Method not found",
                    ),
                };
                let mut bytes = serde_json::to_vec(&resp).expect("serialize response");
                bytes.push(b'\n');
                writer.write_all(&bytes).await.expect("write bytes");
                writer.flush().await.expect("flush");
                line.clear();
            }
        });
        methods
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn run_tool_reports_the_rpc_error_that_blocked_the_screenshot() {
        let socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-run-shot-rpc-{}.sock",
            std::process::id()
        ));
        let _methods = spawn_failing_click_server_with(&socket, ScreenshotMock::RpcError);
        let shots = tempfile::tempdir().expect("temp dir");
        let report = call_run_two_clicks(&socket, None, Some(shots.path())).await;
        let reason = report["steps"][0]["screenshot_error"]
            .as_str()
            .expect("failed step reports why the screenshot is missing");
        assert!(
            reason.contains("window is gone"),
            "unexpected reason: {reason}"
        );
        assert!(report["steps"][0].get("screenshot").is_none());
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn run_tool_without_screenshots_dir_uses_the_private_default() {
        let socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-run-shot-default-{}.sock",
            std::process::id()
        ));
        let _methods = spawn_failing_click_server(&socket);
        let report = call_run_two_clicks(&socket, None, None).await;
        let saved = report["steps"][0]["screenshot"]
            .as_str()
            .expect("failed step reports a screenshot path");
        let expected = default_screenshots_dir();
        assert!(
            Path::new(saved).starts_with(&expected),
            "screenshot {saved} is not under the default {}",
            expected.display()
        );
        assert!(Path::new(saved).is_file(), "screenshot {saved} is missing");
        assert!(
            is_private_dir(&expected),
            "default screenshot directory {} is not owner-only",
            expected.display()
        );
        // The default outlives the test, so only the PNG goes.
        let _ = std::fs::remove_file(saved);
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

    fn tool_error_text(result: &CallToolResult) -> &str {
        result
            .structured_content
            .as_ref()
            .and_then(|content| content.get("error"))
            .and_then(Value::as_str)
            .expect("error payload")
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
            let mut remaining = requests;
            while remaining > 0 {
                let (stream, _) = listener.accept().await.expect("accept");
                let (reader, mut writer) = stream.into_split();
                let mut reader = BufReader::new(reader);
                let mut line = String::new();
                let n = reader.read_line(&mut line).await.expect("read request");
                // Auto-detect probes with a connect/drop (#194); ignore empty peers.
                if n == 0 || line.trim().is_empty() {
                    continue;
                }
                let request: Request = serde_json::from_str(line.trim()).expect("parse request");
                assert_eq!(request.method, "click");
                let mut response =
                    serde_json::to_vec(&Response::success(request.id, json!({"source": source})))
                        .expect("serialize response");
                response.push(b'\n');
                writer.write_all(&response).await.expect("write response");
                remaining -= 1;
            }
        })
    }
}
