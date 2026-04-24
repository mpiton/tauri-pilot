//! `PilotMcpServer` struct, server lifecycle, and `ServerHandler` impl.
//!
//! Helper methods on the server (assert_*, call_*_tool, `target_call`, `window_arg`)
//! live in `methods.rs` to keep this file under 150 lines.

mod methods;

use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use anyhow::Result;
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResult, Implementation, JsonObject, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
    },
    service::{MaybeSendFuture, RequestContext, RoleServer},
    transport::stdio,
};

use super::args::optional_string;
use super::banner::print_startup_banner;
use crate::client::Client;
use crate::mcp::_legacy;
use crate::resolve_socket;

/// MCP server that bridges Claude/LLM tool calls to a live Tauri app via Unix socket.
#[derive(Debug, Clone)]
pub(crate) struct PilotMcpServer {
    pub(super) socket: Option<PathBuf>,
    pub(super) window: Option<String>,
    pub(super) resolved_socket: Arc<OnceLock<PathBuf>>,
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

impl PilotMcpServer {
    pub(crate) fn new(socket: Option<PathBuf>, window: Option<String>) -> Self {
        Self {
            socket,
            window,
            resolved_socket: Arc::new(OnceLock::new()),
        }
    }

    pub(crate) async fn connect_client(&self) -> Result<Client> {
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

    pub(crate) async fn call_app(
        &self,
        method: &'static str,
        params: Option<serde_json::Value>,
        window: Option<String>,
    ) -> Result<serde_json::Value> {
        use crate::with_window;
        let mut client = self.connect_client().await?;
        client
            .call(method, with_window(params, window.as_deref()))
            .await
    }

    pub(crate) async fn call_app_tool(
        &self,
        method: &'static str,
        params: Option<serde_json::Value>,
        window: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        use super::responses::{tool_error, tool_success};
        Ok(match self.call_app(method, params, window).await {
            Ok(result) => tool_success(result),
            Err(err) => tool_error(err),
        })
    }

    pub(crate) fn window_arg(&self, args: &JsonObject) -> Result<Option<String>, McpError> {
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
        std::future::ready(Ok(ListToolsResult::with_all_items(_legacy::tools())))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, McpError>> + MaybeSendFuture + '_ {
        let name = request.name.to_string();
        let args = request.arguments.unwrap_or_default();
        async move { super::handlers::call_tool_by_name(self, &name, args).await }
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        _legacy::cached_tools()
            .iter()
            .find(|tool| tool.name == name)
            .cloned()
    }
}
