//! Handlers for the `assert` tool group: `assert_text`, `assert_contains`,
//! `assert_visible`, `assert_hidden`, `assert_value`, `assert_count`, `assert_checked`,
//! `assert_url`.

use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject},
};

use super::super::server::PilotMcpServer;

pub(super) async fn dispatch(
    server: &PilotMcpServer,
    name: &str,
    args: &JsonObject,
    window: Option<String>,
) -> Result<CallToolResult, McpError> {
    match name {
        "assert_text" => server.assert_text(args.clone(), window, false).await,
        "assert_contains" => server.assert_text(args.clone(), window, true).await,
        "assert_visible" => {
            server
                .assert_bool("visible", args.clone(), window, true)
                .await
        }
        "assert_hidden" => {
            server
                .assert_bool("visible", args.clone(), window, false)
                .await
        }
        "assert_value" => server.assert_value(args.clone(), window).await,
        "assert_count" => server.assert_count(args.clone(), window).await,
        "assert_checked" => {
            server
                .assert_bool("checked", args.clone(), window, true)
                .await
        }
        "assert_url" => server.assert_url(args.clone(), window).await,
        _ => unreachable!("handlers/mod.rs guarantees prefix match: {name}"),
    }
}
