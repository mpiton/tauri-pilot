//! `PilotMcpServer` file-based tool helpers: `call_drop_tool`, `call_replay_tool`.

use std::path::PathBuf;

use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject},
};

use super::super::args::{optional_string, required_string, required_string_array};
use super::super::responses::{invalid_params, tool_error, tool_success};
use super::PilotMcpServer;
use crate::{export_replay_file, run_drop_command, run_replay_command};

impl PilotMcpServer {
    pub(crate) async fn call_drop_tool(
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

    pub(crate) async fn call_replay_tool(
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
}
