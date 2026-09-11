pub mod diff;
mod error;
#[cfg(any(unix, windows))]
pub(crate) mod eval;
#[cfg(any(unix, windows))]
mod handler;
#[cfg(feature = "press")]
pub(crate) mod key;
pub(crate) mod protocol;
pub(crate) mod recorder;
// Native screenshot capture for the `screenshot_native` JSON-RPC method.
// macOS-only today; non-macOS callers receive `PERMISSION_DENIED`.
pub(crate) mod screenshot;
#[cfg(any(unix, windows))]
pub(crate) mod server;
#[cfg(any(unix, windows))]
pub(crate) mod webview;

pub use error::Error;

#[cfg(any(unix, windows))]
use eval::EvalEngine;
#[cfg(any(unix, windows))]
use recorder::Recorder;
#[cfg(any(unix, windows))]
use std::sync::Arc;
#[cfg(any(unix, windows))]
use tauri::Manager;

#[cfg(all(any(unix, windows), debug_assertions))]
pub(crate) const BRIDGE_JS: &str = concat!(
    include_str!("../js/vendor/html-to-image.iife.js"),
    "\n",
    include_str!("../js/bridge.js"),
);

/// Initialize the tauri-pilot plugin.
///
/// On non-Unix, non-Windows platforms or in release builds, returns a no-op plugin.
/// In debug builds on Unix other than Android, injects the JS bridge, stores an `EvalEngine`,
/// and starts a Unix socket server at `$XDG_RUNTIME_DIR/tauri-pilot-{identifier}.sock` (falls back to `/tmp` if unavailable).
/// Android uses `tauri-pilot-{identifier}-{random}.sock` in the abstract namespace,
/// reachable via ADB forwarding. The per-instance address is logged at info level.
/// In debug builds on Windows, starts a Named Pipe server at
/// `\\.\pipe\tauri-pilot-{identifier}` and registers the instance under `%LOCALAPPDATA%\tauri-pilot\instances\`.
///
/// Failing to start the server never prevents the host app from running: if the
/// socket cannot be bound (for example because another instance of the same app
/// already owns it), the failure is logged at warn level and the app starts
/// without a pilot server.
#[must_use]
pub fn init<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    #[cfg(not(all(any(unix, windows), debug_assertions)))]
    {
        return tauri::plugin::Builder::new("pilot").build();
    }

    #[cfg(all(any(unix, windows), debug_assertions))]
    {
        tauri::plugin::Builder::new("pilot")
            .js_init_script(BRIDGE_JS.to_owned())
            .on_webview_ready(|webview| {
                if let Err(err) = webview.eval(BRIDGE_JS) {
                    tracing::warn!(error = %err, "failed to inject tauri-pilot bridge on webview ready");
                }
            })
            .setup(|app, _api| {
                let engine = EvalEngine::new();
                app.manage(engine.clone());

                let identifier = sanitize_identifier(&app.config().identifier);

                let webviews: Arc<dyn webview::Webviews> =
                    Arc::new(webview::TauriWebviews(app.clone()));

                let recorder = Recorder::new();

                // Unix binds with the std (sync) `UnixListener`, which needs no
                // tokio runtime, so binding stays here in `setup` and the socket
                // is ready before the app finishes starting. `run` only upgrades
                // the listener to tokio once it is already on the runtime.
                //
                // The plugin is debug-only tooling: failing to start the QA
                // server (e.g. a second instance of the app already owns the
                // socket, #152) must never prevent the host app from running,
                // so bind errors are logged and skipped, never returned from
                // `setup` (where they become a fatal `PluginInitialization`).
                #[cfg(unix)]
                {
                    let Ok(address) = server::socket_address(&identifier).inspect_err(|e| {
                        tracing::warn!(identifier, "tauri-pilot server not started, failed to build socket address: {e}");
                    }) else {
                        return Ok(());
                    };
                    let Ok((listener, guard)) = server::bind(&address).inspect_err(|e| {
                        tracing::warn!(?address, "tauri-pilot server not started, failed to bind socket: {e}");
                    }) else {
                        return Ok(());
                    };
                    tauri::async_runtime::spawn(server::run(
                        listener,
                        guard,
                        engine,
                        webviews,
                        recorder,
                    ));
                }

                // Windows' tokio `NamedPipeServer` registers with the reactor the
                // instant it is created, so the bind must run inside the spawned
                // task (which lives on the tokio runtime). Binding here in `setup`
                // panics with "there is no reactor running, must be called from
                // the context of a Tokio 1.x runtime" (#115).
                #[cfg(windows)]
                tauri::async_runtime::spawn(server::run(
                    server::socket_path(&identifier),
                    engine,
                    webviews,
                    recorder,
                ));

                Ok(())
            })
            .invoke_handler(tauri::generate_handler![
                handler::callback,
                handler::__callback
            ])
            .build()
    }
}

/// Strip path separators and unsafe characters from the app identifier
/// so it can be safely used in a socket filename.
#[cfg(all(any(unix, windows), debug_assertions))]
fn sanitize_identifier(raw: &str) -> String {
    let sanitized: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "default".to_owned()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    // Bound each function body by the start of the next `function ` declaration
    // (or end-of-string), so the slice is immune to brace indentation changes
    // and to nested blocks closing with the same brace pattern.
    // ASCII needles → `find()` returns offsets that are valid UTF-8 char boundaries.
    #[cfg(all(any(unix, windows), debug_assertions))]
    fn bridge_fn_body<'a>(js: &'a str, fn_decl: &str) -> &'a str {
        let start = js
            .find(fn_decl)
            .unwrap_or_else(|| panic!("{fn_decl} missing"));
        let after = start + fn_decl.len();
        let end = js[after..]
            .find("\n  function ")
            .map_or(js.len(), |off| after + off);
        &js[start..end]
    }

    #[cfg(all(any(unix, windows), debug_assertions))]
    #[test]
    fn bridge_js_contains_html_to_image_and_pilot() {
        let js = super::BRIDGE_JS;
        assert!(
            js.contains("htmlToImage"),
            "BRIDGE_JS must include the html-to-image IIFE bundle"
        );
        assert!(
            js.contains("window.__PILOT__"),
            "BRIDGE_JS must include the pilot bridge"
        );
        let html_idx = js.find("htmlToImage").expect("htmlToImage missing");
        let pilot_idx = js
            .find("window.__PILOT__")
            .expect("window.__PILOT__ missing");
        assert!(
            html_idx < pilot_idx,
            "html-to-image must be injected before pilot bridge code"
        );
    }

    #[cfg(all(any(unix, windows), debug_assertions))]
    #[test]
    fn bridge_says_hello_with_reserved_callback_id() {
        // #153: the plugin learns which origins can call back from this hello.
        let hello = format!(
            r#"invoke("plugin:pilot|__callback", {{ id: {}, result: location.href }})"#,
            crate::eval::HELLO_ID
        );
        assert!(
            super::BRIDGE_JS.contains(&hello),
            "bridge must send its hello as `{hello}`"
        );
    }

    #[cfg(all(any(unix, windows), debug_assertions))]
    #[test]
    fn bridge_click_dispatches_pointer_sequence() {
        let js = super::BRIDGE_JS;
        let js_normalized: String = js.lines().collect::<Vec<_>>().join("\n");
        let pointer_down_idx = js
            .find(r#"dispatchPointerEvent(el, "pointerdown""#)
            .expect("click must dispatch pointerdown for Radix triggers");
        let mouse_down_idx = js
            .find(r#"MouseEvent("mousedown""#)
            .expect("click must keep mousedown compatibility");
        let pointer_up_idx = js
            .find(r#"dispatchPointerEvent(el, "pointerup""#)
            .expect("click must dispatch pointerup for Radix triggers");
        let mouse_up_idx = js
            .find(r#"MouseEvent("mouseup""#)
            .expect("click must keep mouseup compatibility");
        let click_idx = js
            .find(r#"dispatchPointerEvent(el, "click""#)
            .expect("click must dispatch as a pointer event");

        assert!(
            pointer_down_idx < mouse_down_idx
                && mouse_down_idx < pointer_up_idx
                && pointer_up_idx < mouse_up_idx
                && mouse_up_idx < click_idx,
            "click must dispatch pointerdown -> mousedown -> pointerup -> mouseup -> click"
        );
        assert!(
            js.contains(r#"pointerType: "mouse""#),
            "pointer events must include mouse pointer metadata"
        );
        assert!(
            js_normalized.contains(
                "if (pointerDownOk) {\n      const mouseDownOk = el.dispatchEvent(new MouseEvent(\"mousedown\""
            ),
            "mousedown must only dispatch when pointerdown was not canceled"
        );
        assert!(
            js_normalized.contains(
                "if (pointerDownOk) {\n      el.dispatchEvent(new MouseEvent(\"mouseup\""
            ),
            "mouseup must only dispatch when pointerdown was not canceled"
        );
    }

    #[cfg(all(any(unix, windows), debug_assertions))]
    #[test]
    fn bridge_scroll_handles_top_and_bottom_directions() {
        let js = super::BRIDGE_JS;
        assert!(
            js.contains(r#"if (dir === "top")"#),
            "scroll must handle direction \"top\""
        );
        assert!(
            js.contains(r#"if (dir === "bottom")"#),
            "scroll must handle direction \"bottom\""
        );
        assert!(
            js.contains("target.scrollTo(window.scrollX, 0)"),
            "scroll top on window must preserve window.scrollX and set Y=0"
        );
        assert!(
            js.contains("target.scrollTo(window.scrollX, Math.max(0, max))"),
            "scroll bottom on window must preserve window.scrollX and clamp negative max"
        );
        assert!(
            js.contains("Math.max(")
                && js.contains("docEl ? docEl.scrollHeight : 0")
                && js.contains("body ? body.scrollHeight : 0"),
            "scroll bottom on window must use Math.max(documentElement.scrollHeight, body.scrollHeight) for quirks-mode safety"
        );
        assert!(
            js.contains("docEl ? docEl.clientHeight : window.innerHeight"),
            "scroll bottom on window must subtract docEl.clientHeight (excludes horizontal scrollbar) instead of window.innerHeight"
        );
        assert!(
            js.contains("String(dir).slice(0, 64)"),
            "scroll error message must cap user-supplied direction length"
        );
        assert!(
            js.contains("target.scrollTop = 0"),
            "scroll top on element must set scrollTop = 0"
        );
        assert!(
            js.contains(
                "target.scrollTop = Math.max(0, target.scrollHeight - target.clientHeight)"
            ),
            "scroll bottom on element must use scrollHeight - clientHeight (not raw scrollHeight)"
        );
        assert!(
            js.contains("Unknown scroll direction:"),
            "scroll must throw on unknown direction instead of silently no-op"
        );
    }

    #[cfg(all(any(unix, windows), debug_assertions))]
    #[test]
    fn bridge_eval_auto_wraps_top_level_await() {
        // #79: top-level `await` in user scripts must compile via the
        // async-IIFE fallback stages instead of crashing with an opaque
        // SyntaxError from indirect eval.
        let js = super::BRIDGE_JS;
        assert!(
            js.contains("function evalScript("),
            "BRIDGE_JS must define evalScript"
        );
        assert!(
            js.contains("(async () => (\\n\" + script + \"\\n))()"),
            "evalScript must include the async-expression compile stage (#79)"
        );
        assert!(
            js.contains("hasTopLevelAwait(script)"),
            "evalScript must guard the async fallbacks with hasTopLevelAwait (#79)"
        );
        assert!(
            js.contains("(async () => {\\n\" + script + \"\\n})()"),
            "evalScript must include the async-statement IIFE fallback (#79)"
        );
        assert!(
            js.contains("function hasTopLevelAwait("),
            "BRIDGE_JS must define the hasTopLevelAwait helper (#79)"
        );
        assert!(
            js.contains("top-level await detected but the script could not be auto-wrapped"),
            "evalScript must surface a clear error when auto-wrap fails (#79)"
        );

        // Stage ordering: expression compile must precede the async fallbacks,
        // and the async-expression stage must precede the indirect-eval path.
        // Needles are formatting-stable substrings of the JS source, so a
        // future `prettier`/`rustfmt` reflow of `bridge.js` does not silently
        // break the ordering check.
        let evalscript_idx = js.find("function evalScript(").expect("evalScript missing");
        // SAFETY: the needle is ASCII, so `find()` returns a UTF-8 char boundary.
        let body = &js[evalscript_idx..];
        let expr_idx = body
            .find("\"return (\\n\" + script + \"\\n)\"")
            .expect("stage 1 expression compile missing");
        let async_expr_idx = body
            .find("\"return (async () => (\\n\" + script + \"\\n))()\"")
            .expect("stage 2 async-expression compile missing");
        let async_stmt_idx = body
            .find("\"return (async () => {\\n\" + script + \"\\n})()\"")
            .expect("stage 3 async-statement IIFE missing");
        let indirect_idx = body
            .find("var indirectEval = eval;")
            .expect("indirect eval fallback missing");
        assert!(
            expr_idx < async_expr_idx,
            "expression compile must precede async-expression fallback"
        );
        assert!(
            async_expr_idx < async_stmt_idx,
            "async-expression must precede async-statement fallback"
        );
        assert!(
            async_stmt_idx < indirect_idx,
            "async-statement IIFE must precede plain indirect eval (await guard runs first)"
        );
    }

    #[cfg(all(any(unix, windows), debug_assertions))]
    #[test]
    fn bridge_native_value_setter_picks_prototype_per_element() {
        // #85: `fill` and `type` on a <textarea> threw
        // "The HTMLInputElement.value setter can only be used on instances of HTMLInputElement"
        // because the old code used
        //   Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")
        //   || Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value");
        // The first descriptor is always truthy, so the textarea branch was unreachable
        // and the input setter was applied to a textarea, violating the WebIDL brand check.
        let js = super::BRIDGE_JS;

        assert!(
            js.contains("function nativeValueSetter("),
            "BRIDGE_JS must define a nativeValueSetter helper that picks the prototype based on the element (#85)"
        );

        // The helper must use the element's actual prototype to support input,
        // textarea, and select uniformly without violating the brand check.
        assert!(
            js.contains("Object.getPrototypeOf(el)"),
            "nativeValueSetter must derive the prototype from the element instance (#85)"
        );

        // Buggy short-circuit must be gone from fill/typeText.
        let buggy_pattern =
            "Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, \"value\") ||";
        assert!(
            !js.contains(buggy_pattern),
            "fill/typeText must not use the `HTMLInputElement.prototype || HTMLTextAreaElement.prototype` short-circuit (#85)"
        );

        let fill_body = bridge_fn_body(js, "function fill(params)");
        let type_body = bridge_fn_body(js, "function typeText(params)");
        let select_body = bridge_fn_body(js, "function select(params)");

        assert!(
            fill_body.contains("nativeValueSetter("),
            "fill must call nativeValueSetter (#85)"
        );
        assert!(
            type_body.contains("nativeValueSetter("),
            "typeText must call nativeValueSetter (#85)"
        );
        assert!(
            select_body.contains("applySelectOption("),
            "select must apply a matched option (#85)"
        );
        assert!(
            bridge_fn_body(js, "function applySelectOption(").contains("nativeValueSetter("),
            "applySelectOption must call nativeValueSetter (#85) so a future textarea-style brand-check bug cannot reappear in any setter handler"
        );

        // The pre-refactor `select` relied on the WebIDL brand check to reject
        // non-<select> targets implicitly. The helper drops that guarantee, so
        // `select` must keep an explicit guard to fail fast on misrouted
        // selectors instead of silently writing `value` on an unrelated
        // element. The guard must be realm-safe (tag-based, not `instanceof`),
        // because `nativeValueSetter` was added specifically to support
        // elements coming from another window/iframe realm.
        assert!(
            select_body.contains("select requires a <select> element"),
            "select must explicitly reject non-<select> targets after the nativeValueSetter refactor (#85)"
        );
        assert!(
            !select_body.contains("instanceof HTMLSelectElement"),
            "select guard must be realm-safe — `instanceof HTMLSelectElement` rejects valid <select> elements from another realm, which contradicts the cross-realm support that motivated nativeValueSetter (#85)"
        );

        // Helper must be defined before its callers (hoisting works for `function`
        // declarations, but ordering keeps the source readable for reviewers).
        let fill_idx = js
            .find("function fill(params)")
            .expect("fill function missing");
        let helper_idx = js
            .find("function nativeValueSetter(")
            .expect("nativeValueSetter helper missing");
        assert!(
            helper_idx < fill_idx,
            "nativeValueSetter must be declared before fill (#85)"
        );
    }

    #[cfg(all(any(unix, windows), debug_assertions))]
    #[test]
    fn bridge_fill_type_check_reject_non_editable_targets() {
        // #154: fill/type/check used to assign `.value` / `.checked` on any
        // element. On a <div> that creates an expando and reports ok while
        // nothing visible changed. Same failure class as the select guard
        // after nativeValueSetter: a reported ok must mean the action landed.
        let js = super::BRIDGE_JS;

        let fill_body = bridge_fn_body(js, "function fill(params)");
        let type_body = bridge_fn_body(js, "function typeText(params)");
        let check_body = bridge_fn_body(js, "function check(params)");
        let editable_body = bridge_fn_body(js, "function requireEditable(");
        let checkable_body = bridge_fn_body(js, "function requireCheckable(");

        assert!(
            fill_body.contains("requireEditable(el, \"fill\")"),
            "fill must reject non-editable targets before writing (#154)"
        );
        assert!(
            type_body.contains("requireEditable(el, \"type\")"),
            "typeText must reject non-editable targets before writing (#154)"
        );
        assert!(
            check_body.contains("requireCheckable("),
            "check must reject non-checkbox/radio targets before toggling (#154)"
        );
        assert!(
            editable_body
                .contains("requires an <input>, <textarea>, <select>, or contenteditable element"),
            "fill/type error must name the accepted elements (#154)"
        );
        assert!(
            checkable_body
                .contains("check requires an <input type=\"checkbox\"> or <input type=\"radio\">"),
            "check error must name checkbox and radio (#154)"
        );
        assert!(
            !editable_body.contains("instanceof") && !checkable_body.contains("instanceof"),
            "fill/type/check guards must be realm-safe — no instanceof (#154)"
        );
        assert!(
            bridge_fn_body(js, "function fillContentEditable(").contains("insertText"),
            "fill must edit contenteditable via insertText so rich-text editors see the write (#154)"
        );
        assert!(
            !bridge_fn_body(js, "function fillContentEditable(").contains("selectAll"),
            "fill must not selectAll the editing host; that replaces the whole editor (#154)"
        );
        assert!(
            bridge_fn_body(js, "function typeContentEditable(").contains("insertText"),
            "type must edit contenteditable via insertText so rich-text editors see the write (#154)"
        );
        assert!(
            bridge_fn_body(js, "function isContentEditable(").contains("plaintext-only"),
            "contenteditable detection must accept plaintext-only hosts (#154)"
        );
        assert!(
            type_body.contains("type cannot target a <select>"),
            "type must reject <select> instead of writing a raw value (#154)"
        );
        assert!(
            fill_body.contains("applySelectOption("),
            "fill must select an option rather than writing a raw value (#154)"
        );
    }

    #[cfg(all(unix, not(target_os = "android"), debug_assertions))]
    #[test]
    fn second_instance_starts_when_socket_already_bound() {
        // #152: the plugin is debug-only tooling, so a second instance of the
        // host app must start even though the first one already owns the pilot
        // socket. The bind failure is logged and the server is skipped.
        use tauri::Manager;

        let identifier = format!("com.pilot.issue152-{}", std::process::id());
        let address = super::server::socket_address(&super::sanitize_identifier(&identifier))
            .expect("socket address");
        let path = address
            .as_pathname()
            .expect("pathname socket")
            .to_path_buf();
        // First instance: a live listener on the pilot socket.
        let _first = std::os::unix::net::UnixListener::bind(&path).expect("bind first instance");

        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context.config_mut().identifier = identifier;
        let app = tauri::test::mock_builder()
            .plugin(super::init())
            .build(context);
        let _ = std::fs::remove_file(&path);

        let app = app.expect("second instance must start without a pilot server (#152)");
        assert!(
            app.try_state::<super::EvalEngine>().is_some(),
            "plugin setup must still run for the second instance"
        );
    }

    #[cfg(all(any(unix, windows), debug_assertions))]
    #[test]
    fn bridge_role_map_maps_paragraph_and_keeps_it_noninteractive() {
        // #109: <p> text (e.g. the default Tauri template greeting rendered in
        // a <p>) was dropped from snapshots because ROLE_MAP had no P entry, so
        // getRole returned null and walk() never emitted the node.
        let js = super::BRIDGE_JS;

        assert!(
            js.contains("P: \"paragraph\""),
            "ROLE_MAP must map P to \"paragraph\" so snapshot includes <p> text (#109)"
        );

        // The paragraph role must stay non-interactive so `snapshot --interactive`
        // still excludes <p>. Verify INTERACTIVE_ROLES does not list it.
        let set_start = js
            .find("INTERACTIVE_ROLES = new Set([")
            .expect("INTERACTIVE_ROLES set missing");
        let set_body = &js[set_start..];
        let set_end = set_body
            .find("]);")
            .expect("INTERACTIVE_ROLES set unterminated");
        assert!(
            !set_body[..set_end].contains("\"paragraph\""),
            "paragraph must stay out of INTERACTIVE_ROLES so interactive snapshots still exclude <p> (#109)"
        );
    }

    #[cfg(all(any(unix, windows), debug_assertions))]
    #[test]
    fn bridge_snapshot_emits_div_based_interactive_hosts() {
        // #155: unmapped tags (DIV) were dropped from snapshots even when they
        // carried draggable/contenteditable/onclick, so `snapshot -i` had no
        // ref for Kanban cards or editors. The walk still requires a role, so
        // getRole must call the fallback, and DIV must stay out of ROLE_MAP or
        // every layout wrapper would flood the tree.
        let js = super::BRIDGE_JS;

        assert!(
            bridge_fn_body(js, "function getRole(").contains("fallbackRole(el)"),
            "getRole must fall back so unmapped interactive hosts get a role (#155)"
        );

        let interactive = bridge_fn_body(js, "function isInteractiveElement(");
        assert!(
            interactive.contains("getAttribute(\"draggable\")"),
            "isInteractiveElement must read the draggable attribute (#155)"
        );
        assert!(
            interactive.contains("=== \"true\""),
            "isInteractiveElement must require draggable=\"true\", not a truthy value (#155)"
        );
        assert!(
            interactive.contains("hasAttribute(\"onclick\")"),
            "isInteractiveElement must treat an onclick attribute as interactive (#155)"
        );
        assert!(
            interactive.contains("typeof el.onclick"),
            "isInteractiveElement must treat an onclick property as interactive (#155)"
        );
        assert!(
            interactive.contains("carriesContentEditable(el)"),
            "isInteractiveElement must call carriesContentEditable, not inherited isContentEditable (#155)"
        );

        let fallback = bridge_fn_body(js, "function fallbackRole(");
        assert!(
            fallback.contains("carriesContentEditable(el)"),
            "fallbackRole must detect contenteditable hosts (#155)"
        );
        assert!(
            fallback.contains("return \"textbox\""),
            "contenteditable hosts must snapshot as textbox (#155)"
        );
        assert!(
            fallback.contains("return \"generic\""),
            "other unmapped interactive hosts must snapshot as generic (#155)"
        );

        let map_start = js.find("const ROLE_MAP = {").expect("ROLE_MAP missing");
        let map_body = &js[map_start..];
        let map_end = map_body.find("};").expect("ROLE_MAP unterminated");
        assert!(
            !map_body[..map_end].contains("DIV:"),
            "ROLE_MAP must not map DIV, or layout wrappers flood the snapshot (#155)"
        );
    }
}
