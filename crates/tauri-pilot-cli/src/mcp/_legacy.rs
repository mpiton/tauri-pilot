//! TEMPORARY — shrinks step-by-step toward zero (PR1 Tasks 5-8).
//!
//! Contains: `tools()` registry, schema builders.
//! Removed in Task 5: `PilotMcpServer` struct, `run_mcp_server`, impl blocks.
//! Remaining for Task 6: split `tools()` per domain.
//! Remaining for Task 8: delete this file.

use std::sync::{Arc, OnceLock};

use rmcp::model::{JsonObject, Tool, ToolAnnotations};
use serde_json::{Map, Value, json};

use super::schemas::{
    any_prop, array_string_prop, bool_prop, enum_prop, integer_prop, object_schema, props,
    string_prop,
};

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

pub(super) fn tools() -> Vec<Tool> {
    cached_tools().clone()
}

pub(super) fn cached_tools() -> &'static Vec<Tool> {
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

#[cfg(test)]
mod tests {
    use super::super::banner::startup_banner;
    use super::super::server::PilotMcpServer;
    use super::*;
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
