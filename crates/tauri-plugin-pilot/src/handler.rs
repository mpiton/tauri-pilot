use crate::diff;
use crate::eval::{EvalEngine, EvalError, HELLO_ID, ReplyReceiver, origin_key};
#[cfg(feature = "press")]
use crate::key;
use crate::protocol::{RPC_INTERNAL_ERROR, RPC_INVALID_PARAMS, RpcError};
use crate::recorder::{RecordEntry, Recorder, unstable_steps};
use crate::screenshot;
use crate::webview::{TargetWindow, Webviews};

use std::time::Duration;
#[cfg(feature = "press")]
use tokio::sync::Mutex as AsyncMutex;

/// How long `press` waits for the target window to actually gain OS focus
/// after `set_focus`. `set_focus` only reports that the activation request
/// was dispatched; on X11, focus-stealing prevention can ignore it and still
/// return success. Tuned empirically — too short and a legitimate focus
/// transfer is reported as failure; too long and a refused focus delays the
/// error. Polling interval is [`FOCUS_POLL_MS`].
#[cfg(feature = "press")]
const FOCUS_SETTLE_MS: u64 = 80;

/// Pause between `is_focused` polls while waiting for [`FOCUS_SETTLE_MS`].
/// Short enough to notice focus inside the budget, long enough not to spin.
#[cfg(feature = "press")]
const FOCUS_POLL_MS: u64 = 5;

/// Serializes the full `focus → confirm → inject` sequence across concurrent
/// `press` calls. The inner `key::PRESS_LOCK` only covers the OS injection,
/// so without this outer lock two calls targeting different windows could
/// race on the focus step and deliver both keys to whichever window won the
/// focus race.
#[cfg(feature = "press")]
static PRESS_ORDER_LOCK: AsyncMutex<()> = AsyncMutex::const_new(());

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
/// Budget of the `locate` call made before each recorded ref step (#276).
///
/// The lookup is best effort: when it times out the step is saved ref-only
/// and `record stop` reports it. Kept short so a slow or absent bridge
/// cannot add [`DEFAULT_TIMEOUT`] to every recorded action; a healthy
/// bridge answers in milliseconds.
const LOCATE_TIMEOUT: Duration = Duration::from_secs(2);
/// Longest fixed bound. The CLI's `DEFAULT_RPC_TIMEOUT` in
/// `crates/tauri-pilot-cli/src/client/mod.rs` sits above it so the CLI never
/// gives up first. The crates ship separately, so change both together.
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
///
/// The CLI's `rpc_budget` in `crates/tauri-pilot-cli/src/client/mod.rs`
/// adds the same `timeout` to its own deadline; change both together.
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
///
/// The CLI's `rpc_budget` in `crates/tauri-pilot-cli/src/client/mod.rs`
/// adds this gesture time to its own deadline; change both together.
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
    // The recorder keeps "window" on the entry, so it gets the params before
    // window extraction.
    let original_params = params.cloned();

    let (window, owned_params) = extract_window(params);
    let params = owned_params.as_ref().or(params);
    let win = window.as_deref();

    let locators = locate_for_recording(method, params, engine, webviews, win, recorder).await;

    let result = match method {
        "ping" => {
            let mut result = serde_json::json!({"status": "ok"});
            inject_plugin_version(&mut result);
            Ok(result)
        }
        "windows.list" => Ok(serde_json::json!({"windows": webviews.list()})),
        "snapshot" => {
            let result = capture_snapshot(&capture_options(params)?, engine, webviews, win).await?;
            engine.store_snapshot(&result);
            Ok(result)
        }
        "diff" => handle_diff(params, engine, webviews, win).await,
        // Checked before the feature arms: no feature flag makes `press` work
        // on mobile, and with it on the focus check always fails there (#256).
        "press" if cfg!(any(target_os = "android", target_os = "ios")) => {
            Err(press_unsupported_error())
        }
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
        | "value" | "attrs" | "eval" | "ipc" | "title" | "visible" | "count" | "checked" => {
            handle_eval_method(method, params, engine, webviews, win, DEFAULT_TIMEOUT).await
        }
        // `url` comes from the runtime, not the bridge: the webview knows its
        // own URL, so the answer survives a page whose bridge cannot call back
        // (a foreign origin the ACL denies, #153) — exactly when the caller is
        // lost and needs it (#233). `windows.list` already reads it this way.
        "url" => handle_url(webviews, win).await,
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
        "storage.delete" => {
            handle_eval_method(
                "storageDelete",
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
            let unstable = unstable_steps(&entries);
            Ok(serde_json::json!({"entries": entries, "count": count, "unstable": unstable}))
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
        recorder.record(method, original_params.as_ref(), locators.as_ref());
    }

    result
}

/// Resolves the refs of a step being recorded to stable locators (#276).
///
/// Runs before the action, since a click can navigate or remove its element,
/// and only while recording. `None` when there is nothing to locate or the
/// lookup failed: the step stays ref-only and `record stop` reports it.
async fn locate_for_recording(
    method: &str,
    params: Option<&serde_json::Value>,
    engine: &EvalEngine,
    webviews: &dyn Webviews,
    window: Option<&str>,
    recorder: &Recorder,
) -> Option<serde_json::Value> {
    let request = recorder.locate_request(method, params)?;
    let located = handle_eval_method(
        "locate",
        Some(&request),
        engine,
        webviews,
        window,
        LOCATE_TIMEOUT,
    );
    located.await.ok()
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
    let options = capture_options(params)?;
    let warning = check_reference_options(&reference, &options)?;
    let result = capture_snapshot(&options, engine, webviews, window).await?;

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

    let mut value = serde_json::to_value(&diff_result).map_err(|e| RpcError {
        code: -32603,
        message: format!("Serialization error: {e}"),
        data: None,
    })?;
    if let Some(warning) = warning {
        value["warning"] = serde_json::Value::from(warning);
    }
    Ok(value)
}

/// Reads the capture options of a `snapshot` or `diff` request.
fn capture_options(params: Option<&serde_json::Value>) -> Result<diff::CaptureOptions, RpcError> {
    diff::CaptureOptions::from_params(params).map_err(|message| RpcError {
        code: RPC_INVALID_PARAMS,
        message,
        data: None,
    })
}

/// Takes a snapshot with `options` and records them in the result.
///
/// The bridge gets the normalized options, not the raw params, so the
/// recorded options are the ones the tree was captured with and `diff` can
/// check them (#244).
async fn capture_snapshot(
    options: &diff::CaptureOptions,
    engine: &EvalEngine,
    webviews: &dyn Webviews,
    window: Option<&str>,
) -> Result<serde_json::Value, RpcError> {
    let params = serde_json::json!(options);
    let mut result = handle_eval_method(
        "snapshot",
        Some(&params),
        engine,
        webviews,
        window,
        DEFAULT_TIMEOUT,
    )
    .await?;
    if let Some(obj) = result.as_object_mut() {
        obj.insert("options".into(), params);
    }
    Ok(result)
}

/// Refuses a diff whose reference was captured with other options (#244).
///
/// Returns a warning instead when the reference predates recorded options.
fn check_reference_options(
    reference: &serde_json::Value,
    current: &diff::CaptureOptions,
) -> Result<Option<&'static str>, RpcError> {
    let recorded = diff::recorded_options(reference).map_err(|message| RpcError {
        code: RPC_INVALID_PARAMS,
        message,
        data: None,
    })?;
    let Some(recorded) = recorded else {
        return Ok(Some(diff::UNRECORDED_OPTIONS_WARNING));
    };
    match diff::options_mismatch(&recorded, current) {
        Some(message) => Err(RpcError {
            code: RPC_INVALID_PARAMS,
            message,
            data: Some(serde_json::json!({"reference": recorded, "current": current})),
        }),
        None => Ok(None),
    }
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

    // With no window to focus, the key would land in whatever app has focus.
    // Wait for the first window before the lock: without a label this can
    // take up to FIRST_WINDOW_BUDGET (#273), and holding the lock meanwhile
    // would stall a concurrent `--window` press for that long.
    target(webviews, window).await?;

    // Hold this lock across the whole focus → confirm → inject sequence so
    // two concurrent `press` calls cannot interleave their focus steps (call
    // A focuses window X, call B focuses window Y, then both keys land on Y).
    let _order_guard = PRESS_ORDER_LOCK.lock().await;

    // Resolve again, without waiting: the window may have closed or been
    // recreated while this press queued behind another one.
    let target = target_within(webviews, window, Duration::ZERO).await?;

    if let Err(e) = target.focus() {
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
    // `focus()` succeeding only means the activation request was dispatched.
    // Poll until the window actually has OS focus, otherwise keys go elsewhere.
    wait_until_focused(target).await?;

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

/// Poll until `target` reports OS focus, or [`FOCUS_SETTLE_MS`] elapses.
///
/// Takes the window by value so the future stays `Send` without requiring
/// `TargetWindow: Sync` (the box is owned across each sleep). A failed
/// focus query returns immediately instead of looking like another app won.
#[cfg(feature = "press")]
async fn wait_until_focused(target: Box<dyn TargetWindow + '_>) -> Result<(), RpcError> {
    let budget = Duration::from_millis(FOCUS_SETTLE_MS);
    let poll = Duration::from_millis(FOCUS_POLL_MS);
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        match target.is_focused() {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(e) => {
                return Err(RpcError {
                    code: RPC_INTERNAL_ERROR,
                    message: format!(
                        "cannot press: failed to query focus for window '{}': {e}",
                        target.label()
                    ),
                    data: None,
                });
            }
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(press_unfocused_error(target.label()));
        }
        let step = poll.min(remaining);
        tokio::time::sleep(step).await;
    }
}

/// Error when `press` would type into whichever app currently has focus.
#[cfg(feature = "press")]
fn press_unfocused_error(label: &str) -> RpcError {
    RpcError {
        code: RPC_INTERNAL_ERROR,
        message: format!(
            "cannot press: window '{label}' did not gain focus (another application has it)"
        ),
        data: None,
    }
}

/// Error for `press` on Android and iOS, which have no OS keyboard backend.
///
/// `data.error` carries the same `UNSUPPORTED_PLATFORM` code as
/// `screenshot_native` off macOS, so callers match one code for both.
fn press_unsupported_error() -> RpcError {
    RpcError {
        code: RPC_INTERNAL_ERROR,
        message: "press is not supported on Android/iOS (use `fill` or `type` for text input)"
            .to_owned(),
        data: Some(serde_json::json!({"error": screenshot::ipc::codes::UNSUPPORTED_PLATFORM})),
    }
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
/// destination (#153, #260).
///
/// The bridge answers before the webview leaves the page, so the callback
/// alone proves nothing about the destination. Whenever the navigation loads
/// a new document, the hello of that document is the proof, even when its
/// origin already has a bridge. A `javascript:` URL or a fragment change
/// stays in the current document and resolves on the callback. A navigate
/// sent before the first hello waits for the page's callback first, then
/// for the destination's hello the same way: see [`navigate_at_startup`].
async fn handle_navigate(
    params: Option<&serde_json::Value>,
    engine: &EvalEngine,
    webviews: &dyn Webviews,
    window: Option<&str>,
) -> Result<serde_json::Value, RpcError> {
    let since = engine.hellos();
    let target = target(webviews, window).await?;
    // Read before the eval so dest is resolved against the page the
    // script was aimed at, not one a concurrent navigation already left.
    let page = target.url();
    let raw = params.and_then(|p| p.get("url").and_then(serde_json::Value::as_str));
    let dest = navigate_destination(page.as_ref(), raw);
    let script = navigate_eval_script(params, dest.as_ref()).map_err(|msg| RpcError {
        code: -32602,
        message: msg,
        data: None,
    })?;
    let (id, rx) = send_script(&script, engine, target.as_ref(), None)?;

    match (page.as_ref(), dest.as_ref()) {
        (Some(page), dest) if since == 0 => {
            navigate_at_startup(engine, (id, rx), target, page, dest, raw).await
        }
        (Some(page), Some(dest)) if origin_key(page) == origin_key(dest) => {
            let slot = (id, rx);
            navigate_same_origin(engine, slot, target, page, dest, raw, since).await
        }
        (Some(page), Some(dest)) => {
            if engine.has_bridge(page) {
                await_departure(engine, id, rx).await?;
            } else {
                // The script still navigates, but this page cannot call back.
                engine.resolve(id, Err("page has no pilot bridge".to_owned()));
            }
            wait_dest_bridge(engine, target, Some(page), dest, since).await
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
            // No start URL: a window left on a bridged origin with no new
            // hello still counts as never left (see `never_left`).
            wait_dest_bridge(engine, target, None, dest, since).await
        }
        (None, None) => wait(engine, id, rx, DEFAULT_TIMEOUT).await,
    }
}

/// Finish a navigate sent before the first hello, as at app startup (#270).
///
/// Before the first hello the engine cannot know a hello will ever come.
/// By the time the page calls back it usually can: `eval` only runs in a
/// document whose init script already ran, and the bridge sends its hello
/// at the end of that script. The hello is not awaited, so it is sent
/// before the callback but nothing guarantees it is recorded first. Once
/// the callback lands with a hello recorded, a navigate that loads a new
/// document waits for the target window's hello like any later navigate:
/// up to [`DEFAULT_TIMEOUT`] on an origin that already said hello, up to
/// [`BRIDGE_GRACE`] on one that never did, then fails (#153). Without a
/// recorded hello by then (the hello is still in flight or was dropped) or
/// without a new document, the callback settles it, as before #270.
///
/// # Errors
///
/// Fails when the page reports an eval error or never calls back, or when
/// the destination does not say hello in time.
async fn navigate_at_startup(
    engine: &EvalEngine,
    (id, rx): CallbackSlot,
    target: Box<dyn TargetWindow + '_>,
    page: &tauri::Url,
    dest: Option<&tauri::Url>,
    raw: Option<&str>,
) -> Result<serde_json::Value, RpcError> {
    // The baseline is the hello count when the callback was recorded, not
    // when this task wakes up. The destination can say hello in between,
    // and a baseline read after it would hide that hello until the timeout.
    // This assumes the departing page's callback is recorded before the
    // destination's hello. They are two IPC requests from two documents and
    // nothing orders them, but in practice the callback lands first (about
    // 660 ms after the navigate in #270, well before the destination loads).
    // If the destination's hello is recorded first, the wait does not see it
    // and navigate fails after the timeout: a false error, never a false ok.
    let (value, hellos_at_callback) = engine
        .wait_with_hellos(id, rx, DEFAULT_TIMEOUT)
        .await
        .map_err(|e| eval_rpc_error(&e))?;
    let Some(dest) = dest else {
        return Ok(value);
    };
    if hellos_at_callback == 0 || !loads_new_document(page, dest, raw) {
        Ok(value)
    } else if origin_key(page) == origin_key(dest) {
        wait_same_origin_hello(engine, target, page, dest, hellos_at_callback).await
    } else {
        wait_dest_bridge(engine, target, Some(page), dest, hellos_at_callback).await
    }
}

/// Finish a navigate whose destination shares the page's origin.
///
/// A new document must say hello after `since` (#260). A fragment change or
/// a `javascript:` URL keeps the document, so those resolve on the page's
/// callback. The caller handles `since == 0`.
///
/// # Errors
///
/// Fails when the page has no bridge, the page reports an eval error, or
/// no hello arrives from the new document in time.
async fn navigate_same_origin(
    engine: &EvalEngine,
    (id, rx): CallbackSlot,
    target: Box<dyn TargetWindow + '_>,
    page: &tauri::Url,
    dest: &tauri::Url,
    raw: Option<&str>,
    since: u64,
) -> Result<serde_json::Value, RpcError> {
    if !engine.has_bridge(page) {
        Err(fail_no_bridge(engine, id, page))
    } else if loads_new_document(page, dest, raw) {
        await_departure(engine, id, rx).await?;
        wait_same_origin_hello(engine, target, page, dest, since).await
    } else {
        wait(engine, id, rx, DEFAULT_TIMEOUT).await
    }
}

/// Wait for the new document of a same-origin navigate to say hello.
///
/// A hello from the target window on `dest`'s origin after `since` is the
/// proof. A server redirect can land on another origin whose bridge also
/// works, so a hello after `since` from the origin the window now shows
/// counts too. Hellos from other windows on the same origin do not count.
///
/// # Errors
///
/// Fails after [`DEFAULT_TIMEOUT`] with an error that tells a navigation
/// that never happened from a document whose bridge stayed silent.
async fn wait_same_origin_hello(
    engine: &EvalEngine,
    target: Box<dyn TargetWindow + '_>,
    page: &tauri::Url,
    dest: &tauri::Url,
    since: u64,
) -> Result<serde_json::Value, RpcError> {
    let deadline = tokio::time::Instant::now() + DEFAULT_TIMEOUT;
    let label = target.label();
    loop {
        // Read the count before checking, so a hello landing in between
        // wakes the wait below instead of being missed.
        let seen = engine.hellos();
        let now = target.url();
        let redirected_hello = now
            .as_ref()
            .is_some_and(|now| now != page && engine.said_hello_since(label, now, since));
        if engine.said_hello_since(label, dest, since) || redirected_hello {
            return Ok(serde_json::json!({"ok": true}));
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() || !engine.wait_hello_after(seen, remaining).await {
            return Err(same_origin_silence_error(page, dest, target.url().as_ref()));
        }
    }
}

/// Error for a same-origin navigate whose new document never said hello.
///
/// The page's origin is known to answer, so the "navigate back or allow
/// this origin in `remote.urls`" hint of [`no_bridge_error`] does not apply.
fn same_origin_silence_error(
    page: &tauri::Url,
    dest: &tauri::Url,
    now: Option<&tauri::Url>,
) -> RpcError {
    let message = match now {
        Some(now) if now == page && dest != page => format!(
            "navigate to {dest} did not happen: the window still shows {page} after \
             {DEFAULT_TIMEOUT:?}, so the navigation was cancelled or blocked"
        ),
        now => format!(
            "navigate to {dest} loaded a new document ({}), but its pilot bridge \
             never said hello within {DEFAULT_TIMEOUT:?}",
            now.unwrap_or(dest)
        ),
    };
    RpcError {
        code: -32603,
        message,
        data: None,
    }
}

/// Build the navigate eval, using the already-absolute `dest`.
///
/// Relative urls cannot resolve against a page that moved. A `javascript:`
/// url still runs in the document that evals it; `dest` is that page for
/// waiting, not the href to assign.
fn navigate_eval_script(
    params: Option<&serde_json::Value>,
    dest: Option<&tauri::Url>,
) -> Result<String, String> {
    let raw = params.and_then(|p| p.get("url").and_then(serde_json::Value::as_str));
    let javascript = raw.is_some_and(is_javascript_url);
    let url = if javascript {
        raw.unwrap_or_default().to_owned()
    } else if let Some(dest) = dest {
        dest.to_string()
    } else {
        raw.unwrap_or_default().to_owned()
    };
    build_bridge_call("navigate", Some(&serde_json::json!({ "url": url })))
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

/// Whether navigating from `page` to `dest` replaces the document.
///
/// A `javascript:` URL runs in the current document. A URL equal to `page`
/// but for a fragment, with a fragment of its own, only scrolls (the HTML
/// "navigate to a fragment" case). Anything else, reloads included, loads
/// a new document whose bridge has to say hello again.
fn loads_new_document(page: &tauri::Url, dest: &tauri::Url, raw: Option<&str>) -> bool {
    if raw.is_some_and(is_javascript_url) {
        return false;
    }
    let fragment_only =
        dest.fragment().is_some() && without_fragment(page) == without_fragment(dest);
    !fragment_only
}

/// `url` with its fragment removed.
fn without_fragment(url: &tauri::Url) -> tauri::Url {
    let mut url = url.clone();
    url.set_fragment(None);
    url
}

/// Whether `raw` parses as a `javascript:` URL.
fn is_javascript_url(raw: &str) -> bool {
    tauri::Url::parse(raw).is_ok_and(|url| url.scheme() == "javascript")
}

/// Wait for the departing page's navigate callback.
///
/// The page may be torn down before `__callback` runs, so a timeout is not
/// fatal: the destination's hello decides. Any other eval error is.
///
/// # Errors
///
/// Returns the eval error the departing page reported.
async fn await_departure(engine: &EvalEngine, id: u64, rx: ReplyReceiver) -> Result<(), RpcError> {
    match engine.wait(id, rx, BRIDGE_GRACE).await {
        Ok(_) | Err(EvalError::Timeout(_)) => Ok(()),
        Err(e) => Err(eval_rpc_error(&e)),
    }
}

/// Wait for the target window's hello from `dest`.
///
/// The wait lasts [`DEFAULT_TIMEOUT`] when `dest`'s origin already said
/// hello, [`BRIDGE_GRACE`] on a first visit. A redirect can land the window
/// on another origin whose bridge works, so a hello after `since` from the
/// page the window now shows counts too, unless that page is `page` (a
/// reload of the start page proves nothing about `dest`), as in
/// [`wait_same_origin_hello`].
/// `page` is the URL the navigate started from, when known.
///
/// # Errors
///
/// Fails when no hello arrives in time. If the window never left its
/// document (see [`never_left`]), the error says the navigation did not
/// load in time (a refused connection keeps the old document, #278) or,
/// for a local destination (see [`is_local_scheme`]), that it was
/// cancelled or blocked (#310).
/// Otherwise it says no bridge answered on the page the window shows,
/// naming it when it is not `dest`.
async fn wait_dest_bridge(
    engine: &EvalEngine,
    target: Box<dyn TargetWindow + '_>,
    page: Option<&tauri::Url>,
    dest: &tauri::Url,
    since: u64,
) -> Result<serde_json::Value, RpcError> {
    let limit = if engine.has_bridge(dest) {
        DEFAULT_TIMEOUT
    } else {
        BRIDGE_GRACE
    };
    let deadline = tokio::time::Instant::now() + limit;
    let label = target.label();
    loop {
        // Read the count before checking, so a hello landing in between
        // wakes the wait below instead of being missed.
        let seen = engine.hellos();
        let redirected_hello = target
            .url()
            .is_some_and(|now| page != Some(&now) && engine.said_hello_since(label, &now, since));
        if engine.said_hello_since(label, dest, since) || redirected_hello {
            return Ok(serde_json::json!({"ok": true}));
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() || !engine.wait_hello_after(seen, remaining).await {
            break;
        }
    }
    let shown = target.url().unwrap_or_else(|| dest.clone());
    let what = if shown == *dest {
        format!("navigated to {dest}, but no pilot bridge answered there within {limit:?}")
    } else if never_left(engine, page, &shown) {
        return Err(dest_not_loaded_error(engine, &shown, dest, limit));
    } else {
        format!(
            "navigate to {dest}: the window shows {shown}, but no pilot bridge answered there \
             within {limit:?}"
        )
    };
    Err(no_bridge_error(engine, &shown, &what))
}

/// Whether a window showing `now` after a silent wait never left `page`.
///
/// True when `now` is `page`. Also true when `now` is on an origin whose
/// bridge answers and that is `page`'s origin, or `page` is unknown: a new
/// document there would have said hello, so only the URL of the old
/// document changed (`history.pushState`).
fn never_left(engine: &EvalEngine, page: Option<&tauri::Url>, now: &tauri::Url) -> bool {
    if page == Some(now) {
        return true;
    }
    let bridged = engine.hellos() > 0 && engine.has_bridge(now);
    bridged && page.is_none_or(|page| origin_key(page) == origin_key(now))
}

/// Error for a cross-origin navigate whose window never left `page`.
///
/// The destination did not load in time (#278), so the "allow this origin
/// in `remote.urls`" hint of [`no_bridge_error`] does not apply. A local
/// destination (see [`is_local_scheme`]) does not load over the network,
/// so a timeout cannot explain it: the error says the navigation was
/// cancelled or blocked (#310). When `page` has no bridge, the error still
/// names the origins that work.
fn dest_not_loaded_error(
    engine: &EvalEngine,
    page: &tauri::Url,
    dest: &tauri::Url,
    limit: Duration,
) -> RpcError {
    let way_back = if engine.has_bridge(page) {
        String::new()
    } else {
        let origins = engine.bridge_origins().join(", ");
        format!(". Bridge commands only work on {origins}: navigate back there")
    };
    let message = if is_local_scheme(dest) {
        format!(
            "navigate to {dest} did not happen: the window still shows {page} after \
             {limit:?}, so the navigation was cancelled or blocked{way_back}"
        )
    } else {
        format!(
            "navigate to {dest} did not load in time: the window still shows {page} after \
             {limit:?}{way_back}"
        )
    };
    RpcError {
        code: -32603,
        message,
        data: None,
    }
}

fn fail_no_bridge(engine: &EvalEngine, id: u64, page: &tauri::Url) -> RpcError {
    engine.resolve(id, Err("page has no pilot bridge".to_owned()));
    no_bridge_error(
        engine,
        page,
        &format!("no pilot bridge on the current page ({page})"),
    )
}

/// Send a bridge script and wait for its callback.
///
/// Refuses without running the script when the page has no bridge that can
/// call back, instead of waiting out `timeout` (#153). The wrapper also pins
/// the origin of that URL, so a navigation in the gap before eval cannot run
/// the command on a page nobody checked (#173).
async fn eval_bridge(
    script: &str,
    engine: &EvalEngine,
    webviews: &dyn Webviews,
    window: Option<&str>,
    timeout: Duration,
) -> Result<serde_json::Value, RpcError> {
    let target = target(webviews, window).await?;
    let checked = target.url();
    if let Some(page) = checked.as_ref().filter(|page| !engine.has_bridge(page)) {
        return Err(no_bridge_error(
            engine,
            page,
            &format!("no pilot bridge on the current page ({page})"),
        ));
    }
    let (id, rx) = send_script(script, engine, target.as_ref(), checked.as_ref())?;
    let now = target.url();
    if let Some((checked, now)) = origin_moved_page(checked.as_ref(), now.as_ref()) {
        return Err(fail_origin_moved(engine, id, checked, now));
    }
    match engine.wait(id, rx, timeout).await {
        Ok(value) => Ok(value),
        Err(EvalError::Timeout(_)) => Err(timeout_or_moved_origin(
            checked.as_ref(),
            target.url().as_ref(),
            timeout,
        )),
        Err(e) => Err(eval_rpc_error(&e)),
    }
}

/// Origin-moved error when the page left the pinned origin, else the timeout.
fn timeout_or_moved_origin(
    checked: Option<&tauri::Url>,
    now: Option<&tauri::Url>,
    timeout: Duration,
) -> RpcError {
    if let Some((checked, now)) = origin_moved_page(checked, now) {
        origin_moved_error(checked, now)
    } else {
        eval_rpc_error(&EvalError::Timeout(timeout))
    }
}

/// The checked URL and the page `now` if they are different origins.
fn origin_moved_page<'a>(
    checked: Option<&'a tauri::Url>,
    now: Option<&'a tauri::Url>,
) -> Option<(&'a tauri::Url, &'a tauri::Url)> {
    let now = now?;
    let checked = checked?;
    (origin_key(checked) != origin_key(now)).then_some((checked, now))
}

fn fail_origin_moved(
    engine: &EvalEngine,
    id: u64,
    checked: &tauri::Url,
    now: &tauri::Url,
) -> RpcError {
    engine.resolve(id, Err("page origin changed".to_owned()));
    origin_moved_error(checked, now)
}

fn origin_moved_error(checked: &tauri::Url, now: &tauri::Url) -> RpcError {
    RpcError {
        code: -32603,
        message: format!("command was pinned to {checked} and the page changed to {now}"),
        data: None,
    }
}

/// How long a request without `--window` waits for the app's first window (#273).
///
/// The plugin binds its socket during setup, before Tauri creates the windows
/// of `tauri.conf.json`, so `ping` answers before any window exists: for
/// about a second, 1.3 to 1.5 s in #273. Reuses [`BRIDGE_GRACE`], the budget `navigate` gives a page that
/// never said hello: both cover a local page still starting up. It only runs
/// while the app has no window at all, so a request to a running app pays
/// nothing. It adds to the method's own timeout, which keeps the longest
/// path, `screenshot` at [`SCREENSHOT_TIMEOUT`], under the CLI's
/// `DEFAULT_RPC_TIMEOUT` of 35 s.
const FIRST_WINDOW_BUDGET: Duration = BRIDGE_GRACE;

/// Pause between window lookups during [`FIRST_WINDOW_BUDGET`].
///
/// A lookup is a map read on the app handle, so polling is cheap. Short
/// enough that the command follows the new window within a frame or two.
const FIRST_WINDOW_POLL: Duration = Duration::from_millis(20);

/// Resolve the window a request targets.
///
/// Without a label, waits up to [`FIRST_WINDOW_BUDGET`] for the app's first
/// window when it has none yet, as during startup (#273). A label is never
/// waited for.
///
/// # Errors
///
/// A label that names no window is the caller's mistake, so it gets the
/// envelope an unknown `screenshot_native` window id already gets (#149):
/// `RPC_INVALID_PARAMS`, `data.error = WINDOW_NOT_FOUND`, and the windows that
/// do exist under `data.available_windows`, so a typo in `--window` costs no
/// `windows` round-trip (#233). Those rows are `{label, url, title}`, where
/// `screenshot_native` reports `{window_id, owner, title, layer}`; only the
/// envelope is shared. Having no window at all is the app's state, not a bad
/// request, and stays `RPC_INTERNAL_ERROR`, with `data.error = NO_WEBVIEW`
/// so a client can retry it without matching the message.
async fn target<'a>(
    webviews: &'a dyn Webviews,
    window: Option<&str>,
) -> Result<Box<dyn TargetWindow + 'a>, RpcError> {
    target_within(webviews, window, FIRST_WINDOW_BUDGET).await
}

/// Resolve the window a request targets, waiting at most `budget` for a first window.
///
/// [`target`] with an explicit budget. `Duration::ZERO` resolves without
/// waiting, for a caller that already waited and must not wait again.
///
/// # Errors
///
/// Same as [`target`].
async fn target_within<'a>(
    webviews: &'a dyn Webviews,
    window: Option<&str>,
    budget: Duration,
) -> Result<Box<dyn TargetWindow + 'a>, RpcError> {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        match webviews.target(window) {
            Ok(target) => return Ok(target),
            Err(e) if window.is_some() => {
                return Err(RpcError {
                    code: RPC_INVALID_PARAMS,
                    data: Some(serde_json::json!({
                        "error": screenshot::ipc::codes::WINDOW_NOT_FOUND,
                        "message": e,
                        "available_windows": webviews.list(),
                    })),
                    message: e,
                });
            }
            Err(e) if tokio::time::Instant::now() >= deadline => {
                return Err(RpcError {
                    code: RPC_INTERNAL_ERROR,
                    data: Some(serde_json::json!({
                        "error": screenshot::ipc::codes::NO_WEBVIEW,
                        "message": e,
                    })),
                    message: e,
                });
            }
            Err(_) => tokio::time::sleep(FIRST_WINDOW_POLL).await,
        }
    }
}

/// Answer `url` from the runtime, without asking the page.
///
/// # Errors
///
/// Fails when the window cannot be resolved (see [`target`]), or when the
/// runtime reports no URL for it — a failure the bridge route never produced,
/// since a page that answers at all knows its own `location`.
async fn handle_url(
    webviews: &dyn Webviews,
    window: Option<&str>,
) -> Result<serde_json::Value, RpcError> {
    let url = target(webviews, window)
        .await?
        .url()
        .ok_or_else(|| RpcError {
            code: RPC_INTERNAL_ERROR,
            message: "the runtime cannot report the URL of the current page".to_owned(),
            data: None,
        })?;
    Ok(serde_json::Value::String(url.to_string()))
}

/// Register a callback, then eval `script` wrapped in the ADR-001 pattern.
///
/// `origin` is the URL this eval was aimed at. The wrapper refuses to run
/// the command if the document that eventually evals it is elsewhere.
/// `navigate` passes `None` so recovery still runs if the page already left.
fn send_script(
    script: &str,
    engine: &EvalEngine,
    target: &dyn TargetWindow,
    origin: Option<&tauri::Url>,
) -> Result<CallbackSlot, RpcError> {
    let (id, rx) = engine.register();
    let wrapped = EvalEngine::wrap_script(id, script, origin);
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
type CallbackSlot = (u64, ReplyReceiver);

/// Wait for the callback of eval `id`.
async fn wait(
    engine: &EvalEngine,
    id: u64,
    rx: ReplyReceiver,
    timeout: Duration,
) -> Result<serde_json::Value, RpcError> {
    engine
        .wait(id, rx, timeout)
        .await
        .map_err(|e| eval_rpc_error(&e))
}

fn eval_rpc_error(e: &EvalError) -> RpcError {
    RpcError {
        code: -32603,
        message: format!("Eval error: {e}"),
        data: None,
    }
}

/// Build the error for `page`, whose bridge cannot answer, naming the
/// origins where it can.
///
/// `remote.urls` only takes remote URL patterns, so its hint is left out
/// for a local page (see [`is_local_scheme`]).
fn no_bridge_error(engine: &EvalEngine, page: &tauri::Url, what: &str) -> RpcError {
    let origins = engine.bridge_origins().join(", ");
    let hint = if is_local_scheme(page) {
        ""
    } else {
        ", or allow this origin in a capability's remote.urls"
    };
    RpcError {
        code: -32603,
        message: format!(
            "{what}. Bridge commands only work on {origins}: navigate back there{hint}"
        ),
        data: None,
    }
}

/// Whether `url` is a local page that does not load over the network.
///
/// `about:`, `data:`, `blob:` and `file:` pages are built or read by the
/// webview itself: `remote.urls` cannot allow them, and a navigation to one
/// that never happened was cancelled or blocked rather than slow (#310).
fn is_local_scheme(url: &tauri::Url) -> bool {
    matches!(url.scheme(), "about" | "data" | "blob" | "file")
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
///
/// `label` is the invoking webview. A hello is recorded under it, so a
/// navigate waits for its own window (see [`EvalEngine::bridge_hello`]).
pub(crate) fn handle_callback(
    engine: &EvalEngine,
    label: &str,
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
            (Some(webview), _) => engine.bridge_hello(label, webview.as_str()),
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
///
/// Synchronous, so Tauri holds the plugin store lock until it returns. The
/// page is the last load that `Started` on this webview. `webview.url()` is
/// not called: on Android that waits for the main thread, and navigation
/// needs the same lock (#252). A new load cannot be recorded until this
/// returns, so an in-flight hello still names the page it came from.
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
    finish_callback(&eval_engine, &webview, id, result, error);
}

/// Legacy Tauri IPC command for the `__callback` handler.
///
/// Same lock and page rules as [`callback`].
///
/// `#[tauri::command]` binds `State<'_, T>` by value. The generated wrapper is
/// the true consumer, so clippy's view of the body is incomplete. Cannot be
/// rewritten as `&State` (tauri command macro rejects it).
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
    finish_callback(&eval_engine, &webview, id, result, error);
}

/// Record the callback against the page saved when its load started.
fn finish_callback<R: tauri::Runtime>(
    eval_engine: &EvalEngine,
    webview: &tauri::WebviewWindow<R>,
    id: u64,
    result: Option<String>,
    error: Option<String>,
) {
    let page = eval_engine.page_at_start(webview.label());
    handle_callback(
        eval_engine,
        webview.label(),
        id,
        result,
        error,
        page.as_ref(),
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
        assert_eq!(err.code, -32602);
        assert_eq!(err.message, "Window 'settings' not found");
    }

    #[cfg(feature = "press")]
    #[tokio::test(start_paused = true)]
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
    #[tokio::test(start_paused = true)]
    async fn test_dispatch_press_waiting_for_a_window_does_not_block_a_labeled_press() {
        // An unlabeled press waits up to FIRST_WINDOW_BUDGET for the first
        // window (#273). It must not hold the press ordering lock meanwhile,
        // or a `--window` press queues behind it for the whole wait.
        let webviews = FakeWebviews::default();
        let waiting = webviews.clone();
        let unlabeled = tokio::spawn(async move {
            dispatch(
                "press",
                Some(&json!({"key": "Shift"})),
                &EvalEngine::new(),
                &waiting,
                &Recorder::new(),
            )
            .await
        });
        // Let the unlabeled press start its wait.
        tokio::time::sleep(Duration::from_millis(1)).await;
        let start = tokio::time::Instant::now();

        let err = dispatch(
            "press",
            Some(&json!({"key": "Shift", "window": "settings"})),
            &EvalEngine::new(),
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("no window labeled settings");

        // Not `ZERO`: PRESS_ORDER_LOCK is process-wide, so a press test on
        // another thread can hold it for a moment while this paused clock
        // auto-advances. Holding it across the wait costs the full budget.
        assert!(
            start.elapsed() < FIRST_WINDOW_BUDGET / 2,
            "took {:?}",
            start.elapsed()
        );
        assert_eq!(err.code, -32602);
        let err = unlabeled
            .await
            .expect("press task")
            .expect_err("no window ever appears");
        assert_eq!(err.code, -32603);
    }

    #[cfg(feature = "press")]
    #[tokio::test(start_paused = true)]
    async fn test_dispatch_press_resolves_the_window_again_after_queueing() {
        // The first-window wait runs before PRESS_ORDER_LOCK (#273), so a
        // press can queue behind another one. A window closed meanwhile
        // must not be focused through the handle resolved before the lock.
        let webviews = FakeWebviews::window("settings", Some("https://app.test/"));
        let queued = webviews.clone();
        let guard = PRESS_ORDER_LOCK.lock().await;
        let press = tokio::spawn(async move {
            dispatch(
                "press",
                Some(&json!({"key": "Shift", "window": "settings"})),
                &EvalEngine::new(),
                &queued,
                &Recorder::new(),
            )
            .await
        });
        // Let the press resolve its window and queue on the lock.
        tokio::time::sleep(Duration::from_millis(1)).await;
        webviews.close("settings");
        drop(guard);

        let err = press
            .await
            .expect("press task")
            .expect_err("the window closed while the press queued");

        assert_eq!(err.code, -32602);
        let data = err.data.expect("the WINDOW_NOT_FOUND envelope");
        assert_eq!(data["error"], json!("WINDOW_NOT_FOUND"));
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

    #[test]
    fn test_press_unsupported_error_says_mobile_is_unsupported() {
        // #256: on mobile the error must not send the user to recompile with
        // the `press` feature or blame another app for holding focus.
        let err = press_unsupported_error();
        assert_eq!(err.code, RPC_INTERNAL_ERROR);
        assert_eq!(
            err.message,
            "press is not supported on Android/iOS (use `fill` or `type` for text input)"
        );
        assert_eq!(
            err.data
                .as_ref()
                .and_then(|d| d.get("error"))
                .and_then(serde_json::Value::as_str),
            Some("UNSUPPORTED_PLATFORM")
        );
    }

    #[cfg(feature = "press")]
    #[tokio::test(start_paused = true)]
    async fn test_dispatch_press_fails_when_window_does_not_gain_focus() {
        // set_focus reports success even when the WM refuses the transfer
        // (X11 focus-stealing prevention). Injecting anyway would type into
        // whichever app actually has focus (#175).
        let engine = EvalEngine::new();
        let webviews = FakeWebviews::window("main", Some("https://app.test/"));
        let result = dispatch(
            "press",
            Some(&json!({"key": "a"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("unfocused press must not inject");
        assert_eq!(err.code, -32603);
        assert_eq!(
            err.message,
            "cannot press: window 'main' did not gain focus (another application has it)"
        );
    }

    #[cfg(feature = "press")]
    #[tokio::test(start_paused = true)]
    async fn test_dispatch_press_focus_error_names_the_window() {
        let engine = EvalEngine::new();
        let webviews = FakeWebviews::window("settings", Some("https://app.test/"));
        let result = dispatch(
            "press",
            Some(&json!({"key": "a", "window": "settings"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("unfocused press must not inject");
        assert_eq!(err.code, -32603);
        assert_eq!(
            err.message,
            "cannot press: window 'settings' did not gain focus (another application has it)"
        );
    }

    #[cfg(feature = "press")]
    #[tokio::test(start_paused = true)]
    async fn test_wait_until_focused_succeeds_when_already_focused() {
        let webviews = FakeWebviews::window("main", Some("https://app.test/"));
        webviews.set_focused("main", true);
        let target = webviews.target(None).expect("main window");
        wait_until_focused(target).await.expect("already focused");
    }

    #[cfg(feature = "press")]
    #[tokio::test(start_paused = true)]
    async fn test_wait_until_focused_succeeds_when_focus_arrives() {
        let webviews = FakeWebviews::window("main", Some("https://app.test/"));
        let later = webviews.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            later.set_focused("main", true);
        });
        let target = webviews.target(None).expect("main window");
        wait_until_focused(target)
            .await
            .expect("focus arrived within the budget");
    }

    #[cfg(feature = "press")]
    #[tokio::test(start_paused = true)]
    async fn test_wait_until_focused_errors_when_never_focused() {
        let webviews = FakeWebviews::window("main", Some("https://app.test/"));
        let target = webviews.target(None).expect("main window");
        let err = wait_until_focused(target).await.expect_err("never focused");
        assert_eq!(err.code, RPC_INTERNAL_ERROR);
        assert_eq!(
            err.message,
            "cannot press: window 'main' did not gain focus (another application has it)"
        );
    }

    #[cfg(feature = "press")]
    #[tokio::test(start_paused = true)]
    async fn test_wait_until_focused_errors_when_focus_query_fails() {
        let webviews = FakeWebviews::window("main", Some("https://app.test/"));
        webviews.set_focus_query_error("main", "FailedToSendMessage");
        let target = webviews.target(None).expect("main window");
        let err = wait_until_focused(target)
            .await
            .expect_err("query failure is not unfocused");
        assert_eq!(err.code, RPC_INTERNAL_ERROR);
        assert_eq!(
            err.message,
            "cannot press: failed to query focus for window 'main': FailedToSendMessage"
        );
    }

    #[cfg(feature = "press")]
    #[tokio::test(start_paused = true)]
    async fn test_dispatch_press_skips_injection_when_focus_query_fails() {
        let engine = EvalEngine::new();
        let webviews = FakeWebviews::window("main", Some("https://app.test/"));
        webviews.set_focus_query_error("main", "FailedToSendMessage");
        let result = dispatch(
            "press",
            Some(&json!({"key": "a"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await;
        let err = result.expect_err("query failure must not inject");
        assert_eq!(err.code, -32603);
        assert_eq!(
            err.message,
            "cannot press: failed to query focus for window 'main': FailedToSendMessage"
        );
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

    #[tokio::test(start_paused = true)]
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
        assert_eq!(err.message, "No webview available");
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_waits_for_the_first_webview_window() {
        // #273: the socket answers before Tauri creates the first window.
        // A command sent in that gap waits for it instead of failing.
        let engine = EvalEngine::new();
        let webviews = FakeWebviews::default();
        let later = webviews.clone();
        let appear = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1_400)).await;
            later.set_url("main", Some("https://app.test/"));
        });
        let start = tokio::time::Instant::now();

        let result = dispatch("url", None, &engine, &webviews, &Recorder::new()).await;

        assert_eq!(result.expect("a window"), json!("https://app.test/"));
        assert!(
            start.elapsed() >= Duration::from_millis(1_400)
                && start.elapsed() < FIRST_WINDOW_BUDGET,
            "took {:?}",
            start.elapsed()
        );
        appear.await.expect("window task");
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_without_any_webview_fails_with_no_webview_after_the_budget() {
        let engine = EvalEngine::new();
        let start = tokio::time::Instant::now();

        let err = dispatch(
            "snapshot",
            None,
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await
        .expect_err("no window ever appears");

        assert!(
            start.elapsed() >= FIRST_WINDOW_BUDGET,
            "took {:?}",
            start.elapsed()
        );
        assert_eq!(err.code, -32603);
        assert_eq!(err.message, "No webview available");
        let data = err.data.expect("a domain code");
        assert_eq!(data["error"], json!("NO_WEBVIEW"));
        assert_eq!(data["message"], json!("No webview available"));
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_with_a_window_does_not_wait() {
        let engine = EvalEngine::new();
        let webviews = FakeWebviews::window("main", Some("https://app.test/"));
        let start = tokio::time::Instant::now();

        let result = dispatch("url", None, &engine, &webviews, &Recorder::new()).await;

        assert_eq!(result.expect("a window"), json!("https://app.test/"));
        assert_eq!(start.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_unknown_window_fails_at_once_during_startup() {
        // A `--window` label is never waited for (#273), even while the app
        // has no window yet and one with that label appears later.
        let engine = EvalEngine::new();
        let webviews = FakeWebviews::default();
        let later = webviews.clone();
        let appear = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1_400)).await;
            later.set_url("nope", Some("https://app.test/"));
        });
        let start = tokio::time::Instant::now();

        let err = dispatch(
            "url",
            Some(&json!({"window": "nope"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("no window yet");

        assert_eq!(start.elapsed(), Duration::ZERO);
        assert_eq!(err.code, -32602);
        let data = err.data.expect("the WINDOW_NOT_FOUND envelope");
        assert_eq!(data["error"], json!("WINDOW_NOT_FOUND"));
        assert_eq!(data["available_windows"], json!([]));
        appear.abort();
    }

    #[tokio::test(start_paused = true)]
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

    /// A webview whose bridge answers the first eval with `result`.
    fn answering_webviews(engine: &EvalEngine, result: serde_json::Value) -> FakeWebviews {
        let engine = engine.clone();
        // The first registered callback on a fresh engine has id == 1.
        FakeWebviews::window("main", None).on_eval(move || engine.resolve(1, Ok(result.clone())))
    }

    #[tokio::test]
    async fn test_dispatch_snapshot_records_capture_options() {
        // #244: a saved snapshot must say which options produced it, so a
        // later `diff --ref` can refuse a comparison that cannot be right.
        let engine = EvalEngine::new();
        let webviews = answering_webviews(&engine, json!({"elements": []}));
        let params = json!({"interactive": true, "selector": "#app", "depth": 3});
        let result = dispatch(
            "snapshot",
            Some(&params),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("snapshot succeeds");
        let options = json!({"interactive": true, "selector": "#app", "depth": 3});
        assert_eq!(result["options"], options);
        let stored = engine.get_last_snapshot().expect("snapshot stored");
        assert_eq!(stored["options"], options);
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_diff_rejects_reference_with_other_options() {
        // #244: an interactive diff against a full baseline reported every
        // non-interactive element as removed. The check runs before any eval,
        // so a webview that never answers is enough.
        let engine = EvalEngine::new();
        let webviews = FakeWebviews::window("main", None);
        let params = json!({
            "interactive": true,
            "reference": {
                "elements": [],
                "options": {"interactive": false, "selector": null, "depth": null}
            }
        });
        let err = dispatch("diff", Some(&params), &engine, &webviews, &Recorder::new())
            .await
            .expect_err("mismatched options must be refused");
        assert_eq!(err.code, -32602);
        assert!(
            err.message
                .contains("interactive (reference: false, current: true)"),
            "{}",
            err.message
        );
        let data = err.data.expect("structured data");
        assert_eq!(data["reference"]["interactive"], json!(false));
        assert_eq!(data["current"]["interactive"], json!(true));
        assert!(webviews.scripts().is_empty(), "must not snapshot the page");
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_diff_rejects_last_snapshot_with_other_options() {
        // Same trap without `--ref`: `snapshot` then `diff -i` compared the
        // interactive capture against the full in-memory snapshot.
        let engine = EvalEngine::new();
        engine.store_snapshot(&json!({
            "elements": [],
            "options": {"interactive": false, "selector": null, "depth": null}
        }));
        let webviews = FakeWebviews::window("main", None);
        let params = json!({"interactive": true});
        let err = dispatch("diff", Some(&params), &engine, &webviews, &Recorder::new())
            .await
            .expect_err("mismatched options must be refused");
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("interactive"), "{}", err.message);
    }

    #[tokio::test]
    async fn test_dispatch_diff_warns_on_reference_without_options() {
        // A baseline saved by 0.7.3 or earlier has no `options` key: diff it,
        // but say the options could not be checked.
        let engine = EvalEngine::new();
        let webviews = answering_webviews(&engine, json!({"elements": []}));
        let params = json!({"interactive": true, "reference": {"elements": []}});
        let result = dispatch("diff", Some(&params), &engine, &webviews, &Recorder::new())
            .await
            .expect("legacy reference is still diffed");
        let warning = result["warning"].as_str().expect("warning present");
        assert!(warning.contains("capture options"), "{warning}");
        assert_eq!(result["removed"], json!([]));
    }

    #[tokio::test]
    async fn test_dispatch_diff_with_matching_options_stores_them() {
        let engine = EvalEngine::new();
        let webviews = answering_webviews(&engine, json!({"elements": []}));
        let options = json!({"interactive": true, "selector": null, "depth": null});
        let params = json!({
            "interactive": true,
            "reference": {"elements": [], "options": options}
        });
        let result = dispatch("diff", Some(&params), &engine, &webviews, &Recorder::new())
            .await
            .expect("matching options diff");
        assert!(result.get("warning").is_none(), "{result}");
        // The stored snapshot feeds the next `diff`, so it carries options too.
        let stored = engine.get_last_snapshot().expect("snapshot stored");
        assert_eq!(stored["options"], options);
    }

    #[tokio::test]
    async fn test_dispatch_diff_sends_the_bridge_the_recorded_options() {
        // `depth: 255` is the bridge default, so it matches a reference with
        // no depth. The bridge gets the normalized options, not the raw
        // params, and never the embedded reference.
        let engine = EvalEngine::new();
        let webviews = answering_webviews(&engine, json!({"elements": []}));
        let params = json!({
            "interactive": true,
            "depth": 255,
            "reference": {
                "elements": [],
                "options": {"interactive": true, "selector": null, "depth": null}
            }
        });
        dispatch("diff", Some(&params), &engine, &webviews, &Recorder::new())
            .await
            .expect("depth 255 matches the default");
        let scripts = webviews.scripts();
        let script = scripts.first().expect("snapshot evaluated");
        assert!(script.contains(r#""depth":null"#), "{script}");
        assert!(script.contains(r#""interactive":true"#), "{script}");
        assert!(!script.contains("reference"), "{script}");
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_snapshot_rejects_wrong_option_types() {
        // `"false"` is truthy in the bridge: it would capture interactive
        // elements only while the snapshot records `interactive: false`.
        let engine = EvalEngine::new();
        let webviews = FakeWebviews::window("main", None);
        for method in ["snapshot", "diff"] {
            engine.store_snapshot(&json!({"elements": []}));
            let params = json!({"interactive": "false"});
            let err = dispatch(method, Some(&params), &engine, &webviews, &Recorder::new())
                .await
                .expect_err("wrong option type must be refused");
            assert_eq!(err.code, -32602, "{method}");
            assert!(err.message.contains("`interactive`"), "{}", err.message);
        }
        assert!(webviews.scripts().is_empty(), "must not snapshot the page");
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_diff_rejects_incomplete_reference_options() {
        let engine = EvalEngine::new();
        let webviews = FakeWebviews::window("main", None);
        let params = json!({"reference": {"elements": [], "options": {"interactive": false}}});
        let err = dispatch("diff", Some(&params), &engine, &webviews, &Recorder::new())
            .await
            .expect_err("incomplete reference options must be refused");
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("`selector`"), "{}", err.message);
        assert!(webviews.scripts().is_empty(), "must not snapshot the page");
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
            "main",
            id,
            Some(r#"{"title":"hello"}"#.to_owned()),
            None,
            None,
        );
        let val = engine.wait(id, rx, DEFAULT_TIMEOUT).await.expect("eval ok");
        assert_eq!(val, json!({"title": "hello"}));
    }

    #[tokio::test]
    async fn test_callback_with_null_string_resolves_to_value_null() {
        // #48 round-trip: wrap_script sends `result: 'null'` (string) when the
        // JS expr returns undefined. The callback must parse it back to
        // Value::Null so the client sees a clean success, not a warn fallback.
        let engine = EvalEngine::new();
        let (id, rx) = engine.register();
        handle_callback(&engine, "main", id, Some("null".to_owned()), None, None);
        let val = engine.wait(id, rx, DEFAULT_TIMEOUT).await.expect("eval ok");
        assert_eq!(val, serde_json::Value::Null);
    }

    #[tokio::test]
    async fn test_callback_with_error() {
        let engine = EvalEngine::new();
        let (id, rx) = engine.register();
        handle_callback(
            &engine,
            "main",
            id,
            None,
            Some("TypeError: x".to_owned()),
            None,
        );
        let result = engine.wait(id, rx, DEFAULT_TIMEOUT).await;
        assert!(
            matches!(result, Err(EvalError::JsError(ref m)) if m == "TypeError: x"),
            "got {result:?}"
        );
    }

    #[tokio::test(start_paused = true)]
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

    #[tokio::test(start_paused = true)]
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

    #[tokio::test(start_paused = true)]
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

    #[tokio::test(start_paused = true)]
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

    #[tokio::test(start_paused = true)]
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

    #[tokio::test(start_paused = true)]
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

    #[tokio::test(start_paused = true)]
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

    #[tokio::test(start_paused = true)]
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

    #[tokio::test(start_paused = true)]
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

    #[tokio::test(start_paused = true)]
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

    #[tokio::test(start_paused = true)]
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

    /// An unknown method answers -32601; reaching the webview lookup proves
    /// `storage.delete` is routed to the bridge (#284).
    #[tokio::test]
    async fn test_dispatch_storage_delete_without_webview() {
        let engine = EvalEngine::new();
        let params = json!({"key": "auth_token", "session": false});
        let result = dispatch(
            "storage.delete",
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

    #[test]
    fn test_build_bridge_call_storage_delete() {
        let params = json!({"key": "auth_token", "session": true});
        let script = build_bridge_call("storageDelete", Some(&params)).expect("build_bridge_call");
        assert_eq!(
            script,
            r#"window.__PILOT__.storageDelete({"key":"auth_token","session":true})"#
        );
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

    #[tokio::test(start_paused = true)]
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
    async fn test_dispatch_windows_list_sorts_multiple_windows() {
        let engine = EvalEngine::new();
        let webviews = FakeWebviews::windows(&[
            ("settings", Some("https://settings.test/")),
            ("alpha", Some("https://alpha.test/")),
        ]);
        let result = dispatch("windows.list", None, &engine, &webviews, &Recorder::new()).await;
        let val = result.expect("dispatch succeeds");
        let windows = val
            .get("windows")
            .expect("windows key present")
            .as_array()
            .expect("windows is array");
        let labels = windows
            .iter()
            .map(|window| {
                window
                    .get("label")
                    .expect("label key present")
                    .as_str()
                    .expect("label is a string")
            })
            .collect::<Vec<_>>();

        assert_eq!(labels, ["alpha", "settings"]);
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

    #[tokio::test(start_paused = true)]
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
        recorder.record("click", Some(&json!({"ref": "e1"})), None);
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
        // A ref step recorded without a selector is reported (#276).
        assert_eq!(
            result["unstable"],
            json!([{"step": 1, "action": "click", "ref": "e1"}])
        );
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

    // ─── recorded locators (#276) ────────────────────────────────────────────

    /// A webview whose bridge answers `locate` with `located` and any other
    /// call with `{"ok": true}`. Callback ids count up from 1 on a fresh engine.
    fn locating_webviews(
        engine: &EvalEngine,
        windows: &[(&str, Option<&str>)],
        located: serde_json::Value,
    ) -> FakeWebviews {
        let webviews = FakeWebviews::windows(windows);
        let seen = webviews.clone();
        let engine = engine.clone();
        let next = std::sync::atomic::AtomicU64::new(1);
        webviews.on_eval(move || {
            let id = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let script = seen.scripts().pop().expect("script evaluated");
            let answer = if script.contains("__PILOT__.locate(") {
                located.clone()
            } else {
                json!({"ok": true})
            };
            engine.resolve(id, Ok(answer));
        })
    }

    #[tokio::test]
    async fn test_dispatch_records_a_ref_step_with_its_locator_and_window() {
        // Case A of #276: the saved step must carry what a fresh document
        // needs, not only a ref that dies with the snapshot.
        let engine = EvalEngine::new();
        let recorder = Recorder::new();
        recorder.start();
        let fingerprint = json!({"tag": "input", "role": "radio", "name": "Pro"});
        let webviews = locating_webviews(
            &engine,
            &[("main", None), ("settings", None)],
            json!({"self": {"selector": "#plan-pro", "expect": fingerprint}}),
        );
        let params = json!({"ref": "e9", "window": "settings"});
        dispatch("check", Some(&params), &engine, &webviews, &recorder)
            .await
            .expect("check succeeds");

        let scripts = webviews.scripts();
        assert_eq!(scripts.len(), 2, "{scripts:?}");
        // Located before the action runs: a click can remove the element.
        assert!(scripts[0].contains(r#"__PILOT__.locate({"refs":{"self":"e9"}})"#));
        assert!(scripts[1].contains("__PILOT__.check("));
        // Refs are per window: locating in another one would read its map.
        assert_eq!(webviews.script_windows(), ["settings", "settings"]);

        let entries = recorder.stop().expect("recording active");
        let entry = serde_json::to_value(&entries[0]).expect("entry serializes");
        assert_eq!(entry["ref"], "e9");
        assert_eq!(entry["selector"], "#plan-pro");
        assert_eq!(entry["expect"], fingerprint);
        assert_eq!(entry["window"], "settings");
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_bounds_a_locate_the_bridge_never_answers() {
        // `locate` is best effort: a silent bridge must not hold every
        // recorded step for the full DEFAULT_TIMEOUT before it runs.
        let engine = EvalEngine::new();
        let recorder = Recorder::new();
        recorder.start();
        let webviews = FakeWebviews::window("main", None);
        let seen = webviews.clone();
        let answering = engine.clone();
        let next = std::sync::atomic::AtomicU64::new(1);
        let webviews = webviews.on_eval(move || {
            let id = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let script = seen.scripts().pop().expect("script evaluated");
            if !script.contains("__PILOT__.locate(") {
                answering.resolve(id, Ok(json!({"ok": true})));
            }
        });
        let start = tokio::time::Instant::now();
        dispatch(
            "click",
            Some(&json!({"ref": "e1"})),
            &engine,
            &webviews,
            &recorder,
        )
        .await
        .expect("click succeeds");
        assert!(
            start.elapsed() <= Duration::from_secs(2),
            "took {:?}",
            start.elapsed()
        );
        // The step is still recorded, ref-only.
        let entries = recorder.stop().expect("recording active");
        assert_eq!(entries[0].params.get("selector"), None);
        assert_eq!(entries[0].params["ref"], "e1");
    }

    #[tokio::test]
    async fn test_dispatch_does_not_locate_when_not_recording() {
        let engine = EvalEngine::new();
        let webviews = locating_webviews(&engine, &[("main", None)], json!({}));
        dispatch(
            "click",
            Some(&json!({"ref": "e1"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("click succeeds");
        let scripts = webviews.scripts();
        assert_eq!(scripts.len(), 1, "{scripts:?}");
        assert!(scripts[0].contains("__PILOT__.click("));
    }

    #[tokio::test]
    async fn test_dispatch_replays_a_recorded_step_by_selector_and_fingerprint() {
        // Cases A and B of #276: the recorded selector and fingerprint reach
        // the bridge, which resolves them strictly instead of the stale ref.
        let engine = EvalEngine::new();
        let recorder = Recorder::new();
        recorder.start();
        let located = json!({"self": {"selector": "#plan-pro", "expect": {"tag": "input"}}});
        let webviews = locating_webviews(&engine, &[("main", None)], located);
        dispatch(
            "check",
            Some(&json!({"ref": "e9"})),
            &engine,
            &webviews,
            &recorder,
        )
        .await
        .expect("check succeeds");
        let entry = recorder.stop().expect("recording active").remove(0);

        dispatch(
            &entry.action,
            Some(&serde_json::Value::Object(entry.params)),
            &engine,
            &webviews,
            &recorder,
        )
        .await
        .expect("replayed check succeeds");
        let replayed = webviews.scripts().pop().expect("script evaluated");
        assert!(
            replayed.contains(
                r##"__PILOT__.check({"expect":{"tag":"input"},"ref":"e9","selector":"#plan-pro"})"##
            ),
            "{replayed}"
        );
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

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_url_answers_without_the_bridge() {
        // No responder, so no callback ever lands: a bridge-routed `url` would
        // time out here, the way it does on a foreign origin whose `__callback`
        // the ACL denies (issue #233).
        let engine = EvalEngine::new();
        let webviews = FakeWebviews::window("main", Some("https://example.com/page"));

        let result = dispatch("url", None, &engine, &webviews, &Recorder::new())
            .await
            .expect("url is answered by the runtime");

        assert_eq!(result, json!("https://example.com/page"));
        assert!(
            webviews.scripts().is_empty(),
            "url must not touch the page: {:?}",
            webviews.scripts()
        );
    }

    #[tokio::test]
    async fn test_dispatch_unknown_window_lists_the_available_labels() {
        // An unknown `--window` label must name the valid ones, the way
        // `screenshot_native` answers an unknown window id (issues #149, #233).
        let engine = EvalEngine::new();
        let webviews = FakeWebviews::windows(&[
            ("main", Some("https://app.test/")),
            ("settings", Some("https://app.test/settings")),
        ]);

        let err = dispatch(
            "url",
            Some(&json!({"window": "nope"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("unknown label");

        // Same envelope as an unknown `screenshot_native` window id, so a
        // consumer that is not this CLI can tell a typo from a plugin failure
        // without parsing the message.
        assert_eq!(err.code, -32602);
        assert_eq!(err.message, "Window 'nope' not found");
        let data = err.data.expect("available windows");
        assert_eq!(data["error"], json!("WINDOW_NOT_FOUND"));
        assert_eq!(data["message"], json!("Window 'nope' not found"));
        let labels = data["available_windows"]
            .as_array()
            .expect("a list")
            .iter()
            .map(|w| w["label"].as_str().expect("a label").to_owned())
            .collect::<Vec<_>>();
        assert_eq!(labels, ["main", "settings"]);
    }

    #[tokio::test]
    async fn test_dispatch_url_errors_when_the_runtime_reports_no_url() {
        // `TargetWindow::url` is `None` whenever the runtime call fails or its
        // `catch_unwind` trips. Answering an empty string there would tell the
        // caller it is on a page called "", so it has to be an error (#233).
        let engine = EvalEngine::new();
        let webviews = FakeWebviews::window("main", None);

        let err = dispatch("url", None, &engine, &webviews, &Recorder::new())
            .await
            .expect_err("no URL to report");

        assert_eq!(err.code, -32603);
        assert_eq!(
            err.message,
            "the runtime cannot report the URL of the current page"
        );
        assert!(err.data.is_none());
    }

    #[tokio::test]
    async fn test_dispatch_unknown_window_lists_nothing_when_the_app_has_none() {
        // An app with no window still answers the envelope, with an empty
        // list: an absent key would read as "the plugin did not look".
        let engine = EvalEngine::new();

        let err = dispatch(
            "url",
            Some(&json!({"window": "nope"})),
            &engine,
            &FakeWebviews::default(),
            &Recorder::new(),
        )
        .await
        .expect_err("unknown label");

        assert_eq!(err.code, -32602);
        let data = err.data.expect("the WINDOW_NOT_FOUND envelope");
        assert_eq!(data["error"], json!("WINDOW_NOT_FOUND"));
        assert_eq!(data["available_windows"], json!([]));
    }

    const APP_PAGE: &str = "tauri://localhost/";
    const FOREIGN_PAGE: &str = "https://example.com/";

    fn url(text: &str) -> tauri::Url {
        tauri::Url::parse(text).expect("valid test URL")
    }

    #[test]
    fn test_navigate_eval_script_resolves_relative_against_dest() {
        let dest = url("tauri://localhost/settings");
        let script = navigate_eval_script(Some(&json!({"url": "/settings"})), Some(&dest))
            .expect("navigate script");
        assert!(script.contains("tauri://localhost/settings"));
        assert!(!script.contains("\"/settings\""));
    }

    #[test]
    fn test_navigate_eval_script_keeps_javascript_url() {
        let dest = url(APP_PAGE);
        let script = navigate_eval_script(Some(&json!({"url": "javascript:void(0)"})), Some(&dest))
            .expect("navigate script");
        assert!(script.contains("javascript:void(0)"));
        assert!(!script.contains(APP_PAGE));
    }

    /// Engine whose bridge already said hello from the app origin, as it does
    /// when the app page loads.
    fn engine_with_app_bridge() -> EvalEngine {
        let engine = EvalEngine::new();
        handle_callback(&engine, "main", HELLO_ID, None, None, Some(&url(APP_PAGE)));
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
    async fn test_dispatch_navigate_cross_origin_reports_page_never_left() {
        // #278: a destination that refuses the connection leaves the window
        // on the page. The error must say so, not send people to edit their
        // capabilities.
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE))
            .on_eval(move || engine_clone.resolve(1, Ok(json!({"ok": true}))));

        let err = dispatch(
            "navigate",
            Some(&json!({"url": "http://127.0.0.1:1/"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("a navigation that never left the page must not report ok");

        assert_eq!(err.code, -32603);
        assert_eq!(
            err.message,
            "navigate to http://127.0.0.1:1/ did not load in time: the window still shows \
             tauri://localhost/ after 3s"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_cross_origin_reports_silent_destination() {
        // The window reached the foreign page, but no bridge answers there:
        // the capability hint still applies.
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE));
        let webviews_clone = webviews.clone();
        let webviews = webviews.on_eval(move || {
            engine_clone.resolve(1, Ok(json!({"ok": true})));
            webviews_clone.set_url("main", Some(FOREIGN_PAGE));
        });

        let err = dispatch(
            "navigate",
            Some(&json!({"url": FOREIGN_PAGE})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("a silent foreign page must not report ok");

        assert_eq!(
            err.message,
            "navigated to https://example.com/, but no pilot bridge answered there \
             within 3s. Bridge commands only work on tauri://localhost: navigate back \
             there, or allow this origin in a capability's remote.urls"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_cross_origin_names_page_reached_instead() {
        // A redirect can land the window on a page other than the
        // destination: the error must name it, not claim it reached `dest`.
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE));
        let webviews_clone = webviews.clone();
        let webviews = webviews.on_eval(move || {
            engine_clone.resolve(1, Ok(json!({"ok": true})));
            webviews_clone.set_url("main", Some("https://example.com/login"));
        });

        let err = dispatch(
            "navigate",
            Some(&json!({"url": FOREIGN_PAGE})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("a silent redirect target must not report ok");

        assert_eq!(
            err.message,
            "navigate to https://example.com/: the window shows https://example.com/login, \
             but no pilot bridge answered there within 3s. Bridge commands only work on \
             tauri://localhost: navigate back there, or allow this origin in a \
             capability's remote.urls"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_about_blank_omits_remote_urls_hint() {
        // #310: `remote.urls` takes remote URL patterns, so it cannot give
        // `about:blank` a bridge. The error must only point back to the app.
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE));
        let webviews_clone = webviews.clone();
        let webviews = webviews.on_eval(move || {
            engine_clone.resolve(1, Ok(json!({"ok": true})));
            webviews_clone.set_url("main", Some("about:blank"));
        });

        let err = dispatch(
            "navigate",
            Some(&json!({"url": "about:blank"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("about:blank has no bridge");

        assert_eq!(
            err.message,
            "navigated to about:blank, but no pilot bridge answered there within 3s. \
             Bridge commands only work on tauri://localhost: navigate back there"
        );
    }

    #[tokio::test]
    async fn test_dispatch_on_local_scheme_page_omits_remote_urls_hint() {
        // #310: a command run on a local page fails before eval; the hint to
        // edit `remote.urls` does not apply to it either.
        let engine = engine_with_app_bridge();
        for page in [
            "about:blank",
            "data:text/html,hi",
            "blob:tauri://localhost/0",
            "file:///tmp/a.html",
        ] {
            let webviews = FakeWebviews::window("main", Some(page));

            let err = dispatch("snapshot", None, &engine, &webviews, &Recorder::new())
                .await
                .expect_err("a local page has no bridge");

            assert_eq!(
                err.message,
                format!(
                    "no pilot bridge on the current page ({page}). Bridge commands only \
                     work on tauri://localhost: navigate back there"
                )
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_data_url_never_left_reports_blocked() {
        // #310: a `data:` URL does not load over the network, so a window
        // still on the page was refused, not slow.
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE))
            .on_eval(move || engine_clone.resolve(1, Ok(json!({"ok": true}))));

        let err = dispatch(
            "navigate",
            Some(&json!({"url": "data:text/html,<p>hi</p>"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("a blocked data: navigation must not report ok");

        assert_eq!(
            err.message,
            "navigate to data:text/html,<p>hi</p> did not happen: the window still shows \
             tauri://localhost/ after 3s, so the navigation was cancelled or blocked"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_about_url_never_left_reports_blocked() {
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE))
            .on_eval(move || engine_clone.resolve(1, Ok(json!({"ok": true}))));

        let err = dispatch(
            "navigate",
            Some(&json!({"url": "about:blank"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("a blocked about: navigation must not report ok");

        assert_eq!(
            err.message,
            "navigate to about:blank did not happen: the window still shows \
             tauri://localhost/ after 3s, so the navigation was cancelled or blocked"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_file_and_blob_urls_never_left_report_blocked() {
        // #310: `file:` and `blob:` do not load over the network either, so
        // a window still on the page was refused, not slow.
        for dest in ["file:///tmp/a.html", "blob:tauri://localhost/0"] {
            let engine = engine_with_app_bridge();
            let engine_clone = engine.clone();
            let webviews = FakeWebviews::window("main", Some(APP_PAGE))
                .on_eval(move || engine_clone.resolve(1, Ok(json!({"ok": true}))));

            let err = dispatch(
                "navigate",
                Some(&json!({ "url": dest })),
                &engine,
                &webviews,
                &Recorder::new(),
            )
            .await
            .expect_err("a blocked local navigation must not report ok");

            assert_eq!(
                err.message,
                format!(
                    "navigate to {dest} did not happen: the window still shows \
                     tauri://localhost/ after 3s, so the navigation was cancelled or blocked"
                )
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_redirected_to_about_blank_omits_remote_urls_hint() {
        // #310: the hint is judged on the page the window shows, not on
        // `dest`. An https destination that lands on `about:blank` cannot be
        // fixed through `remote.urls`.
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE));
        let webviews_clone = webviews.clone();
        let webviews = webviews.on_eval(move || {
            engine_clone.resolve(1, Ok(json!({"ok": true})));
            webviews_clone.set_url("main", Some("about:blank"));
        });

        let err = dispatch(
            "navigate",
            Some(&json!({"url": FOREIGN_PAGE})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("about:blank has no bridge");

        assert_eq!(
            err.message,
            "navigate to https://example.com/: the window shows about:blank, but no pilot \
             bridge answered there within 3s. Bridge commands only work on \
             tauri://localhost: navigate back there"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_to_about_blank_landing_on_remote_page_keeps_hint() {
        // #310: a local `dest` does not drop the hint when the window shows
        // a remote page, which `remote.urls` can allow.
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE));
        let webviews_clone = webviews.clone();
        let webviews = webviews.on_eval(move || {
            engine_clone.resolve(1, Ok(json!({"ok": true})));
            webviews_clone.set_url("main", Some(FOREIGN_PAGE));
        });

        let err = dispatch(
            "navigate",
            Some(&json!({"url": "about:blank"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("a silent remote page must not report ok");

        assert_eq!(
            err.message,
            "navigate to about:blank: the window shows https://example.com/, but no pilot \
             bridge answered there within 3s. Bridge commands only work on \
             tauri://localhost: navigate back there, or allow this origin in a \
             capability's remote.urls"
        );
    }

    #[tokio::test]
    async fn test_dispatch_navigate_from_about_blank_omits_remote_urls_hint() {
        // #310: a navigate from a local start page fails before waiting
        // (`fail_no_bridge`); the hint does not apply there either.
        let engine = engine_with_app_bridge();
        let webviews = FakeWebviews::window("main", Some("about:blank"));

        let err = dispatch(
            "navigate",
            Some(&json!({"url": "about:blank"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("about:blank has no bridge");

        assert_eq!(
            err.message,
            "no pilot bridge on the current page (about:blank). Bridge commands only work \
             on tauri://localhost: navigate back there"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_cross_origin_redirected_to_bridged_origin_reports_ok() {
        // A redirect (an OAuth hop) can bring the window back to an origin
        // whose bridge answers: its hello proves commands work there.
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE));
        let webviews_clone = webviews.clone();
        let webviews = webviews.on_eval(move || {
            engine_clone.resolve(1, Ok(json!({"ok": true})));
            webviews_clone.set_url("main", Some("tauri://localhost/login"));
        });
        hello_later(
            &engine,
            "main",
            "tauri://localhost/login",
            Duration::from_millis(500),
        );

        let start = tokio::time::Instant::now();
        let result = dispatch(
            "navigate",
            Some(&json!({"url": FOREIGN_PAGE})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("a hello from the bridged origin the window reached is the proof");

        assert_eq!(result, json!({"ok": true}));
        assert!(
            start.elapsed() < BRIDGE_GRACE,
            "must return on the hello, took {:?}",
            start.elapsed()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_cross_origin_start_page_reload_is_not_ok() {
        // The start page reloads (dev hot-reload) while the destination
        // refuses the connection: its hello proves nothing about `dest`.
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE))
            .on_eval(move || engine_clone.resolve(1, Ok(json!({"ok": true}))));
        hello_later(&engine, "main", APP_PAGE, Duration::from_millis(500));

        let err = dispatch(
            "navigate",
            Some(&json!({"url": "http://127.0.0.1:1/"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("a reload of the start page must not report ok");

        assert_eq!(
            err.message,
            "navigate to http://127.0.0.1:1/ did not load in time: the window still shows \
             tauri://localhost/ after 3s"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_cross_origin_without_start_url_reports_page_never_left() {
        // The start URL could not be read, but the window shows a page of an
        // origin whose bridge answers and that said no new hello: the
        // destination did not load, the capability hint does not apply.
        let engine = engine_with_app_bridge();
        let webviews = FakeWebviews::window("main", None);
        let webviews_clone = webviews.clone();
        let webviews = webviews.on_eval(move || webviews_clone.set_url("main", Some(APP_PAGE)));

        let err = dispatch(
            "navigate",
            Some(&json!({"url": "http://127.0.0.1:1/"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("a navigation that never left the page must not report ok");

        assert_eq!(
            err.message,
            "navigate to http://127.0.0.1:1/ did not load in time: the window still shows \
             tauri://localhost/ after 3s"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_cross_origin_reports_page_never_left_after_url_change() {
        // The start page changed only its URL (`history.pushState`) during
        // the wait: the window still never left its bridged document.
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE));
        let webviews_clone = webviews.clone();
        let webviews = webviews.on_eval(move || {
            engine_clone.resolve(1, Ok(json!({"ok": true})));
            webviews_clone.set_url("main", Some("tauri://localhost/other"));
        });

        let err = dispatch(
            "navigate",
            Some(&json!({"url": "http://127.0.0.1:1/"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("a navigation that never left the document must not report ok");

        assert_eq!(
            err.message,
            "navigate to http://127.0.0.1:1/ did not load in time: the window still shows \
             tauri://localhost/other after 3s"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_cross_origin_redirected_to_other_bridged_origin_names_it() {
        // The window moved to another bridged origin than the start page's
        // and no hello came from it: it left the page, so this is no
        // "did not load".
        let engine = engine_with_app_bridge();
        handle_callback(
            &engine,
            "other",
            HELLO_ID,
            None,
            None,
            Some(&url("http://localhost:1420/")),
        );
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE));
        let webviews_clone = webviews.clone();
        let webviews = webviews.on_eval(move || {
            engine_clone.resolve(1, Ok(json!({"ok": true})));
            webviews_clone.set_url("main", Some("http://localhost:1420/login"));
        });

        let err = dispatch(
            "navigate",
            Some(&json!({"url": FOREIGN_PAGE})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("a silent redirect target must not report ok");

        assert_eq!(
            err.message,
            "navigate to https://example.com/: the window shows http://localhost:1420/login, \
             but no pilot bridge answered there within 3s. Bridge commands only work on \
             http://localhost:1420, tauri://localhost: navigate back there, or allow this \
             origin in a capability's remote.urls"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_without_start_url_reaching_dest_keeps_navigated_to() {
        // No start URL and the window shows `dest` itself: it navigated.
        let engine = engine_with_app_bridge();
        let webviews = FakeWebviews::window("main", None);
        let webviews_clone = webviews.clone();
        let webviews = webviews
            .on_eval(move || webviews_clone.set_url("main", Some("tauri://localhost/settings")));

        let err = dispatch(
            "navigate",
            Some(&json!({"url": "tauri://localhost/settings"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("a silent destination must not report ok");

        assert_eq!(
            err.message,
            "navigated to tauri://localhost/settings, but no pilot bridge answered there \
             within 10s. Bridge commands only work on tauri://localhost: navigate back \
             there, or allow this origin in a capability's remote.urls"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_without_start_url_on_bridgeless_page_names_it() {
        // No start URL and the window shows a page whose origin never said
        // hello: its silence proves nothing, so the window may have moved.
        let engine = engine_with_app_bridge();
        let webviews = FakeWebviews::window("main", None);
        let webviews_clone = webviews.clone();
        let webviews = webviews.on_eval(move || webviews_clone.set_url("main", Some(FOREIGN_PAGE)));

        let err = dispatch(
            "navigate",
            Some(&json!({"url": "http://127.0.0.1:1/"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("a silent destination must not report ok");

        assert_eq!(
            err.message,
            "navigate to http://127.0.0.1:1/: the window shows https://example.com/, but no \
             pilot bridge answered there within 3s. Bridge commands only work on \
             tauri://localhost: navigate back there, or allow this origin in a \
             capability's remote.urls"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_without_start_url_or_hello_does_not_claim_never_left() {
        // Before any hello the engine cannot tell which origins answer, so
        // it cannot conclude the window never left.
        let engine = EvalEngine::new();
        let webviews = FakeWebviews::window("main", None);
        let webviews_clone = webviews.clone();
        let webviews = webviews.on_eval(move || webviews_clone.set_url("main", Some(APP_PAGE)));

        let err = dispatch(
            "navigate",
            Some(&json!({"url": FOREIGN_PAGE})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("a silent destination must not report ok");

        assert!(
            err.message.starts_with(
                "navigate to https://example.com/: the window shows tauri://localhost/, but no \
                 pilot bridge answered there"
            ),
            "got: {}",
            err.message
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_cross_origin_from_bridgeless_page_names_working_origins() {
        // Stuck on a page without a bridge, the user still needs to know
        // where commands work, but not the capability hint.
        let engine = engine_with_app_bridge();
        let webviews = FakeWebviews::window("main", Some(FOREIGN_PAGE));

        let err = dispatch(
            "navigate",
            Some(&json!({"url": "http://127.0.0.1:1/"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("a navigation that never left the page must not report ok");

        assert_eq!(
            err.message,
            "navigate to http://127.0.0.1:1/ did not load in time: the window still shows \
             https://example.com/ after 3s. Bridge commands only work on tauri://localhost: \
             navigate back there"
        );
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
    async fn test_dispatch_after_origin_change_fails_fast_with_no_bridge() {
        // #173: the check reads target.url(), then eval queues the script.
        // A concurrent navigate (or a redirect) can replace the page in that
        // gap. The command must not run there, and the RPC must fail with
        // the no-bridge error rather than wait out DEFAULT_TIMEOUT.
        let engine = engine_with_app_bridge();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE));
        let pages = webviews.clone();
        let webviews = webviews.on_eval(move || {
            pages.set_url("main", Some(FOREIGN_PAGE));
        });

        let start = tokio::time::Instant::now();
        let err = dispatch(
            "click",
            Some(&json!({"ref": "e1"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("a command whose page was replaced must not succeed");

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
        assert!(
            err.message.contains("pinned") && err.message.contains("page changed"),
            "got: {}",
            err.message
        );
        assert!(
            !err.message.contains("no pilot bridge"),
            "got: {}",
            err.message
        );
        let scripts = webviews.scripts();
        assert_eq!(scripts.len(), 1, "the script is sent, then refused in-page");
        assert!(
            scripts[0].contains("location.protocol")
                && scripts[0].contains("localhost")
                && scripts[0].contains("tauri:"),
            "wrapper must pin the origin that passed the check; got: {}",
            scripts[0]
        );
        assert!(
            scripts[0].contains("window.__PILOT__.click"),
            "got: {}",
            scripts[0]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_timeout_on_same_origin_stays_eval_timeout() {
        // A silent page that did not navigate is still an eval timeout, not
        // the no-bridge error used when the origin moved (#173).
        let engine = engine_with_app_bridge();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE));

        let start = tokio::time::Instant::now();
        let err = dispatch("title", None, &engine, &webviews, &Recorder::new())
            .await
            .expect_err("no callback means timeout");

        assert!(
            start.elapsed() >= DEFAULT_TIMEOUT,
            "took {:?}",
            start.elapsed()
        );
        assert!(err.message.contains("timed out"), "got: {}", err.message);
        assert!(
            !err.message.contains("no pilot bridge"),
            "got: {}",
            err.message
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_timeout_on_same_origin_path_change_in_eval_stays_eval_timeout() {
        let engine = engine_with_app_bridge();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE));
        let pages = webviews.clone();
        let webviews = webviews.on_eval(move || {
            pages.set_url("main", Some("tauri://localhost/settings"));
        });

        let start = tokio::time::Instant::now();
        let err = dispatch("title", None, &engine, &webviews, &Recorder::new())
            .await
            .expect_err("no callback means timeout");

        assert!(
            start.elapsed() >= DEFAULT_TIMEOUT,
            "took {:?}",
            start.elapsed()
        );
        assert!(err.message.contains("timed out"), "got: {}", err.message);
        assert!(
            !err.message.contains("pinned") && !err.message.contains("no pilot bridge"),
            "got: {}",
            err.message
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_timeout_on_same_origin_path_change_after_delay_stays_eval_timeout() {
        let engine = engine_with_app_bridge();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE));
        let pages = webviews.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            pages.set_url("main", Some("tauri://localhost/settings"));
        });

        let start = tokio::time::Instant::now();
        let err = dispatch("title", None, &engine, &webviews, &Recorder::new())
            .await
            .expect_err("no callback means timeout");

        assert!(
            start.elapsed() >= DEFAULT_TIMEOUT,
            "took {:?}",
            start.elapsed()
        );
        assert!(err.message.contains("timed out"), "got: {}", err.message);
        assert!(
            !err.message.contains("pinned") && !err.message.contains("no pilot bridge"),
            "got: {}",
            err.message
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_after_origin_change_does_not_claim_new_page_has_no_bridge() {
        let engine = engine_with_app_bridge();
        handle_callback(
            &engine,
            "main",
            HELLO_ID,
            None,
            None,
            Some(&url(FOREIGN_PAGE)),
        );
        let webviews = FakeWebviews::window("main", Some(APP_PAGE));
        let pages = webviews.clone();
        let webviews = webviews.on_eval(move || {
            pages.set_url("main", Some(FOREIGN_PAGE));
        });

        let err = dispatch(
            "click",
            Some(&json!({"ref": "e1"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("a command whose page was replaced must not succeed");

        assert_eq!(err.code, -32603);
        assert!(
            err.message.contains("pinned") && err.message.contains("page changed"),
            "got: {}",
            err.message
        );
        assert!(
            !err.message.contains("no pilot bridge"),
            "got: {}",
            err.message
        );
        assert!(
            engine
                .bridge_origins()
                .iter()
                .any(|origin| origin.contains("example.com")),
            "new page already said hello: {:?}",
            engine.bridge_origins()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_timeout_after_late_origin_change_is_no_bridge() {
        // #173: a redirect after eval is queued is invisible until the
        // callback times out. Re-read the URL then, and surface no-bridge
        // instead of a generic eval timeout.
        let engine = engine_with_app_bridge();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE));
        let pages = webviews.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            pages.set_url("main", Some(FOREIGN_PAGE));
        });

        let start = tokio::time::Instant::now();
        let err = dispatch("title", None, &engine, &webviews, &Recorder::new())
            .await
            .expect_err("a command whose page was replaced must not succeed");

        assert!(
            start.elapsed() >= DEFAULT_TIMEOUT,
            "took {:?}",
            start.elapsed()
        );
        assert_eq!(err.code, -32603);
        assert!(err.message.contains(FOREIGN_PAGE), "got: {}", err.message);
        assert!(
            err.message.contains("pinned") && err.message.contains("page changed"),
            "got: {}",
            err.message
        );
        assert!(
            !err.message.contains("no pilot bridge"),
            "got: {}",
            err.message
        );
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
                handle_callback(&engine, "main", HELLO_ID, None, None, Some(&url(APP_PAGE)));
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
        let scripts = webviews.scripts();
        assert_eq!(scripts.len(), 1);
        assert!(
            !scripts[0].contains("location.protocol"),
            "navigate must not pin origin; got: {}",
            scripts[0]
        );
        assert!(scripts[0].contains(APP_PAGE), "got: {}", scripts[0]);
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_within_app_origin_assigns_absolute_url() {
        let engine = engine_with_app_bridge();
        let webviews = navigating_webviews(
            &engine,
            APP_PAGE,
            "tauri://localhost/settings",
            Duration::ZERO,
        );

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
        let scripts = webviews.scripts();
        assert_eq!(scripts.len(), 1);
        assert!(
            scripts[0].contains("tauri://localhost/settings"),
            "relative dest must be assigned as the absolute URL; got: {}",
            scripts[0]
        );
        assert!(
            !scripts[0].contains("location.protocol"),
            "navigate must not pin origin; got: {}",
            scripts[0]
        );
    }

    /// Webviews on `page` whose eval answers the navigate callback at once,
    /// as the departing page does, and says hello from `dest` after `delay`.
    fn navigating_webviews(
        engine: &EvalEngine,
        page: &str,
        dest: &'static str,
        delay: Duration,
    ) -> FakeWebviews {
        let engine = engine.clone();
        FakeWebviews::window("main", Some(page)).on_eval(move || {
            let engine = engine.clone();
            engine.resolve(1, Ok(json!({"ok": true})));
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                handle_callback(&engine, "main", HELLO_ID, None, None, Some(&url(dest)));
            });
        })
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_within_app_origin_waits_for_destination_hello() {
        // #260: the departing page answers before the load starts, so a
        // same-origin navigate must wait for the destination's hello.
        let engine = engine_with_app_bridge();
        let delay = Duration::from_millis(500);
        let webviews =
            navigating_webviews(&engine, APP_PAGE, "tauri://localhost/settings.html", delay);

        let start = tokio::time::Instant::now();
        let result = dispatch(
            "navigate",
            Some(&json!({"url": "/settings.html"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("same-origin navigate succeeds once the destination says hello");
        assert_eq!(result, json!({"ok": true}));
        assert!(
            start.elapsed() >= delay,
            "returned before the destination loaded, after {:?}",
            start.elapsed()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_to_known_origin_waits_for_destination_hello() {
        // #260: an origin that said hello before still needs this load's hello.
        let engine = engine_with_app_bridge();
        let dest = "https://allowed.example/";
        handle_callback(&engine, "main", HELLO_ID, None, None, Some(&url(dest)));
        let delay = Duration::from_millis(500);
        let webviews = navigating_webviews(&engine, APP_PAGE, dest, delay);

        let start = tokio::time::Instant::now();
        let result = dispatch(
            "navigate",
            Some(&json!({"url": dest})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("navigate to a known origin succeeds once it says hello");
        assert_eq!(result, json!({"ok": true}));
        assert!(
            start.elapsed() >= delay,
            "returned before the destination loaded, after {:?}",
            start.elapsed()
        );
    }

    /// Two app windows: the navigate targets `popup`, and `main` (same
    /// origin) says hello once the departing page answered. `popup` itself
    /// says hello only when `popup_hello` is set.
    fn two_window_navigate(
        engine: &EvalEngine,
        dest: &'static str,
        popup_hello: bool,
    ) -> FakeWebviews {
        let engine = engine.clone();
        FakeWebviews::windows(&[("main", Some(APP_PAGE)), ("popup", Some(APP_PAGE))]).on_eval(
            move || {
                let engine = engine.clone();
                engine.resolve(1, Ok(json!({"ok": true})));
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    handle_callback(&engine, "main", HELLO_ID, None, None, Some(&url(APP_PAGE)));
                    if popup_hello {
                        tokio::time::sleep(Duration::from_millis(400)).await;
                        handle_callback(&engine, "popup", HELLO_ID, None, None, Some(&url(dest)));
                    }
                });
            },
        )
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_ignores_same_origin_hello_from_another_window() {
        // A hello from another window on the same origin is not the target's
        // new document, so navigate must keep waiting and fail.
        let engine = engine_with_app_bridge();
        let webviews = two_window_navigate(&engine, "tauri://localhost/settings.html", false);

        let start = tokio::time::Instant::now();
        let result = dispatch(
            "navigate",
            Some(&json!({"url": "/settings.html", "window": "popup"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await;
        assert!(
            result.is_err(),
            "another window's hello must not report ok, got {result:?}"
        );
        assert!(
            start.elapsed() >= DEFAULT_TIMEOUT,
            "took {:?}",
            start.elapsed()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_succeeds_on_target_window_hello() {
        let engine = engine_with_app_bridge();
        let webviews = two_window_navigate(&engine, "tauri://localhost/settings.html", true);

        let start = tokio::time::Instant::now();
        let result = dispatch(
            "navigate",
            Some(&json!({"url": "/settings.html", "window": "popup"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("the target window's hello reports ok");
        assert_eq!(result, json!({"ok": true}));
        assert!(
            start.elapsed() >= Duration::from_millis(500),
            "returned on the other window's hello, after {:?}",
            start.elapsed()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_cross_origin_ignores_hello_from_another_window() {
        // Same race on the cross-origin path: `main` already shows the
        // destination origin and reloads while `popup` navigates there.
        let engine = engine_with_app_bridge();
        let dest = "https://allowed.example/";
        handle_callback(&engine, "main", HELLO_ID, None, None, Some(&url(dest)));
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::windows(&[("main", Some(dest)), ("popup", Some(APP_PAGE))])
            .on_eval(move || {
                let engine = engine_clone.clone();
                engine.resolve(1, Ok(json!({"ok": true})));
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    handle_callback(&engine, "main", HELLO_ID, None, None, Some(&url(dest)));
                });
            });

        let result = dispatch(
            "navigate",
            Some(&json!({"url": dest, "window": "popup"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await;
        assert!(
            result.is_err(),
            "another window's hello must not report ok, got {result:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_within_app_origin_fails_without_destination_hello() {
        // #260: the departing page's `{ok: true}` is not proof of arrival.
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE))
            .on_eval(move || engine_clone.resolve(1, Ok(json!({"ok": true}))));

        let start = tokio::time::Instant::now();
        let err = dispatch(
            "navigate",
            Some(&json!({"url": "/settings.html"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("no destination hello must not report ok");
        assert!(
            start.elapsed() >= DEFAULT_TIMEOUT,
            "took {:?}",
            start.elapsed()
        );
        // The window never left the page, so the navigation did not happen.
        assert_eq!(
            err.message,
            "navigate to tauri://localhost/settings.html did not happen: the window \
             still shows tauri://localhost/ after 10s, so the navigation was cancelled \
             or blocked"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_within_app_origin_reports_silent_new_document() {
        // The window reached the new document, but its bridge stayed silent.
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE));
        let webviews_clone = webviews.clone();
        let webviews = webviews.on_eval(move || {
            engine_clone.resolve(1, Ok(json!({"ok": true})));
            webviews_clone.set_url("main", Some("tauri://localhost/settings.html"));
        });

        let err = dispatch(
            "navigate",
            Some(&json!({"url": "/settings.html"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("a silent new document must not report ok");
        assert_eq!(
            err.message,
            "navigate to tauri://localhost/settings.html loaded a new document \
             (tauri://localhost/settings.html), but its pilot bridge never said hello \
             within 10s"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_within_app_origin_accepts_redirect_to_bridged_origin() {
        // A same-origin URL whose server redirects to another origin with a
        // working bridge must still succeed on that origin's hello.
        let engine = engine_with_app_bridge();
        let redirect = "https://auth.example/login";
        handle_callback(&engine, "main", HELLO_ID, None, None, Some(&url(redirect)));
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE));
        let webviews_clone = webviews.clone();
        let delay = Duration::from_millis(500);
        let webviews = webviews.on_eval(move || {
            let engine = engine_clone.clone();
            engine.resolve(1, Ok(json!({"ok": true})));
            webviews_clone.set_url("main", Some(redirect));
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                handle_callback(&engine, "main", HELLO_ID, None, None, Some(&url(redirect)));
            });
        });

        let start = tokio::time::Instant::now();
        let result = dispatch(
            "navigate",
            Some(&json!({"url": "/logout"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("a redirect to a bridged origin succeeds on its hello");
        assert_eq!(result, json!({"ok": true}));
        assert!(
            start.elapsed() >= delay && start.elapsed() < DEFAULT_TIMEOUT,
            "took {:?}",
            start.elapsed()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_within_app_origin_survives_departure_timeout() {
        // WebKitGTK can tear the page down before `__callback` runs.
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let hello_after = Duration::from_secs(4);
        let webviews = FakeWebviews::window("main", Some(APP_PAGE)).on_eval(move || {
            let engine = engine_clone.clone();
            tokio::spawn(async move {
                tokio::time::sleep(hello_after).await;
                handle_callback(&engine, "main", HELLO_ID, None, None, Some(&url(APP_PAGE)));
            });
        });

        let start = tokio::time::Instant::now();
        let result = dispatch(
            "navigate",
            Some(&json!({"url": "/settings.html"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("a lost departure callback is not fatal when the dest says hello");
        assert_eq!(result, json!({"ok": true}));
        assert!(
            start.elapsed() >= BRIDGE_GRACE,
            "took {:?}",
            start.elapsed()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_reload_waits_for_hello() {
        // `navigate <same url>` reloads, so the new document must say hello.
        let engine = engine_with_app_bridge();
        let delay = Duration::from_millis(500);
        let webviews = navigating_webviews(&engine, APP_PAGE, APP_PAGE, delay);

        let start = tokio::time::Instant::now();
        let result = dispatch(
            "navigate",
            Some(&json!({"url": APP_PAGE})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("reload succeeds once the page says hello again");
        assert_eq!(result, json!({"ok": true}));
        assert!(start.elapsed() >= delay, "took {:?}", start.elapsed());
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_cross_origin_before_any_hello_resolves_on_callback() {
        // Same rule as same-origin: without a hello the engine cannot know
        // one will come, so a cross-origin navigate keeps the plain path.
        let engine = EvalEngine::new();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE))
            .on_eval(move || engine_clone.resolve(1, Ok(json!({"ok": true}))));

        let start = tokio::time::Instant::now();
        let result = dispatch(
            "navigate",
            Some(&json!({"url": FOREIGN_PAGE})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("cross-origin navigate before any hello resolves on the callback");
        assert_eq!(result, json!({"ok": true}));
        assert_eq!(start.elapsed(), Duration::ZERO, "must not wait for a hello");
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_before_any_hello_resolves_on_callback() {
        // Without a hello the engine cannot tell whether hellos will come,
        // so every command, navigate included, keeps the plain path.
        let engine = EvalEngine::new();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE))
            .on_eval(move || engine_clone.resolve(1, Ok(json!({"ok": true}))));

        let result = dispatch(
            "navigate",
            Some(&json!({"url": "/settings.html"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("navigate before any hello resolves on the callback");
        assert_eq!(result, json!({"ok": true}));
    }

    /// Webviews on `start` at app startup: the start page says hello only
    /// once the navigate script is queued, then calls back. `after` runs
    /// right after the callback, before `dispatch` reads it.
    fn startup_webviews(
        engine: &EvalEngine,
        windows: &[(&str, Option<&str>)],
        target: &'static str,
        start: &'static str,
        after: impl Fn(&EvalEngine) + Send + Sync + 'static,
    ) -> FakeWebviews {
        let engine = engine.clone();
        FakeWebviews::windows(windows).on_eval(move || {
            handle_callback(&engine, target, HELLO_ID, None, None, Some(&url(start)));
            engine.resolve(1, Ok(json!({"ok": true})));
            after(&engine);
        })
    }

    /// Say hello from `dest` in webview `label` after `delay`.
    fn hello_later(engine: &EvalEngine, label: &'static str, dest: &'static str, delay: Duration) {
        let engine = engine.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            handle_callback(&engine, label, HELLO_ID, None, None, Some(&url(dest)));
        });
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_at_startup_waits_for_destination_hello() {
        // #270: right after startup no hello has arrived when navigate
        // starts, but the start page says hello before it can call back.
        let engine = EvalEngine::new();
        let delay = Duration::from_millis(500);
        let webviews = startup_webviews(
            &engine,
            &[("main", Some(APP_PAGE))],
            "main",
            APP_PAGE,
            move |engine| hello_later(engine, "main", "tauri://localhost/settings.html", delay),
        );

        let start = tokio::time::Instant::now();
        let result = dispatch(
            "navigate",
            Some(&json!({"url": "/settings.html"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("navigate at startup succeeds once the destination says hello");
        assert_eq!(result, json!({"ok": true}));
        assert!(
            start.elapsed() >= delay,
            "returned before the destination loaded, after {:?}",
            start.elapsed()
        );
    }

    /// Two app windows at startup: navigate targets `popup`. `main` says
    /// hello again at 100 ms, and `popup` says hello from `dest` at 500 ms
    /// when `popup_hello` is set.
    fn startup_two_windows(
        engine: &EvalEngine,
        dest: &'static str,
        popup_hello: bool,
    ) -> FakeWebviews {
        let windows = [("main", Some(APP_PAGE)), ("popup", Some(APP_PAGE))];
        startup_webviews(engine, &windows, "popup", APP_PAGE, move |engine| {
            handle_callback(engine, "main", HELLO_ID, None, None, Some(&url(APP_PAGE)));
            hello_later(engine, "main", APP_PAGE, Duration::from_millis(100));
            if popup_hello {
                hello_later(engine, "popup", dest, Duration::from_millis(500));
            }
        })
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_at_startup_waits_for_target_window_hello() {
        let engine = EvalEngine::new();
        let webviews = startup_two_windows(&engine, "tauri://localhost/settings.html", true);

        let start = tokio::time::Instant::now();
        let result = dispatch(
            "navigate",
            Some(&json!({"url": "/settings.html", "window": "popup"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("the target window's hello reports ok");
        assert_eq!(result, json!({"ok": true}));
        assert!(
            start.elapsed() >= Duration::from_millis(500),
            "returned before the target window's hello, after {:?}",
            start.elapsed()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_at_startup_ignores_hello_from_another_window() {
        let engine = EvalEngine::new();
        let webviews = startup_two_windows(&engine, "tauri://localhost/settings.html", false);

        let start = tokio::time::Instant::now();
        let result = dispatch(
            "navigate",
            Some(&json!({"url": "/settings.html", "window": "popup"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await;
        assert!(
            result.is_err(),
            "another window's hello must not report ok, got {result:?}"
        );
        assert!(
            start.elapsed() >= DEFAULT_TIMEOUT,
            "took {:?}",
            start.elapsed()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_at_startup_to_known_origin_waits_for_destination_hello() {
        // Cross-origin at startup: `side` already shows the destination
        // origin and says hello there, but only `main`'s hello counts.
        let engine = EvalEngine::new();
        let dest = "https://allowed.example/";
        let delay = Duration::from_millis(500);
        let windows = [("main", Some(APP_PAGE)), ("side", Some(dest))];
        let webviews = startup_webviews(&engine, &windows, "main", APP_PAGE, move |engine| {
            handle_callback(engine, "side", HELLO_ID, None, None, Some(&url(dest)));
            hello_later(engine, "main", dest, delay);
        });

        let start = tokio::time::Instant::now();
        let result = dispatch(
            "navigate",
            Some(&json!({"url": dest, "window": "main"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("navigate to a known origin succeeds once main says hello there");
        assert_eq!(result, json!({"ok": true}));
        assert!(
            start.elapsed() >= delay,
            "returned before the destination loaded, after {:?}",
            start.elapsed()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_at_startup_to_foreign_origin_does_not_report_ok() {
        // #153 at startup: once the start page said hello, a navigate to an
        // origin that never says hello fails like any later navigate.
        let engine = EvalEngine::new();
        let webviews = startup_webviews(
            &engine,
            &[("main", Some(APP_PAGE))],
            "main",
            APP_PAGE,
            |_| {},
        );

        let start = tokio::time::Instant::now();
        let err = dispatch(
            "navigate",
            Some(&json!({"url": FOREIGN_PAGE})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("navigate at startup to a silent foreign origin must not report ok");

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
    async fn test_dispatch_navigate_at_startup_cross_origin_reports_page_never_left() {
        // #278 at startup: the window never left the start page.
        let engine = EvalEngine::new();
        let webviews = startup_webviews(
            &engine,
            &[("main", Some(APP_PAGE))],
            "main",
            APP_PAGE,
            |_| {},
        );

        let err = dispatch(
            "navigate",
            Some(&json!({"url": "http://127.0.0.1:1/"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("a navigation that never left the page must not report ok");

        assert_eq!(
            err.message,
            "navigate to http://127.0.0.1:1/ did not load in time: the window still shows \
             tauri://localhost/ after 3s"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_at_startup_to_new_origin_reports_ok_on_its_hello() {
        // A foreign origin allowed in `remote.urls` says hello within the
        // grace on its first visit, so the startup navigate reports ok then.
        let engine = EvalEngine::new();
        let dest = "https://allowed.example/";
        let delay = Duration::from_millis(500);
        let webviews = startup_webviews(
            &engine,
            &[("main", Some(APP_PAGE))],
            "main",
            APP_PAGE,
            move |engine| {
                hello_later(engine, "main", dest, delay);
            },
        );

        let start = tokio::time::Instant::now();
        let result = dispatch(
            "navigate",
            Some(&json!({"url": dest})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("navigate to a new origin succeeds once it says hello");
        assert_eq!(result, json!({"ok": true}));
        assert!(
            start.elapsed() >= delay,
            "returned before the destination loaded, after {:?}",
            start.elapsed()
        );
        assert!(start.elapsed() < BRIDGE_GRACE, "took {:?}", start.elapsed());
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_at_startup_counts_destination_hello_before_dispatch_resumes() {
        // The destination's hello can be recorded before `dispatch` reads
        // the callback. It must still count as the destination's hello.
        let engine = EvalEngine::new();
        let webviews = startup_webviews(
            &engine,
            &[("main", Some(APP_PAGE))],
            "main",
            APP_PAGE,
            |engine| {
                let dest = url("tauri://localhost/settings.html");
                handle_callback(engine, "main", HELLO_ID, None, None, Some(&dest));
            },
        );

        let start = tokio::time::Instant::now();
        let result = dispatch(
            "navigate",
            Some(&json!({"url": "/settings.html"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("a destination hello recorded early still counts");
        assert_eq!(result, json!({"ok": true}));
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "took {:?}",
            start.elapsed()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_at_startup_without_hello_resolves_on_callback() {
        // The page calls back but no hello was recorded (still in flight,
        // or dropped on an origin mismatch): no proof a hello will come,
        // keep the plain path.
        let engine = EvalEngine::new();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE))
            .on_eval(move || engine_clone.resolve(1, Ok(json!({"ok": true}))));

        let start = tokio::time::Instant::now();
        let result = dispatch(
            "navigate",
            Some(&json!({"url": "/settings.html"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("navigate without any hello resolves on the callback");
        assert_eq!(result, json!({"ok": true}));
        assert_eq!(start.elapsed(), Duration::ZERO, "must not wait for a hello");
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_at_startup_in_same_document_resolves_on_callback() {
        // A `javascript:` URL or a fragment change keeps the document, so
        // no new hello will come even though the start page said hello.
        let page = "tauri://localhost/index.html";
        for raw in ["javascript:void(0)", "#section"] {
            let engine = EvalEngine::new();
            let webviews = startup_webviews(&engine, &[("main", Some(page))], "main", page, |_| {});

            let start = tokio::time::Instant::now();
            let result = dispatch(
                "navigate",
                Some(&json!({"url": raw})),
                &engine,
                &webviews,
                &Recorder::new(),
            )
            .await
            .expect("navigate in the same document resolves on the callback");
            assert_eq!(result, json!({"ok": true}), "{raw}");
            assert_eq!(start.elapsed(), Duration::ZERO, "{raw} must not wait");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_within_app_origin_reports_departing_page_error() {
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some(APP_PAGE))
            .on_eval(move || engine_clone.resolve(1, Err("boom".to_owned())));

        let err = dispatch(
            "navigate",
            Some(&json!({"url": "/settings.html"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect_err("an error from the departing page must be reported");
        assert!(err.message.contains("boom"), "got: {}", err.message);
    }

    #[tokio::test(start_paused = true)]
    async fn test_dispatch_navigate_to_fragment_resolves_on_callback() {
        // A fragment change keeps the document, so no hello will come.
        let engine = engine_with_app_bridge();
        let engine_clone = engine.clone();
        let webviews = FakeWebviews::window("main", Some("tauri://localhost/index.html"))
            .on_eval(move || engine_clone.resolve(1, Ok(json!({"ok": true}))));

        let start = tokio::time::Instant::now();
        let result = dispatch(
            "navigate",
            Some(&json!({"url": "#section"})),
            &engine,
            &webviews,
            &Recorder::new(),
        )
        .await
        .expect("fragment navigate stays in the document");
        assert_eq!(result, json!({"ok": true}));
        assert_eq!(start.elapsed(), Duration::ZERO, "must not wait for a hello");
    }

    #[test]
    fn test_loads_new_document_only_for_document_changes() {
        let page = url("tauri://localhost/index.html?tab=1#top");
        let cases = [
            ("tauri://localhost/index.html?tab=1#other", None, false),
            ("tauri://localhost/index.html?tab=1#top", None, false),
            ("tauri://localhost/index.html?tab=1", None, true),
            ("tauri://localhost/index.html?tab=2#top", None, true),
            ("tauri://localhost/settings.html", None, true),
            (
                "tauri://localhost/index.html?tab=1#top",
                Some("javascript:void(0)"),
                false,
            ),
        ];
        for (dest, raw, expected) in cases {
            assert_eq!(
                loads_new_document(&page, &url(dest), raw),
                expected,
                "{dest} (raw {raw:?})"
            );
        }
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
                handle_callback(&engine, "main", HELLO_ID, None, None, Some(&url(dest)));
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
            "main",
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
            "main",
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
                handle_callback(&engine, "main", HELLO_ID, None, None, Some(&url(dest)));
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

    /// An in-flight hello with no client result must not mark the destination.
    ///
    /// The live webview URL is already the destination. Reading it would tag
    /// that origin, and on Android it would also wait on the main thread
    /// while the plugin store lock is held (#252). The hello has to use the
    /// URL recorded when the previous page started loading.
    #[cfg(all(unix, not(target_os = "android"), debug_assertions))]
    #[test]
    fn in_flight_hello_without_result_does_not_mark_destination() {
        use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

        use crate::webview::url_gate::{LABEL, UrlGate};

        let gate = UrlGate::install();
        let _gate_guard = scopeguard_release(gate.clone());
        let (_socket, app) = callback_test_app();
        let window =
            WebviewWindowBuilder::new(&app, LABEL, WebviewUrl::External(url(FOREIGN_PAGE)))
                .data_directory(std::env::temp_dir())
                .build()
                .expect("url-wait window");
        let live = window.url().expect("live url");
        assert_eq!(
            origin_key(&live),
            origin_key(&url(FOREIGN_PAGE)),
            "the fixture's live URL must be the destination"
        );

        let engine = app.state::<EvalEngine>();
        engine.record_page_start(LABEL, url(APP_PAGE));
        for command in ["plugin:pilot|__callback", "plugin:pilot|callback"] {
            let before = engine.hellos();
            invoke_hello(&window, &gate, command);
            assert!(!gate.entered(), "{command} read the live webview URL");
            assert_eq!(
                engine.hellos(),
                before + 1,
                "{command} did not record the hello"
            );
            assert!(
                engine.has_bridge(&url(APP_PAGE)),
                "{command} dropped the page recorded at start"
            );
            assert!(
                !engine.has_bridge(&url(FOREIGN_PAGE)),
                "{command} marked the destination from an in-flight hello with no result"
            );
        }
    }

    /// App whose plugin socket is removed when the guard drops.
    #[cfg(all(unix, not(target_os = "android"), debug_assertions))]
    fn callback_test_app() -> (SocketCleanup, tauri::App<tauri::test::MockRuntime>) {
        let identifier = format!("com.pilot.issue252-{}", std::process::id());
        let address = crate::server::socket_address(&identifier).expect("socket address");
        let path = address
            .as_pathname()
            .expect("pathname socket")
            .to_path_buf();
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context.config_mut().identifier = identifier;
        for command in ["plugin:pilot|__callback", "plugin:pilot|callback"] {
            context.runtime_authority_mut().__allow_command(
                command.to_owned(),
                tauri::utils::acl::ExecutionContext::Local,
            );
        }
        let app = tauri::test::mock_builder()
            .plugin(crate::init())
            .build(context)
            .expect("app starts");
        (SocketCleanup(path), app)
    }

    #[cfg(all(unix, not(target_os = "android"), debug_assertions))]
    struct SocketCleanup(std::path::PathBuf);

    #[cfg(all(unix, not(target_os = "android"), debug_assertions))]
    impl Drop for SocketCleanup {
        fn drop(&mut self) {
            crate::server::unix::cleanup_bind_files(&self.0);
        }
    }

    /// Send a hello with no result. Returns once the command does.
    ///
    /// A command that calls `current_url` blocks in the URL gate, so this
    /// waits only long enough for a lock-free callback to finish.
    #[cfg(all(unix, not(target_os = "android"), debug_assertions))]
    fn invoke_hello(
        window: &tauri::WebviewWindow<tauri::test::MockRuntime>,
        gate: &crate::webview::url_gate::UrlGate,
        command: &str,
    ) {
        let window = window.clone();
        let command_owned = command.to_owned();
        let ipc = std::thread::spawn(move || {
            window.on_message(
                tauri::webview::InvokeRequest {
                    cmd: command_owned,
                    callback: tauri::ipc::CallbackFn(0),
                    error: tauri::ipc::CallbackFn(1),
                    url: url(APP_PAGE),
                    body: tauri::ipc::InvokeBody::Json(serde_json::json!({ "id": 0 })),
                    headers: tauri::http::HeaderMap::default(),
                    invoke_key: tauri::test::INVOKE_KEY.to_owned(),
                },
                Box::new(|_webview, _cmd, _response, _callback, _error| {}),
            );
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !ipc.is_finished() {
            assert!(!gate.entered(), "{command} read the live webview URL");
            assert!(
                std::time::Instant::now() <= deadline,
                "{command} did not return; it is waiting on the webview URL"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        ipc.join().expect("callback thread");
    }

    #[cfg(all(unix, not(target_os = "android"), debug_assertions))]
    fn scopeguard_release(gate: crate::webview::url_gate::UrlGate) -> impl Drop {
        struct Guard(crate::webview::url_gate::UrlGate);
        impl Drop for Guard {
            fn drop(&mut self) {
                self.0.release();
                crate::webview::url_gate::clear();
            }
        }
        Guard(gate)
    }
}
