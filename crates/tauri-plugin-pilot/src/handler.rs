use crate::diff;
use crate::eval::{EvalEngine, EvalError, HELLO_ID, origin_key};
#[cfg(feature = "press")]
use crate::key;
use crate::protocol::{RPC_INVALID_PARAMS, RpcError};
use crate::recorder::{RecordEntry, Recorder};
use crate::screenshot;
use crate::webview::{TargetWindow, Webviews};

use std::time::Duration;
#[cfg(feature = "press")]
use tokio::sync::Mutex as AsyncMutex;

/// Delay after requesting window focus before injecting OS-level keyboard
/// events, so the window manager has time to actually transfer focus. Tuned
/// empirically — too short and the first key on Wayland drops; too long and
/// press feels sluggish.
#[cfg(feature = "press")]
const FOCUS_SETTLE_MS: u64 = 80;

/// Serializes the full `focus → settle → inject` sequence across concurrent
/// `press` calls. The inner `key::PRESS_LOCK` only covers the OS injection,
/// so without this outer lock two calls targeting different windows could
/// race on the focus step and deliver both keys to whichever window won the
/// focus race.
#[cfg(feature = "press")]
static PRESS_ORDER_LOCK: AsyncMutex<()> = AsyncMutex::const_new(());

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(30);
/// Default JS-side timeout when params omit it. Mirrors the bridge default
/// (`waitFor`/`watch` both fall back to `10_000` ms in `bridge.js`).
const DEFAULT_BRIDGE_TIMEOUT_MS: u64 = 10_000;
/// Extra headroom added to the JS-side timeout so the Rust oneshot channel
/// doesn't expire before the JS callback can resolve/reject. Without this,
/// a user-supplied `wait`/`watch` timeout above `DEFAULT_TIMEOUT` would be
/// silently capped by the Rust channel (fixes #91 — the cryptic
/// "eval timed out after 10s" that hid the bridge-side selector error).
///
/// This is a *ceiling*, not a per-call cost: when the bridge resolves or
/// rejects normally, the channel returns within milliseconds and the buffer
/// is never consumed. The buffer only kicks in when JS is stuck (frozen
/// webview, blocked main thread). Matches the prior `WATCH_BUFFER_MS` value;
/// covers the `__TAURI_INTERNALS__.invoke('plugin:pilot|__callback', …)`
/// roundtrip plus a GC pause without slowing fast-path failures.
const BRIDGE_TIMEOUT_BUFFER_MS: u64 = 2_000;

/// Compute the Rust-side eval timeout for a method whose JS implementation
/// honors `options.timeout` (currently `wait` and `watch`). Pads the user
/// value with [`BRIDGE_TIMEOUT_BUFFER_MS`] so the bridge always gets to surface
/// its own well-formed rejection before the channel goes silent.
fn bridge_eval_timeout(params: Option<&serde_json::Value>) -> Duration {
    let timeout_ms = params
        .and_then(|p| p.get("timeout"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(DEFAULT_BRIDGE_TIMEOUT_MS);
    Duration::from_millis(timeout_ms.saturating_add(BRIDGE_TIMEOUT_BUFFER_MS))
}

/// Compute the Rust-side eval timeout for `drag`, whose JS implementation
/// awaits its own timers: `steps` moves spaced by `stepDelayMs`, then a
/// `settleMs` pause after the release. Coercion, defaults and the `steps`
/// clamp mirror `drag()` in `bridge.js`. Without this, a caller tuning the gesture past
/// [`DEFAULT_TIMEOUT`] gets an RPC timeout while the bridge is still mid-drag,
/// and the pending result is dropped.
fn drag_eval_timeout(params: Option<&serde_json::Value>) -> Duration {
    // Values `bridge.js` itself rejects (NaN, negative) fall back to its own
    // default here too. The shapes `Number()` accepts and Rust doesn't —
    // `null` and `""` give 0, `true` gives 1 — land on a default larger than
    // what the bridge computes, so they over-estimate the budget. That costs
    // nothing but headroom the caller never pays unless JS is genuinely stuck.
    let field = |key: &str, default: f64| {
        params
            .and_then(|p| p.get(key))
            .and_then(coerce_number)
            .filter(|v| v.is_finite() && *v >= 0.0)
            .unwrap_or(default)
    };
    // `bridge.js` floors `steps` and sends anything below 1 back to its default
    // of 12, so a clamp to 1 here would under-budget a `steps: 0` call by an
    // order of magnitude.
    let steps = match field("steps", 12.0).floor() {
        s if s < 1.0 => 12.0,
        s => s.min(60.0),
    };
    let gesture_ms = steps * field("stepDelayMs", 16.0) + field("settleMs", 250.0);
    // Only a value near f64::MAX can overflow the conversion, and a caller
    // asking for a gesture longer than the heat death of the universe gets the
    // floor rather than a panic.
    Duration::try_from_secs_f64(gesture_ms / 1000.0)
        .unwrap_or(DEFAULT_TIMEOUT)
        .saturating_add(Duration::from_millis(BRIDGE_TIMEOUT_BUFFER_MS))
        .max(DEFAULT_TIMEOUT)
}

/// Mirror the `Number()` coercion `bridge.js` applies to each drag tunable
/// before validating it. A numeric string is a real value over there
/// (`Number("1000")` is `1000`, and so is `Number("0x3e8")`), so reading one
/// as "absent" here would under-budget the gesture by three orders of
/// magnitude.
fn coerce_number(value: &serde_json::Value) -> Option<f64> {
    let serde_json::Value::String(text) = value else {
        return value.as_f64();
    };
    let text = text.trim();
    // Radix-prefixed integer literals are the one string form `Number()`
    // accepts and `f64::from_str` rejects. Accumulated in `f64` rather than an
    // integer type because `Number()` puts no width limit on them and `steps`
    // clamps to 60 afterwards, so a capped parse would send a huge count back
    // to the default of 12 instead of to the clamp.
    let lowered = text.to_ascii_lowercase();
    for (prefix, radix) in [("0x", 16_u32), ("0o", 8), ("0b", 2)] {
        if let Some(digits) = lowered.strip_prefix(prefix) {
            // `Number("0x")` is NaN, and `try_fold` would call it 0.
            if digits.is_empty() {
                return None;
            }
            return digits.chars().try_fold(0.0_f64, |acc, digit| {
                digit
                    .to_digit(radix)
                    .map(|d| acc.mul_add(f64::from(radix), f64::from(d)))
            });
        }
    }
    text.parse().ok()
}

/// Extract and remove the optional `"window"` key from params.
///
/// Returns `(window_label, cleaned_params)`:
/// - When `"window"` is present: `cleaned_params` is `Some(...)` with the key stripped.
/// - When `"window"` is absent: `cleaned_params` is `None` — the caller must fall back
///   to the original `params` reference (e.g. via `.as_ref().or(params)`).
fn extract_window(
    params: Option<&serde_json::Value>,
) -> (Option<String>, Option<serde_json::Value>) {
    let window = params
        .and_then(|o| o.get("window"))
        .and_then(|v| v.as_str())
        .map(String::from);
    match (window, params) {
        (Some(w), Some(p)) => {
            let mut cleaned = p.clone();
            if let Some(obj) = cleaned.as_object_mut() {
                obj.remove("window");
            }
            (Some(w), Some(cleaned))
        }
        (w, _) => (w, None),
    }
}

/// Merge the plugin's compile-time version into a JSON object response.
///
/// No-op when the value is not an object. Single source for the
/// `plugin_version` field so `ping` and `state` report the same value (#135).
fn inject_plugin_version(result: &mut serde_json::Value) {
    if let Some(obj) = result.as_object_mut() {
        obj.insert(
            "plugin_version".to_owned(),
            serde_json::json!(env!("CARGO_PKG_VERSION")),
        );
    }
}

/// Dispatch a JSON-RPC method call to the appropriate handler.
#[allow(clippy::too_many_lines)]
pub(crate) async fn dispatch(
    method: &str,
    params: Option<&serde_json::Value>,
    engine: &EvalEngine,
    webviews: &dyn Webviews,
    recorder: &Recorder,
) -> Result<serde_json::Value, RpcError> {
    // Save original params before window extraction so the recorder can strip
    // "window" internally.
    let original_params = params.cloned();

    let (window, owned_params) = extract_window(params);
    let params = owned_params.as_ref().or(params);
    let win = window.as_deref();

    let result = match method {
        "ping" => {
            let mut result = serde_json::json!({"status": "ok"});
            inject_plugin_version(&mut result);
            Ok(result)
        }
        "windows.list" => Ok(serde_json::json!({"windows": webviews.list()})),
        "snapshot" => {
            let result =
                handle_eval_method("snapshot", params, engine, webviews, win, DEFAULT_TIMEOUT)
                    .await?;
            engine.store_snapshot(&result);
            Ok(result)
        }
        "diff" => handle_diff(params, engine, webviews, win).await,
        #[cfg(feature = "press")]
        "press" => handle_press(params, webviews, win).await,
        #[cfg(not(feature = "press"))]
        "press" => Err(RpcError {
            code: -32601,
            message: "press disabled (compile `tauri-plugin-pilot` with the `press` feature)"
                .to_owned(),
            data: None,
        }),
        "click" | "fill" | "type" | "select" | "check" | "scroll" | "drop" | "text" | "html"
        | "value" | "attrs" | "eval" | "ipc" | "url" | "title" | "visible" | "count"
        | "checked" => {
            handle_eval_method(method, params, engine, webviews, win, DEFAULT_TIMEOUT).await
        }
        "navigate" => handle_navigate(params, engine, webviews, win).await,
        // `drag` spends `steps × stepDelayMs + settleMs` in JS timers before it
        // resolves, so the channel timeout has to cover the gesture the caller
        // asked for.
        "drag" => {
            handle_eval_method(
                method,
                params,
                engine,
                webviews,
                win,
                drag_eval_timeout(params),
            )
            .await
        }
        // `state` is a bridge-derived method (url/title/ready), but the dispatch
        // merges the plugin's compile-time version into the result so a single
        // `state` call also surfaces plugin/CLI version drift (issue #135).
        "state" => {
            let mut result =
                handle_eval_method("state", params, engine, webviews, win, DEFAULT_TIMEOUT).await?;
            inject_plugin_version(&mut result);
            Ok(result)
        }
        // `wait` and `watch` both honor a JS-side `options.timeout`; the Rust
        // channel timeout must outlive that so the bridge can surface its own
        // rejection (issue #91).
        "wait" | "watch" => {
            handle_eval_method(
                method,
                params,
                engine,
                webviews,
                win,
                bridge_eval_timeout(params),
            )
            .await
        }
        // The bare `screenshot` JSON-RPC method is the bridge-side
        // html-to-image path (returns a base64 PNG data URL); native window
        // capture ships under the distinct `screenshot_native` method so the
        // two surfaces can't be confused by callers or accidentally folded
        // together by a future refactor.
        "screenshot" => {
            handle_eval_method(method, params, engine, webviews, win, SCREENSHOT_TIMEOUT).await
        }
        "screenshot_native" => screenshot::handle_screenshot(params).await,
        "console.getLogs" => {
            handle_eval_method(
                "consoleLogs",
                params,
                engine,
                webviews,
                win,
                DEFAULT_TIMEOUT,
            )
            .await
        }
        "console.clear" => {
            handle_eval_method("clearLogs", params, engine, webviews, win, DEFAULT_TIMEOUT).await
        }
        "network.getRequests" => {
            handle_eval_method(
                "networkRequests",
                params,
                engine,
                webviews,
                win,
                DEFAULT_TIMEOUT,
            )
            .await
        }
        "network.clear" => {
            handle_eval_method(
                "clearNetwork",
                params,
                engine,
                webviews,
                win,
                DEFAULT_TIMEOUT,
            )
            .await
        }
        "storage.get" => {
            handle_eval_method("storageGet", params, engine, webviews, win, DEFAULT_TIMEOUT).await
        }
        "storage.set" => {
            handle_eval_method("storageSet", params, engine, webviews, win, DEFAULT_TIMEOUT).await
        }
        "storage.list" => {
            handle_eval_method(
                "storageList",
                params,
                engine,
                webviews,
                win,
                DEFAULT_TIMEOUT,
            )
            .await
        }
        "storage.clear" => {
            handle_eval_method(
                "storageClear",
                params,
                engine,
                webviews,
                win,
                DEFAULT_TIMEOUT,
            )
            .await
        }
        "forms.dump" => {
            handle_eval_method("formDump", params, engine, webviews, win, DEFAULT_TIMEOUT).await
        }
        "record.start" => {
            recorder.start();
            Ok(serde_json::json!({"status": "recording"}))
        }
        "record.stop" => {
            let entries = recorder.stop().ok_or_else(|| RpcError {
                code: RPC_INVALID_PARAMS,
                message: "No recording in progress. Run `record start` first".to_owned(),
                data: None,
            })?;
            let count = entries.len();
            Ok(serde_json::json!({"entries": entries, "count": count}))
        }
        "record.status" => Ok(recorder.status()),
        "record.add" => {
            let entry: RecordEntry =
                serde_json::from_value(params.cloned().unwrap_or(serde_json::Value::Null))
                    .map_err(|e| RpcError {
                        code: -32602,
                        message: e.to_string(),
                        data: None,
                    })?;
            recorder.add_entry(entry);
            Ok(serde_json::json!({"status": "ok"}))
        }
        _ => Err(RpcError {
            code: -32601,
            message: format!("Method not found: {method}"),
            data: None,
        }),
    };

    // Auto-record on successful dispatches
    if result.is_ok() && recorder.is_active() {
        recorder.record(method, original_params.as_ref());
    }

    result
}

/// Handle the "diff" method: take a new snapshot, compare with the reference, and return `DiffResult`.
async fn handle_diff(
    params: Option<&serde_json::Value>,
    engine: &EvalEngine,
    webviews: &dyn Webviews,
    window: Option<&str>,
) -> Result<serde_json::Value, RpcError> {
    // Determine reference snapshot: from params["reference"] or last stored snapshot
    let reference = if let Some(ref_val) = params.and_then(|p| p.get("reference")) {
        ref_val.clone()
    } else {
        engine.get_last_snapshot().ok_or_else(|| RpcError {
            code: -32602,
            message:
                "No previous snapshot available. Run `snapshot` first or use `diff --ref <file>`"
                    .to_owned(),
            data: None,
        })?
    };

    // Take a new snapshot using the bridge — strip "reference" to avoid embedding
    // the entire old snapshot in the JS eval string (the bridge doesn't use it).
    let snapshot_params = params.map(|p| {
        let mut cleaned = p.clone();
        if let Some(obj) = cleaned.as_object_mut() {
            obj.remove("reference");
        }
        cleaned
    });
    let script =
        build_bridge_call("snapshot", snapshot_params.as_ref()).map_err(|msg| RpcError {
            code: -32602,
            message: msg,
            data: None,
        })?;
    let result = eval_bridge(&script, engine, webviews, window, DEFAULT_TIMEOUT).await?;

    // Parse both snapshots: extract "elements" arrays
    let old_elements: Vec<diff::SnapshotElement> = reference
        .get("elements")
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()
        .map_err(|e| RpcError {
            code: -32602,
            message: format!("Failed to parse reference snapshot elements: {e}"),
            data: None,
        })?
        .unwrap_or_default();

    let new_elements: Vec<diff::SnapshotElement> = result
        .get("elements")
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()
        .map_err(|e| RpcError {
            code: -32603,
            message: format!("Failed to parse new snapshot elements: {e}"),
            data: None,
        })?
        .unwrap_or_default();

    let diff_result = diff::compute_diff(&old_elements, &new_elements);

    // Store the new snapshot for subsequent diffs
    engine.store_snapshot(&result);

    serde_json::to_value(&diff_result).map_err(|e| RpcError {
        code: -32603,
        message: format!("Serialization error: {e}"),
        data: None,
    })
}

/// Handle the "press" method by injecting an OS-level keyboard event.
///
/// JS-dispatched `KeyboardEvent`s are flagged `isTrusted: false` and never
/// reach Tauri accelerators (#45). Native injection via `enigo` produces real
/// keyboard events that DOM listeners and Tauri accelerators see as trusted.
///
/// Note: on X11, global shortcuts (`tauri-plugin-global-shortcut` / `XGrabKey`
/// passive grabs) are keyed on physical keycodes. Letter and digit combos fire
/// (#114), but characters that sit above shift-level 0 on an exotic layout may
/// still be remapped by `enigo` and miss the grab. See the `key` module-level
/// docs (#45, #75, #114).
#[cfg(feature = "press")]
async fn handle_press(
    params: Option<&serde_json::Value>,
    webviews: &dyn Webviews,
    window: Option<&str>,
) -> Result<serde_json::Value, RpcError> {
    let key_str = params
        .and_then(|p| p.get("key"))
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| RpcError {
            code: -32602,
            message: "press requires a non-empty \"key\" string param".to_owned(),
            data: None,
        })?;

    // Parse the combo up front: a bad combo is a client input error, so we
    // shouldn't take the serialization lock, steal focus, or sleep for it —
    // and we report it as -32602 (invalid params) instead of letting the
    // later spawn_blocking path surface it as -32603 (internal error).
    key::parse_combo(key_str).map_err(|e| RpcError {
        code: -32602,
        message: format!("invalid press combo: {e}"),
        data: None,
    })?;

    // Hold this lock across the whole focus → settle → inject sequence so
    // two concurrent `press` calls cannot interleave their focus steps (call
    // A focuses window X, call B focuses window Y, then both keys land on Y).
    let _order_guard = PRESS_ORDER_LOCK.lock().await;

    // With no window to focus, the key would land in whatever app has focus.
    let target = webviews.target(window).map_err(|e| RpcError {
        code: -32603,
        message: format!("cannot focus target window: {e}"),
        data: None,
    })?;
    match target.focus() {
        Ok(()) => {
            // Only wait if the WM actually accepted the focus request —
            // a failed focus call won't transfer focus, so sleeping
            // would just delay the press for nothing.
            tokio::time::sleep(Duration::from_millis(FOCUS_SETTLE_MS)).await;
        }
        Err(e) => {
            if let Some(label) = window {
                // The caller explicitly targeted a window; silently
                // falling through would deliver the key to whatever
                // window currently has focus and still return ok.
                return Err(RpcError {
                    code: -32603,
                    message: format!("failed to focus window '{label}': {e}"),
                    data: None,
                });
            }
            tracing::warn!(error = %e, "focus before press failed (continuing)");
        }
    }

    let combo = key_str.to_owned();
    tokio::task::spawn_blocking(move || key::simulate_press(&combo))
        .await
        .map_err(|e| {
            // A JoinError can be a panic, a cancellation, or a runtime
            // shutdown — reporting every one as "panicked" misleads during
            // teardown.
            let message = if e.is_panic() {
                format!("press task panicked: {e}")
            } else if e.is_cancelled() {
                "press task was cancelled".to_owned()
            } else {
                format!("press task failed: {e}")
            };
            RpcError {
                code: -32603,
                message,
                data: None,
            }
        })?
        .map_err(|e| RpcError {
            code: -32603,
            message: format!("press failed: {e}"),
            data: None,
        })?;

    Ok(serde_json::json!({"ok": true}))
}

/// Handle a method that requires JS evaluation via the bridge.
async fn handle_eval_method(
    method: &str,
    params: Option<&serde_json::Value>,
    engine: &EvalEngine,
    webviews: &dyn Webviews,
    window: Option<&str>,
    timeout: Duration,
) -> Result<serde_json::Value, RpcError> {
    let script = build_bridge_call(method, params).map_err(|msg| RpcError {
        code: -32602,
        message: msg,
        data: None,
    })?;
    eval_bridge(&script, engine, webviews, window, timeout).await
}

/// How long `navigate` waits for a hello from an origin whose bridge never
/// said hello before (#153).
///
/// A foreign origin can only call back when a capability lists it in
/// `remote.urls`, and a denied call sends no signal, so silence past this
/// delay is the verdict. Long enough for a local page to load and run the
/// init script. A slow remote page that misses it still answers the next
/// commands once its hello lands.
const BRIDGE_GRACE: Duration = Duration::from_secs(3);

/// Handle `navigate`, which reports ok only once a bridge can answer on the
/// destination (#153).
///
/// The bridge answers before the webview leaves the page, so the callback
/// alone proves nothing about the destination. When the destination origin
/// has no known bridge, the hello of the new page is the proof.
async fn handle_navigate(
    params: Option<&serde_json::Value>,
    engine: &EvalEngine,
    webviews: &dyn Webviews,
    window: Option<&str>,
) -> Result<serde_json::Value, RpcError> {
    let script = build_bridge_call("navigate", params).map_err(|msg| RpcError {
        code: -32602,
        message: msg,
        data: None,
    })?;
    let since = engine.hellos();
    let target = target(webviews, window)?;
    // Read before the eval so the URL names the page the script lands on.
    let page = target.url();
    let (id, rx) = send_script(&script, engine, target.as_ref())?;
    let dest = navigate_destination(
        page.as_ref(),
        params.and_then(|p| p.get("url").and_then(serde_json::Value::as_str)),
    );

    match (page.as_ref(), dest.as_ref()) {
        (Some(page), Some(dest)) if origin_key(page) == origin_key(dest) => {
            if engine.has_bridge(page) {
                wait(engine, id, rx, DEFAULT_TIMEOUT).await
            } else {
                Err(fail_no_bridge(engine, id, page))
            }
        }
        (Some(page), Some(dest)) => {
            if engine.has_bridge(page) {
                // The departing page may be torn down before `__callback` runs.
                // Treat that timeout as non-fatal and let the dest hello decide.
                match engine.wait(id, rx, BRIDGE_GRACE).await {
                    Ok(result) if engine.has_bridge(dest) => return Ok(result),
                    Ok(_) | Err(EvalError::Timeout(_)) => {}
                    Err(e) => {
                        return Err(RpcError {
                            code: -32603,
                            message: format!("Eval error: {e}"),
                            data: None,
                        });
                    }
                }
            } else {
                // The script still navigates, but this page cannot call back.
                engine.resolve(id, Err("page has no pilot bridge".to_owned()));
            }
            wait_dest_bridge(engine, dest, since).await
        }
        (Some(page), None) => {
            if engine.has_bridge(page) {
                wait(engine, id, rx, DEFAULT_TIMEOUT).await
            } else {
                Err(fail_no_bridge(engine, id, page))
            }
        }
        (None, Some(dest)) => {
            engine.resolve(id, Err("waiting for destination bridge".to_owned()));
            wait_dest_bridge(engine, dest, since).await
        }
        (None, None) => wait(engine, id, rx, DEFAULT_TIMEOUT).await,
    }
}

/// Resolve `navigate`'s `url` param against the current page when needed.
///
/// Absolute URLs do not need a base, so they survive a missing page URL.
/// A `javascript:` URL runs in the current document, so dest is that page.
fn navigate_destination(page: Option<&tauri::Url>, raw: Option<&str>) -> Option<tauri::Url> {
    let raw = raw?;
    match tauri::Url::parse(raw) {
        Ok(absolute) if absolute.scheme() == "javascript" => page.cloned(),
        Ok(absolute) => Some(absolute),
        Err(_) => page.and_then(|base| base.join(raw).ok()),
    }
}

/// Wait for a hello from `dest`, using the 3s grace on a first visit.
async fn wait_dest_bridge(
    engine: &EvalEngine,
    dest: &tauri::Url,
    since: u64,
) -> Result<serde_json::Value, RpcError> {
    let limit = if engine.has_bridge(dest) {
        DEFAULT_TIMEOUT
    } else {
        BRIDGE_GRACE
    };
    if engine.wait_bridge(dest, since, limit).await {
        Ok(serde_json::json!({"ok": true}))
    } else {
        Err(no_bridge_error(
            engine,
            &format!("navigated to {dest}, but no pilot bridge answered there within {limit:?}"),
        ))
    }
}

fn fail_no_bridge(engine: &EvalEngine, id: u64, page: &tauri::Url) -> RpcError {
    engine.resolve(id, Err("page has no pilot bridge".to_owned()));
    no_bridge_error(
        engine,
        &format!("no pilot bridge on the current page ({page})"),
    )
}

/// Send a bridge script and wait for its callback.
///
/// Refuses without running the script when the page has no bridge that can
/// call back, instead of waiting out `timeout` (#153).
async fn eval_bridge(
    script: &str,
    engine: &EvalEngine,
    webviews: &dyn Webviews,
    window: Option<&str>,
    timeout: Duration,
) -> Result<serde_json::Value, RpcError> {
    let target = target(webviews, window)?;
    if let Some(page) = target.url().filter(|page| !engine.has_bridge(page)) {
        return Err(no_bridge_error(
            engine,
            &format!("no pilot bridge on the current page ({page})"),
        ));
    }
    let (id, rx) = send_script(script, engine, target.as_ref())?;
    wait(engine, id, rx, timeout).await
}

/// Resolve the window a request targets.
fn target<'a>(
    webviews: &'a dyn Webviews,
    window: Option<&str>,
) -> Result<Box<dyn TargetWindow + 'a>, RpcError> {
    webviews.target(window).map_err(|e| RpcError {
        code: -32603,
        message: format!("Eval failed: {e}"),
        data: None,
    })
}

/// Register a callback, then eval `script` wrapped in the ADR-001 pattern.
fn send_script(
    script: &str,
    engine: &EvalEngine,
    target: &dyn TargetWindow,
) -> Result<CallbackSlot, RpcError> {
    let (id, rx) = engine.register();
    let wrapped = EvalEngine::wrap_script(id, script);
    match target.eval(&wrapped) {
        Ok(()) => Ok((id, rx)),
        Err(e) => {
            // Clean up pending entry on eval failure
            engine.resolve(id, Err(format!("Eval failed: {e}")));
            Err(RpcError {
                code: -32603,
                message: format!("Eval failed: {e}"),
                data: None,
            })
        }
    }
}

/// Callback id and its receiver.
type CallbackSlot = (
    u64,
    tokio::sync::oneshot::Receiver<Result<serde_json::Value, String>>,
);

/// Wait for the callback of eval `id`.
async fn wait(
    engine: &EvalEngine,
    id: u64,
    rx: tokio::sync::oneshot::Receiver<Result<serde_json::Value, String>>,
    timeout: Duration,
) -> Result<serde_json::Value, RpcError> {
    engine.wait(id, rx, timeout).await.map_err(|e| RpcError {
        code: -32603,
        message: format!("Eval error: {e}"),
        data: None,
    })
}

/// Build the error for a page whose bridge cannot answer, naming the
/// origins where it can.
fn no_bridge_error(engine: &EvalEngine, what: &str) -> RpcError {
    let origins = engine.bridge_origins().join(", ");
    RpcError {
        code: -32603,
        message: format!(
            "{what}. Bridge commands only work on {origins}: navigate back there, \
             or allow this origin in a capability's remote.urls"
        ),
        data: None,
    }
}

/// Build a `window.__PILOT__.<method>(params)` JS call string.
/// Returns `Err` with a message for invalid params (e.g. missing ipc command).
fn build_bridge_call(method: &str, params: Option<&serde_json::Value>) -> Result<String, String> {
    let args = match params {
        Some(v) if !v.is_null() => v.to_string(),
        _ => "{}".to_owned(),
    };

    if method == "ipc" {
        // ipc calls Tauri's backend invoke directly
        // serde_json::to_string produces a valid JS string literal (escaped quotes, backslashes, etc.)
        let command = params
            .and_then(|p| p.get("command"))
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "ipc requires a non-empty \"command\" string param".to_owned())?;
        let command_js = serde_json::to_string(command).unwrap_or_else(|_| "\"\"".to_owned());
        let ipc_args = params
            .and_then(|p| p.get("args"))
            .map_or("{}".to_owned(), ToString::to_string);
        return Ok(format!(
            "window.__TAURI_INTERNALS__.invoke({command_js}, {ipc_args})"
        ));
    }

    Ok(format!("window.__PILOT__.{method}({args})"))
}

/// Process the IPC callback from the JS bridge (ADR-001).
pub(crate) fn handle_callback(
    engine: &EvalEngine,
    id: u64,
    result: Option<String>,
    error: Option<String>,
    page: Option<&tauri::Url>,
) {
    if id == HELLO_ID {
        // Record the invoking webview's URL, not the client payload. If the
        // payload parses as a different origin, the webview likely navigated
        // before this IPC landed; drop the hello rather than tagging dest.
        let client = result
            .as_deref()
            .and_then(|href| tauri::Url::parse(href).ok());
        match (page, client.as_ref()) {
            (Some(webview), Some(client)) if origin_key(webview) != origin_key(client) => {
                tracing::warn!("bridge hello origin mismatch, ignoring");
            }
            (Some(webview), _) => engine.bridge_hello(webview.as_str()),
            (None, _) => tracing::warn!("bridge hello without a webview URL"),
        }
        return;
    }
    if let Some(err) = error {
        engine.resolve(id, Err(err));
    } else if let Some(res) = result {
        match serde_json::from_str(&res) {
            Ok(val) => engine.resolve(id, Ok(val)),
            Err(_) => engine.resolve(id, Ok(serde_json::Value::String(res))),
        }
    } else {
        tracing::warn!(id, "callback received with neither result nor error");
        engine.resolve(id, Ok(serde_json::Value::Null));
    }
}

/// Tauri IPC command for the eval callback handler.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "tauri::command contract — macro wrapper is the real consumer"
)]
pub(crate) fn callback<R: tauri::Runtime>(
    eval_engine: tauri::State<'_, EvalEngine>,
    webview: tauri::WebviewWindow<R>,
    id: u64,
    result: Option<String>,
    error: Option<String>,
) {
    handle_callback(
        &eval_engine,
        id,
        result,
        error,
        crate::webview::current_url(&webview).as_ref(),
    );
}

/// Legacy Tauri IPC command for the `__callback` handler.
///
/// `#[tauri::command]` binds `State<'_, T>` by value — the generated wrapper
/// is the true consumer, so clippy's view of the body is incomplete. Cannot
/// be rewritten as `&State` (tauri command macro rejects it). Documented
/// here rather than suppressed; this is the only call site that cannot
/// satisfy `needless_pass_by_value`.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "tauri::command contract — macro wrapper is the real consumer"
)]
pub(crate) fn __callback<R: tauri::Runtime>(
    eval_engine: tauri::State<'_, EvalEngine>,
    webview: tauri::WebviewWindow<R>,
    id: u64,
    result: Option<String>,
    error: Option<String>,
) {
    handle_callback(
        &eval_engine,
        id,
        result,
        error,
        crate::webview::current_url(&webview).as_ref(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webview::fake::FakeWebviews;
    use serde_json::json;

    #[tokio::test]
    async fn test_dispatch_ping_returns_ok() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "ping",
            None,
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await
        .expect("dispatch succeeds");
        assert_eq!(result["status"], json!("ok"));
    }

    #[tokio::test]
    async fn test_dispatch_ping_reports_plugin_version() {
        // The ping response carries the plugin's own compile-time version so a
        // caller can detect when the plugin baked into the app has drifted from
        // the CLI (issue #135). Older plugins (<= 0.7.0) omit the field, which
        // the CLI reads as "pre-introspection, upgrade recommended".
        let engine = EvalEngine::new();
        let result = dispatch(
            "ping",
            None,
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await
        .expect("dispatch succeeds");
        assert_eq!(result["plugin_version"], json!(env!("CARGO_PKG_VERSION")));
    }

    #[cfg(feature = "press")]
    #[tokio::test]
    async fn test_dispatch_press_with_invalid_combo_returns_invalid_params() {
        // A malformed combo must not steal focus or acquire the serialization
        // lock — it should short-circuit with -32602 (invalid params).
        let engine = EvalEngine::new();
        let result = dispatch(
            "press",
            Some(&json!({"key": "Control++P"})),
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("dispatch returns Err");
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("invalid press combo"));
    }

    #[cfg(feature = "press")]
    #[tokio::test]
    async fn test_dispatch_press_with_unknown_window_errors() {
        // --window <label> naming no window must not silently inject into
        // the currently focused window. We can pass `window` through params
        // (handler extracts it before dispatch).
        let engine = EvalEngine::new();
        let result = dispatch(
            "press",
            Some(&json!({"key": "Enter", "window": "settings"})),
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("dispatch returns Err");
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("focus"));
    }

    #[cfg(feature = "press")]
    #[tokio::test]
    async fn test_dispatch_press_without_any_window_errors() {
        // Without --window and with no webview, the key would reach another
        // app. Shift alone keeps a regression harmless.
        let engine = EvalEngine::new();
        let result = dispatch(
            "press",
            Some(&json!({"key": "Shift"})),
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("dispatch returns Err");
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview available"));
    }

    #[cfg(feature = "press")]
    #[tokio::test]
    async fn test_dispatch_press_with_missing_key_returns_invalid_params() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "press",
            None,
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("dispatch returns Err");
        assert_eq!(err.code, -32602);
    }

    #[tokio::test]
    async fn test_dispatch_unknown_method_returns_error() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "nonexistent",
            None,
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("dispatch returns Err");
        assert_eq!(err.code, -32601);
    }

    #[tokio::test]
    async fn test_dispatch_snapshot_without_webview() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "snapshot",
            None,
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("dispatch returns Err");
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[tokio::test]
    async fn test_dispatch_diff_without_webview() {
        let engine = EvalEngine::new();
        let params = json!({"reference": {"elements": []}});
        let result = dispatch(
            "diff",
            Some(&params),
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("dispatch returns Err");
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[tokio::test]
    async fn test_dispatch_diff_without_previous_snapshot() {
        let engine = EvalEngine::new();
        // The reference check runs before any eval, so a webview that never
        // answers is enough: no reference in params + no last_snapshot → -32602
        let webviews = FakeWebviews::window("main", None);
        let result = dispatch("diff", None, &engine, &webviews, &Recorder::new()).await;
        let err = result.expect_err("dispatch returns Err");
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("No previous snapshot"));
    }

    #[test]
    fn test_build_bridge_call_snapshot() {
        let params = json!({"interactive": true, "selector": null, "depth": 3});
        let script = build_bridge_call("snapshot", Some(&params)).expect("build_bridge_call");
        assert!(script.starts_with("window.__PILOT__.snapshot("));
        assert!(script.contains("\"interactive\":true"));
    }

    #[test]
    fn test_build_bridge_call_no_params() {
        let script = build_bridge_call("snapshot", None).expect("build_bridge_call");
        assert_eq!(script, "window.__PILOT__.snapshot({})");
    }

    #[test]
    fn test_build_bridge_call_ipc_missing_command() {
        let result = build_bridge_call("ipc", None);
        assert!(result.is_err());
        assert!(
            result
                .expect_err("ipc rejects missing command")
                .contains("command")
        );
    }

    #[tokio::test]
    async fn test_callback_with_json_result() {
        let engine = EvalEngine::new();
        let (id, rx) = engine.register();
        handle_callback(
            &engine,
            id,
            Some(r#"{"title":"hello"}"#.to_owned()),
            None,
            None,
        );
        let val = rx.await.expect("channel not dropped").expect("eval ok");
        assert_eq!(val, json!({"title": "hello"}));
    }

    #[tokio::test]
    async fn test_callback_with_null_string_resolves_to_value_null() {
        // #48 round-trip: wrap_script sends `result: 'null'` (string) when the
        // JS expr returns undefined. The callback must parse it back to
        // Value::Null so the client sees a clean success, not a warn fallback.
        let engine = EvalEngine::new();
        let (id, rx) = engine.register();
        handle_callback(&engine, id, Some("null".to_owned()), None, None);
        let val = rx.await.expect("channel not dropped").expect("eval ok");
        assert_eq!(val, serde_json::Value::Null);
    }

    #[tokio::test]
    async fn test_callback_with_error() {
        let engine = EvalEngine::new();
        let (id, rx) = engine.register();
        handle_callback(&engine, id, None, Some("TypeError: x".to_owned()), None);
        let result = rx.await.expect("channel not dropped");
        assert_eq!(result, Err("TypeError: x".to_owned()));
    }

    #[tokio::test]
    async fn test_dispatch_console_get_logs_without_webview() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "console.getLogs",
            None,
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("dispatch returns Err");
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[tokio::test]
    async fn test_dispatch_console_clear_without_webview() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "console.clear",
            None,
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("dispatch returns Err");
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[test]
    fn test_build_bridge_call_console_logs() {
        let params = json!({"level": "error", "last": 10});
        let script = build_bridge_call("consoleLogs", Some(&params)).expect("build_bridge_call");
        assert!(script.starts_with("window.__PILOT__.consoleLogs("));
        assert!(script.contains("\"level\":\"error\""));
    }

    #[test]
    fn test_build_bridge_call_clear_logs() {
        let script = build_bridge_call("clearLogs", None).expect("build_bridge_call");
        assert_eq!(script, "window.__PILOT__.clearLogs({})");
    }

    #[tokio::test]
    async fn test_dispatch_network_get_requests_without_webview() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "network.getRequests",
            None,
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("dispatch returns Err");
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[tokio::test]
    async fn test_dispatch_network_clear_without_webview() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "network.clear",
            None,
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("dispatch returns Err");
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[test]
    fn test_build_bridge_call_network_requests() {
        let params = json!({"filter": "/api", "failedOnly": true, "last": 10});
        let script =
            build_bridge_call("networkRequests", Some(&params)).expect("build_bridge_call");
        assert!(script.starts_with("window.__PILOT__.networkRequests("));
        assert!(script.contains("\"filter\":\"/api\""));
    }

    #[test]
    fn test_build_bridge_call_clear_network() {
        let script = build_bridge_call("clearNetwork", None).expect("build_bridge_call");
        assert_eq!(script, "window.__PILOT__.clearNetwork({})");
    }

    #[test]
    fn test_build_bridge_call_visible() {
        let params = json!({"ref": "el-1"});
        let script = build_bridge_call("visible", Some(&params)).expect("build_bridge_call");
        assert!(script.starts_with("window.__PILOT__.visible("));
        assert!(script.contains("\"ref\":\"el-1\""));
    }

    #[test]
    fn test_build_bridge_call_count() {
        let params = json!({"selector": ".item"});
        let script = build_bridge_call("count", Some(&params)).expect("build_bridge_call");
        assert!(script.starts_with("window.__PILOT__.count("));
        assert!(script.contains("\"selector\":\".item\""));
    }

    #[test]
    fn test_build_bridge_call_checked() {
        let params = json!({"ref": "el-2"});
        let script = build_bridge_call("checked", Some(&params)).expect("build_bridge_call");
        assert!(script.starts_with("window.__PILOT__.checked("));
        assert!(script.contains("\"ref\":\"el-2\""));
    }

    #[tokio::test]
    async fn test_dispatch_watch_without_webview() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "watch",
            None,
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("dispatch returns Err");
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[test]
    fn test_build_bridge_call_watch() {
        let params = json!({"timeout": 5000, "selector": ".results", "stable": 500});
        let script = build_bridge_call("watch", Some(&params)).expect("build_bridge_call");
        assert!(script.starts_with("window.__PILOT__.watch("));
        assert!(script.contains("\"timeout\":5000"));
    }

    #[tokio::test]
    async fn test_dispatch_drag_routes_to_eval() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "drag",
            None,
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("dispatch returns Err");
        assert_ne!(err.code, -32601);
    }

    #[tokio::test]
    async fn test_dispatch_drop_routes_to_eval() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "drop",
            None,
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("dispatch returns Err");
        assert_ne!(err.code, -32601);
    }

    #[test]
    fn test_build_bridge_call_drag() {
        let params = json!({"source": {"ref": "e5"}, "target": {"ref": "e6"}});
        let script = build_bridge_call("drag", Some(&params)).expect("build_bridge_call");
        assert!(script.starts_with("window.__PILOT__.drag("));
    }

    #[test]
    fn test_build_bridge_call_drop() {
        let params = json!({"ref": "e3", "files": []});
        let script = build_bridge_call("drop", Some(&params)).expect("build_bridge_call");
        assert!(script.starts_with("window.__PILOT__.drop("));
    }

    #[tokio::test]
    async fn test_dispatch_storage_get_without_webview() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "storage.get",
            None,
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("dispatch returns Err");
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[tokio::test]
    async fn test_dispatch_storage_set_without_webview() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "storage.set",
            None,
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("dispatch returns Err");
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[tokio::test]
    async fn test_dispatch_storage_list_without_webview() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "storage.list",
            None,
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("dispatch returns Err");
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[tokio::test]
    async fn test_dispatch_storage_clear_without_webview() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "storage.clear",
            None,
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("dispatch returns Err");
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[test]
    fn test_build_bridge_call_storage_get() {
        let params = json!({"key": "auth_token", "session": false});
        let script = build_bridge_call("storageGet", Some(&params)).expect("build_bridge_call");
        assert_eq!(
            script,
            r#"window.__PILOT__.storageGet({"key":"auth_token","session":false})"#
        );
    }

    #[test]
    fn test_build_bridge_call_storage_set() {
        let params = json!({"key": "theme", "value": "dark", "session": false});
        let script = build_bridge_call("storageSet", Some(&params)).expect("build_bridge_call");
        assert!(script.starts_with("window.__PILOT__.storageSet("));
        assert!(script.contains("\"key\":\"theme\""));
        assert!(script.contains("\"value\":\"dark\""));
        assert!(script.contains("\"session\":false"));
    }

    #[test]
    fn test_build_bridge_call_storage_list() {
        let params = json!({"session": true});
        let script = build_bridge_call("storageList", Some(&params)).expect("build_bridge_call");
        assert!(script.starts_with("window.__PILOT__.storageList("));
        assert!(script.contains("\"session\":true"));
    }

    #[test]
    fn test_build_bridge_call_storage_clear() {
        let params = json!({"session": false});
        let script = build_bridge_call("storageClear", Some(&params)).expect("build_bridge_call");
        assert!(script.starts_with("window.__PILOT__.storageClear("));
        assert!(script.contains("\"session\":false"));
    }

    #[test]
    fn test_build_bridge_call_form_dump() {
        let script = build_bridge_call("formDump", None).expect("build_bridge_call");
        assert_eq!(script, "window.__PILOT__.formDump({})");
    }

    #[test]
    fn test_build_bridge_call_form_dump_with_selector() {
        let params = json!({"selector": "#login-form"});
        let script = build_bridge_call("formDump", Some(&params)).expect("build_bridge_call");
        assert!(script.starts_with("window.__PILOT__.formDump("));
        assert!(script.contains("\"selector\":\"#login-form\""));
    }

    #[tokio::test]
    async fn test_dispatch_forms_dump_without_webview() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "forms.dump",
            None,
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("dispatch returns Err");
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[tokio::test]
    async fn test_dispatch_windows_list() {
        let engine = EvalEngine::new();
        let webviews = FakeWebviews::window("main", Some("http://localhost/"));
        let result = dispatch("windows.list", None, &engine, &webviews, &Recorder::new()).await;
        let val = result.expect("dispatch succeeds");
        let windows = val
            .get("windows")
            .expect("windows key present")
            .as_array()
            .expect("windows is array");
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].get("label").expect("label key present"), "main");
    }

    #[tokio::test]
    async fn test_dispatch_window_param_extracted_from_params() {
        let engine = EvalEngine::new();
        // The "window" key must be stripped before being forwarded to the bridge.
        // We verify this by inspecting the script the webview received.
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("settings", None).on_eval(move || {
            // Resolve the callback immediately to avoid blocking for the default 10s timeout.
            // ID 1 is the first registered callback on a fresh EvalEngine.
            engine_clone.resolve(1, Ok(serde_json::json!({"ok": true})));
        });
        let params = serde_json::json!({"ref": "el-1", "window": "settings"});
        let _ = dispatch("click", Some(&params), &engine, &webviews, &Recorder::new()).await;
        let script = webviews.scripts().pop().expect("script evaluated");
        // "window" param must not appear in the JS call args
        assert!(!script.contains("\"window\""));
        assert!(script.contains("\"ref\""));
    }

    #[tokio::test]
    async fn test_dispatch_screenshot_native_rejects_missing_window_id() {
        // The native method must validate `window_id` before any platform
        // path is touched, so this works the same on every host.
        let engine = EvalEngine::new();
        let params = json!({"output_path": "/tmp/x.png"});
        let err = dispatch(
            "screenshot_native",
            Some(&params),
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await
        .expect_err("missing window_id must surface as Err");
        assert_eq!(err.code, -32602);
        assert!(
            err.message.contains("window_id"),
            "error message must reference window_id"
        );
    }

    #[tokio::test]
    async fn test_dispatch_screenshot_native_rejects_relative_output_path() {
        // The native method must reject a non-absolute `output_path` before
        // any platform capture work — this works the same on every host.
        let engine = EvalEngine::new();
        let params = json!({"window_id": 1_u32, "output_path": "relative/path.png"});
        let err = dispatch(
            "screenshot_native",
            Some(&params),
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await
        .expect_err("relative path must error before any capture");
        assert_eq!(err.code, -32602);
        let data = err.data.as_ref().expect("error data present");
        assert_eq!(
            data.get("error").and_then(|v| v.as_str()),
            Some("INVALID_OUTPUT_PATH")
        );
    }

    #[tokio::test]
    async fn test_dispatch_screenshot_routes_to_bridge_regardless_of_params() {
        // The bare `screenshot` JSON-RPC method always goes to the bridge
        // (html-to-image, base64) — even if a caller mistakenly includes an
        // `output_path` field, that is no longer a signal to dispatch to the
        // native handler. The two surfaces are wholly separate methods.
        let engine = EvalEngine::new();
        for params in [json!({}), json!({"output_path": "/tmp/x.png"})] {
            let result = dispatch(
                "screenshot",
                Some(&params),
                &engine,
                &FakeWebviews::default(),
                &Recorder::new(),
            )
            .await;
            let err = result.expect_err("dispatch returns Err");
            assert_eq!(err.code, -32603);
            assert!(err.message.contains("No webview"));
        }
    }

    #[tokio::test]
    async fn test_dispatch_record_start_returns_recording() {
        let engine = EvalEngine::new();
        let recorder = Recorder::new();
        let result = dispatch(
            "record.start",
            None,
            &engine,
            &FakeWebviews::default(),
            &recorder,
        )
        .await
        .expect("dispatch succeeds");
        assert_eq!(result["status"], "recording");
        assert!(recorder.is_active());
    }

    #[tokio::test]
    async fn test_dispatch_record_stop_returns_entries() {
        let engine = EvalEngine::new();
        let recorder = Recorder::new();
        recorder.start();
        recorder.record("click", Some(&json!({"ref": "e1"})));
        let result = dispatch(
            "record.stop",
            None,
            &engine,
            &FakeWebviews::default(),
            &recorder,
        )
        .await
        .expect("dispatch succeeds");
        assert_eq!(result["count"], 1);
        assert!(result["entries"].as_array().is_some());
        assert!(!recorder.is_active());
    }

    /// Issue #161: stopping without a recording must fail instead of handing
    /// the CLI an empty entry list to save as a successful capture.
    #[tokio::test]
    async fn test_dispatch_record_stop_without_start_errors() {
        let err = dispatch(
            "record.stop",
            None,
            &EvalEngine::new(),
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await
        .expect_err("record.stop without record.start must fail");
        assert_eq!(err.code, RPC_INVALID_PARAMS);
        assert!(
            err.message.contains("No recording in progress"),
            "{}",
            err.message
        );
    }

    /// Issue #161: a second stop fails the same way instead of saving `[]`.
    #[tokio::test]
    async fn test_dispatch_second_record_stop_errors() {
        let engine = EvalEngine::new();
        let webviews = FakeWebviews::default();
        let recorder = Recorder::new();
        for method in ["record.start", "record.stop"] {
            dispatch(method, None, &engine, &webviews, &recorder)
                .await
                .expect("dispatch succeeds");
        }
        let err = dispatch("record.stop", None, &engine, &webviews, &recorder)
            .await
            .expect_err("second record.stop must fail");
        assert_eq!(err.code, RPC_INVALID_PARAMS);
    }

    #[tokio::test]
    async fn test_dispatch_record_status() {
        let engine = EvalEngine::new();
        let recorder = Recorder::new();
        recorder.start();
        let result = dispatch(
            "record.status",
            None,
            &engine,
            &FakeWebviews::default(),
            &recorder,
        )
        .await
        .expect("dispatch succeeds");
        assert_eq!(result["active"], true);
        assert_eq!(result["count"], 0);
    }

    #[tokio::test]
    async fn test_dispatch_record_add_entry() {
        let engine = EvalEngine::new();
        let recorder = Recorder::new();
        recorder.start();
        let params = json!({"action": "navigate", "timestamp": 100, "url": "/home"});
        let result = dispatch(
            "record.add",
            Some(&params),
            &engine,
            &FakeWebviews::default(),
            &recorder,
        )
        .await
        .expect("dispatch succeeds");
        assert_eq!(result["status"], "ok");
        let entries = recorder.stop().expect("recording active");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "navigate");
    }

    // ─── bridge_eval_timeout (issue #91) ─────────────────────────────────────

    #[test]
    fn test_bridge_eval_timeout_uses_param_plus_buffer() {
        // The Rust channel must outlive the JS timer so the bridge gets to
        // surface its own well-formed rejection (e.g.
        // `Timeout waiting for [data-testid="..."]`) instead of the channel
        // tripping first with the cryptic "eval timed out after 10s".
        let params = json!({"selector": "#root", "timeout": 60_000_u64});
        let got = bridge_eval_timeout(Some(&params));
        assert_eq!(
            got,
            Duration::from_millis(60_000 + BRIDGE_TIMEOUT_BUFFER_MS)
        );
    }

    #[test]
    fn test_bridge_eval_timeout_defaults_when_missing() {
        // Mirror the bridge's own fallback: 10_000 ms + buffer.
        let got = bridge_eval_timeout(None);
        assert_eq!(
            got,
            Duration::from_millis(DEFAULT_BRIDGE_TIMEOUT_MS + BRIDGE_TIMEOUT_BUFFER_MS)
        );
    }

    // ─── drag_eval_timeout ───────────────────────────────────────────────────

    #[test]
    fn test_drag_eval_timeout_covers_a_tuned_gesture() {
        // 60 moves 500 ms apart plus a 5 s settle is 35 s of JS timers. The old
        // flat 10 s channel expired mid-drag and dropped the pending result.
        let params = json!({"steps": 60, "stepDelayMs": 500, "settleMs": 5_000});
        let got = drag_eval_timeout(Some(&params));
        assert_eq!(
            got,
            Duration::from_millis(60 * 500 + 5_000 + BRIDGE_TIMEOUT_BUFFER_MS)
        );
    }

    #[test]
    fn test_drag_eval_timeout_keeps_the_default_floor() {
        // A default or fast gesture computes well under DEFAULT_TIMEOUT; the
        // channel must not shrink below it, since the bridge still has to
        // resolve elements and dispatch before the timers even start.
        assert_eq!(drag_eval_timeout(None), DEFAULT_TIMEOUT);
        let fast = json!({"steps": 2, "stepDelayMs": 0, "settleMs": 0});
        assert_eq!(drag_eval_timeout(Some(&fast)), DEFAULT_TIMEOUT);
    }

    #[test]
    fn test_drag_eval_timeout_clamps_steps_like_the_bridge() {
        // The bridge clamps steps to 1..=60, so a hostile count must not inflate
        // the channel timeout past what the gesture can actually take.
        let params = json!({"steps": 100_000, "stepDelayMs": 1_000, "settleMs": 0});
        let got = drag_eval_timeout(Some(&params));
        assert_eq!(
            got,
            Duration::from_millis(60 * 1_000 + BRIDGE_TIMEOUT_BUFFER_MS)
        );
    }

    #[test]
    fn test_drag_eval_timeout_treats_zero_steps_as_the_bridge_default() {
        // `bridge.js` maps steps < 1 to 12, so zero is 12 s of move delays here,
        // not one. Clamping to 1 would expire the channel mid-gesture.
        let params = json!({"steps": 0, "stepDelayMs": 1_000, "settleMs": 0});
        let got = drag_eval_timeout(Some(&params));
        assert_eq!(
            got,
            Duration::from_millis(12 * 1_000 + BRIDGE_TIMEOUT_BUFFER_MS)
        );
    }

    #[test]
    fn test_drag_eval_timeout_coerces_numeric_strings_like_the_bridge() {
        // `bridge.js` runs every tunable through `Number()`, so "1000" is a real
        // 1 s step delay over there. Reading it as absent here budgeted 10 s
        // against a 60 s gesture and timed the channel out mid-drag.
        let params = json!({"steps": "60", "stepDelayMs": "1000", "settleMs": "500"});
        let got = drag_eval_timeout(Some(&params));
        assert_eq!(
            got,
            Duration::from_millis(60 * 1_000 + 500 + BRIDGE_TIMEOUT_BUFFER_MS)
        );
    }

    #[test]
    fn test_drag_eval_timeout_coerces_radix_prefixed_strings() {
        // `Number("0x3e8")` is 1000, so the bridge really does wait a second
        // between moves here. Rejecting the literal would put the channel back
        // on its 10 s floor against a 60 s gesture.
        let params = json!({"steps": "0x3c", "stepDelayMs": "0X3E8", "settleMs": "0b0"});
        let got = drag_eval_timeout(Some(&params));
        assert_eq!(
            got,
            Duration::from_millis(60 * 1_000 + BRIDGE_TIMEOUT_BUFFER_MS)
        );
    }

    #[test]
    fn test_drag_eval_timeout_clamps_a_wide_radix_step_count() {
        // A literal past any integer width still floors to the bridge's 60-step
        // clamp, not back to its default of 12 — the difference is 60 s of
        // gesture against a 12 s budget.
        let params = json!({"steps": "0xFFFFFFFFFF", "stepDelayMs": 1_000, "settleMs": 0});
        let got = drag_eval_timeout(Some(&params));
        assert_eq!(
            got,
            Duration::from_millis(60 * 1_000 + BRIDGE_TIMEOUT_BUFFER_MS)
        );
    }

    #[test]
    fn test_drag_eval_timeout_rejects_a_bare_radix_prefix() {
        // `Number("0x")` is NaN, so the bridge uses its default and so must the
        // budget.
        let params = json!({"steps": "0x", "stepDelayMs": "0b2", "settleMs": 0});
        assert_eq!(drag_eval_timeout(Some(&params)), DEFAULT_TIMEOUT);
    }

    #[test]
    fn test_drag_eval_timeout_honors_fractional_delays() {
        // Fractional delays are legal in JS and only `steps` gets floored, so
        // the budget has to follow the same rules or a `stepDelayMs: 900.5`
        // gesture outlives its channel.
        let params = json!({"steps": 60.7, "stepDelayMs": 900.5, "settleMs": 0});
        let got = drag_eval_timeout(Some(&params));
        assert_eq!(
            got,
            // 60 steps of 900.5 ms.
            Duration::from_millis(54_030 + BRIDGE_TIMEOUT_BUFFER_MS)
        );
    }

    #[test]
    fn test_drag_eval_timeout_ignores_hostile_values() {
        // Non-numeric or negative tunables fall back to the bridge defaults
        // instead of panicking or coercing to something absurd.
        let params = json!({"steps": "lots", "stepDelayMs": -5, "settleMs": null});
        assert_eq!(drag_eval_timeout(Some(&params)), DEFAULT_TIMEOUT);
    }

    #[test]
    fn test_bridge_eval_timeout_defaults_when_param_not_u64() {
        // A non-integer "timeout" (string, negative, float) must not panic and
        // must fall back to the bridge default rather than silently coercing.
        let params = json!({"timeout": "soon"});
        let got = bridge_eval_timeout(Some(&params));
        assert_eq!(
            got,
            Duration::from_millis(DEFAULT_BRIDGE_TIMEOUT_MS + BRIDGE_TIMEOUT_BUFFER_MS)
        );
    }

    #[test]
    fn test_bridge_eval_timeout_saturates_on_overflow() {
        // u64::MAX + buffer must saturate, not wrap to a tiny value.
        let params = json!({"timeout": u64::MAX});
        let got = bridge_eval_timeout(Some(&params));
        assert_eq!(got, Duration::from_millis(u64::MAX));
    }

    #[test]
    fn test_bridge_eval_timeout_zero_still_padded() {
        // A user-supplied 0 ms timeout still gets the buffer — the buffer is a
        // *ceiling* for stuck JS, not a per-call cost. JS receives `timeout: 0`
        // and rejects on the next microtask; the buffer never elapses on the
        // happy path.
        let got = bridge_eval_timeout(Some(&json!({"timeout": 0_u64})));
        assert_eq!(got, Duration::from_millis(BRIDGE_TIMEOUT_BUFFER_MS));
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_wait_honors_user_timeout_above_default() {
        // Issue #91: with the previous wiring, `wait --timeout 30000` was
        // capped at `DEFAULT_TIMEOUT` (10 s) by the Rust channel and surfaced
        // as "Eval error: eval timed out after 10s" — masking the real bridge
        // outcome. After the fix, the Rust side must wait
        // `30_000 + BRIDGE_TIMEOUT_BUFFER_MS` ms before giving up.
        //
        // Runs under `start_paused = true`, so virtual time auto-advances to
        // whatever timer the dispatch is parked on. The elapsed value reflects
        // the *effective* Rust-side cap.
        let engine = EvalEngine::new();
        // A webview that accepts the script but never resolves the callback —
        // the timeout decides who wins.
        let webviews = FakeWebviews::window("main", None);

        let params = json!({
            "selector": "[data-testid=\"never-exists\"]",
            "timeout": 30_000_u64,
        });

        let start = tokio::time::Instant::now();
        let err = dispatch("wait", Some(&params), &engine, &webviews, &Recorder::new())
            .await
            .expect_err("dispatch must time out");
        let elapsed = start.elapsed();

        // The behavioral invariant being defended: the Rust channel must
        // outlive the user's JS-side timeout. Pre-fix it expired at
        // `DEFAULT_TIMEOUT` (10 s), well before the 30 s user_timeout.
        let user_timeout = Duration::from_secs(30);
        assert!(
            elapsed > user_timeout,
            "elapsed {elapsed:?} should outlive user timeout {user_timeout:?}"
        );
        assert_eq!(err.code, -32603);
        assert!(
            err.message.contains("timed out"),
            "unexpected error message: {}",
            err.message
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_wait_default_timeout_outlives_bridge_default() {
        // Without `timeout` in params the helper falls back to the bridge
        // default. The Rust channel must still outlive that default so the
        // bridge gets to surface its own `Timeout waiting for …` rejection.
        let engine = EvalEngine::new();
        let webviews = FakeWebviews::window("main", None);

        let start = tokio::time::Instant::now();
        let _err = dispatch(
            "wait",
            Some(&json!({"selector": "#root"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("dispatch must time out");
        let elapsed = start.elapsed();
        let bridge_default = Duration::from_millis(DEFAULT_BRIDGE_TIMEOUT_MS);
        assert!(
            elapsed > bridge_default,
            "elapsed {elapsed:?} should outlive bridge default {bridge_default:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_watch_still_outlives_user_timeout() {
        // Regression guard: `wait` and `watch` share the helper, so the
        // existing `watch` behavior must remain intact.
        let engine = EvalEngine::new();
        let webviews = FakeWebviews::window("main", None);

        let start = tokio::time::Instant::now();
        let _err = dispatch(
            "watch",
            Some(&json!({"timeout": 25_000_u64})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("dispatch must time out");
        let elapsed = start.elapsed();
        let user_timeout = Duration::from_secs(25);
        assert!(
            elapsed > user_timeout,
            "elapsed {elapsed:?} should outlive user timeout {user_timeout:?}"
        );
    }

    #[tokio::test]
    async fn test_dispatch_wait_returns_callback_value_before_timeout() {
        // Happy path: the bridge resolves the callback well before the padded
        // timeout. The dispatch must return that value rather than the
        // channel error — the buffer is a ceiling, not a per-call cost.
        let engine = EvalEngine::new();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", None).on_eval(move || {
            // The first registered callback on a fresh engine has id == 1
            // (see EvalEngine::register / next_id init in eval.rs).
            engine_clone.resolve(1, Ok(json!({"found": true})));
        });

        let result = dispatch(
            "wait",
            Some(&json!({"selector": "#root", "timeout": 60_000_u64})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("dispatch should resolve via callback, not time out");
        assert_eq!(result, json!({"found": true}));
    }

    #[tokio::test]
    async fn test_dispatch_state_injects_plugin_version() {
        // `state` returns url/title/ready from the bridge, then the dispatch
        // merges in the plugin's own version so callers can spot version drift
        // from a single `state` call (issue #135).
        let engine = EvalEngine::new();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", None).on_eval(move || {
            // First register on a fresh engine has id == 1 (see eval.rs).
            engine_clone.resolve(
                1,
                Ok(json!({"url": "http://localhost/", "title": "App", "ready": true})),
            );
        });

        let result = dispatch("state", None, &engine, &webviews, &Recorder::new())
            .await
            .expect("dispatch succeeds");

        assert_eq!(result["url"], json!("http://localhost/"));
        assert_eq!(result["ready"], json!(true));
        assert_eq!(result["plugin_version"], json!(env!("CARGO_PKG_VERSION")));
    }

    const APP_PAGE: &str = "tauri://localhost/";
    const FOREIGN_PAGE: &str = "https://example.com/";

    fn url(text: &str) -> tauri::Url {
        tauri::Url::parse(text).expect("valid test URL")
    }

    /// Engine whose bridge already said hello from the app origin, as it does
    /// when the app page loads.
    fn engine_with_app_bridge() -> EvalEngine {
        let engine = EvalEngine::new();
        handle_callback(&engine, HELLO_ID, None, None, Some(&url(APP_PAGE)));
        engine
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_to_foreign_origin_does_not_report_ok() {
        // #153: the bridge on the app page answers `{ok: true}` before the
        // webview leaves for an origin whose bridge cannot call back, so
        // navigate used to report success and leave the session broken.
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE))
            .on_eval(move || engine_clone.resolve(1, Ok(json!({"ok": true}))));

        let start = tokio::time::Instant::now();
        let err = dispatch(
            "navigate",
            Some(&json!({"url": FOREIGN_PAGE})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("navigate to a foreign origin must not report ok");

        assert!(
            start.elapsed() >= BRIDGE_GRACE,
            "must wait the 3s grace, took {:?}",
            start.elapsed()
        );
        assert!(
            start.elapsed() < DEFAULT_TIMEOUT,
            "took {:?}",
            start.elapsed()
        );
        assert_eq!(err.code, -32603);
        assert!(err.message.contains(FOREIGN_PAGE), "got: {}", err.message);
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_on_page_without_bridge_fails_fast_naming_origins() {
        // #153: every bridge command used to hang for DEFAULT_TIMEOUT once the
        // webview sat on a foreign origin.
        let engine = engine_with_app_bridge();
        let webviews = FakeWebviews::window("main", Some(FOREIGN_PAGE));

        let start = tokio::time::Instant::now();
        let err = dispatch("title", None, &engine, &webviews, &Recorder::new())
            .await
            .expect_err("a page without a bridge cannot answer");

        assert!(
            start.elapsed() < Duration::from_secs(1),
            "took {:?}",
            start.elapsed()
        );
        assert_eq!(err.code, -32603);
        assert!(err.message.contains(FOREIGN_PAGE), "got: {}", err.message);
        assert!(
            err.message.contains("tauri://localhost"),
            "got: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn test_dispatch_on_page_without_bridge_runs_no_script() {
        // The page cannot report a result, so a click there must not act on
        // it either.
        let engine = engine_with_app_bridge();
        let webviews = FakeWebviews::window("main", Some(FOREIGN_PAGE));
        let params = json!({"ref": "e1"});
        dispatch("click", Some(&params), &engine, &webviews, &Recorder::new())
            .await
            .expect_err("a page without a bridge cannot answer");
        assert_eq!(webviews.scripts(), Vec::<String>::new());
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_back_to_app_origin_waits_for_bridge_hello() {
        // #153: the way out of a foreign page is navigating back. The foreign
        // page cannot call back, so success is the app bridge's hello.
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(FOREIGN_PAGE)).on_eval(move || {
            let engine = engine_clone.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(500)).await;
                handle_callback(&engine, HELLO_ID, None, None, Some(&url(APP_PAGE)));
            });
        });

        let result = dispatch(
            "navigate",
            Some(&json!({"url": APP_PAGE})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("navigate back to the app origin must succeed");
        assert_eq!(result, json!({"ok": true}));
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_within_app_origin_returns_bridge_result() {
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE))
            .on_eval(move || engine_clone.resolve(1, Ok(json!({"ok": true}))));

        let result = dispatch(
            "navigate",
            Some(&json!({"url": "/settings"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("same-origin navigate succeeds");
        assert_eq!(result, json!({"ok": true}));
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_to_new_origin_succeeds_when_hello_arrives() {
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let dest = "https://allowed.example/";
        let webviews = FakeWebviews::window("main", Some(APP_PAGE)).on_eval(move || {
            let engine = engine_clone.clone();
            engine.resolve(1, Ok(json!({"ok": true})));
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                handle_callback(&engine, HELLO_ID, None, None, Some(&url(dest)));
            });
        });

        let start = tokio::time::Instant::now();
        let result = dispatch(
            "navigate",
            Some(&json!({"url": dest})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("first visit must succeed when the dest hellos in time");
        assert_eq!(result, json!({"ok": true}));
        assert!(
            start.elapsed() < DEFAULT_TIMEOUT,
            "took {:?}",
            start.elapsed()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_javascript_url_stays_on_page() {
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE))
            .on_eval(move || engine_clone.resolve(1, Ok(json!({"ok": true}))));

        let start = tokio::time::Instant::now();
        let result = dispatch(
            "navigate",
            Some(&json!({"url": "javascript:void(0)"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("javascript: navigate stays on the current page");
        assert_eq!(result, json!({"ok": true}));
        assert!(
            start.elapsed() < BRIDGE_GRACE,
            "must not wait the dest grace, took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn test_hello_uses_webview_url_not_client_payload() {
        let engine = engine_with_app_bridge();
        handle_callback(
            &engine,
            HELLO_ID,
            Some("https://evil.example/".to_owned()),
            None,
            Some(&url(APP_PAGE)),
        );
        assert!(engine.has_bridge(&url(APP_PAGE)));
        assert!(
            !engine.has_bridge(&url("https://evil.example/")),
            "client-supplied href must not be recorded as a hello"
        );
    }

    #[test]
    fn test_hello_dropped_when_webview_url_does_not_match_payload() {
        let engine = engine_with_app_bridge();
        handle_callback(
            &engine,
            HELLO_ID,
            Some(APP_PAGE.to_owned()),
            None,
            Some(&url(FOREIGN_PAGE)),
        );
        assert!(engine.has_bridge(&url(APP_PAGE)));
        assert!(
            !engine.has_bridge(&url(FOREIGN_PAGE)),
            "a stale hello must not tag the page the webview already left"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_succeeds_when_page_callback_times_out_but_dest_hellos() {
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let dest = "https://allowed.example/";
        let webviews = FakeWebviews::window("main", Some(APP_PAGE)).on_eval(move || {
            let engine = engine_clone.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                handle_callback(&engine, HELLO_ID, None, None, Some(&url(dest)));
            });
        });

        let start = tokio::time::Instant::now();
        let result = dispatch(
            "navigate",
            Some(&json!({"url": dest})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("dest hello must still succeed if the old page never callbacks");
        assert_eq!(result, json!({"ok": true}));
        assert!(
            start.elapsed() < DEFAULT_TIMEOUT,
            "took {:?}",
            start.elapsed()
        );
    }
}
