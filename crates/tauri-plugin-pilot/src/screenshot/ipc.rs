//! JSON-RPC handler for `screenshot_native`.
//!
//! The contract (path-only, `window_id`-targeted, atomic write, TCC-fallback
//! surfaced as metadata) is implemented in [`handle_screenshot`]. The handler
//! validates the request shape on every platform, then dispatches to the
//! macOS-only native capture pipeline; non-macOS callers always receive
//! `PERMISSION_DENIED` so the contract surface is identical on every host.

#[cfg(target_os = "macos")]
use std::path::Path;
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;
#[cfg(target_os = "macos")]
use serde_json::json;

use crate::protocol::{RPC_INTERNAL_ERROR, RPC_INVALID_PARAMS, RpcError};

/// Domain error codes carried in the JSON-RPC `error.data.error` field.
///
/// Kept centralized so tests, the implementation, and the docs cannot drift.
pub(crate) mod codes {
    pub(crate) const INVALID_PARAMS: &str = "INVALID_PARAMS";
    pub(crate) const INVALID_OUTPUT_PATH: &str = "INVALID_OUTPUT_PATH";
    pub(crate) const UNSUPPORTED_FORMAT: &str = "UNSUPPORTED_FORMAT";
    #[cfg_attr(
        not(target_os = "macos"),
        allow(dead_code, reason = "only referenced by the macOS capture path")
    )]
    pub(crate) const WINDOW_NOT_FOUND: &str = "WINDOW_NOT_FOUND";
    #[cfg_attr(
        not(target_os = "macos"),
        allow(dead_code, reason = "only referenced by the macOS capture path")
    )]
    pub(crate) const CAPTURE_FAILED: &str = "CAPTURE_FAILED";
    pub(crate) const PERMISSION_DENIED: &str = "PERMISSION_DENIED";
}

/// Build a JSON-RPC error response with the given numeric code, message, and
/// domain error code carried in `data.error`. Extra structured detail can be
/// merged in via `extras`.
fn rpc_error(rpc_code: i32, domain: &str, message: impl Into<String>, extras: Value) -> RpcError {
    let message = message.into();
    let mut data = serde_json::Map::new();
    data.insert("error".to_owned(), Value::String(domain.to_owned()));
    data.insert("message".to_owned(), Value::String(message.clone()));
    if let Value::Object(obj) = extras {
        for (k, v) in obj {
            data.insert(k, v);
        }
    }
    RpcError {
        code: rpc_code,
        message,
        data: Some(Value::Object(data)),
    }
}

/// Parsed view of the `screenshot_native` request params with all field-level
/// validation already enforced by the validators below.
#[cfg_attr(
    not(target_os = "macos"),
    allow(
        dead_code,
        reason = "fields are read by the macOS-only capture pipeline"
    )
)]
struct ScreenshotRequest {
    window_id: u32,
    output_path: std::path::PathBuf,
}

/// Validate the request envelope. Order of checks is fixed by the contract:
///   1. `format` must be `"png"` if present (`UNSUPPORTED_FORMAT`)
///   2. `window_id` must be a u32 (`INVALID_PARAMS`)
///   3. `output_path` must be a string, absolute, with an existing parent
///      directory (`INVALID_OUTPUT_PATH`)
fn parse_request(params: Option<&Value>) -> Result<ScreenshotRequest, RpcError> {
    let obj = params.and_then(Value::as_object).ok_or_else(|| {
        rpc_error(
            RPC_INVALID_PARAMS,
            codes::INVALID_PARAMS,
            "screenshot requires an object params payload",
            Value::Null,
        )
    })?;

    // `format` is validated before `window_id` so callers using an unsupported
    // codec see the specific error rather than a generic missing-field one
    // when they also forgot `window_id`.
    let format = obj.get("format").and_then(Value::as_str).unwrap_or("png");
    if format != "png" {
        return Err(rpc_error(
            RPC_INVALID_PARAMS,
            codes::UNSUPPORTED_FORMAT,
            format!("format \"{format}\" is not supported in v1 (only \"png\")"),
            Value::Null,
        ));
    }

    let window_id_value = obj.get("window_id").ok_or_else(|| {
        rpc_error(
            RPC_INVALID_PARAMS,
            codes::INVALID_PARAMS,
            "screenshot requires a numeric \"window_id\"",
            Value::Null,
        )
    })?;
    let window_id_u64 = window_id_value.as_u64().ok_or_else(|| {
        rpc_error(
            RPC_INVALID_PARAMS,
            codes::INVALID_PARAMS,
            "\"window_id\" must be a non-negative integer",
            Value::Null,
        )
    })?;
    let window_id = u32::try_from(window_id_u64).map_err(|_| {
        rpc_error(
            RPC_INVALID_PARAMS,
            codes::INVALID_PARAMS,
            "\"window_id\" overflows u32",
            Value::Null,
        )
    })?;

    let output_path = parse_output_path(obj)?;

    Ok(ScreenshotRequest {
        window_id,
        output_path,
    })
}

/// Pull `output_path` out of the request payload and apply the security
/// contract: must be a string, absolute, not a symlink, and live under an
/// existing parent directory.
fn parse_output_path(obj: &serde_json::Map<String, Value>) -> Result<std::path::PathBuf, RpcError> {
    let output_path_str = obj
        .get("output_path")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            rpc_error(
                RPC_INVALID_PARAMS,
                codes::INVALID_OUTPUT_PATH,
                "screenshot requires a string \"output_path\"",
                Value::Null,
            )
        })?;
    let output_path = std::path::PathBuf::from(output_path_str);
    if !output_path.is_absolute() {
        return Err(rpc_error(
            RPC_INVALID_PARAMS,
            codes::INVALID_OUTPUT_PATH,
            format!(
                "output_path must be absolute, got \"{}\"",
                output_path.display()
            ),
            Value::Null,
        ));
    }
    // A pre-existing symlink at `output_path` is a footgun: the atomic rename
    // would replace the symlink itself (so the link's target is unchanged), but
    // a future read-back through the same path follows the link to wherever the
    // attacker last pointed it. Refusing symlinks here makes the contract
    // ("output_path is the file on disk") hold for the caller.
    if output_path.is_symlink() {
        return Err(rpc_error(
            RPC_INVALID_PARAMS,
            codes::INVALID_OUTPUT_PATH,
            format!(
                "output_path must not be a symlink: {}",
                output_path.display()
            ),
            Value::Null,
        ));
    }
    if output_path.is_dir() {
        return Err(rpc_error(
            RPC_INVALID_PARAMS,
            codes::INVALID_OUTPUT_PATH,
            format!(
                "output_path must be a file path, got directory: {}",
                output_path.display()
            ),
            Value::Null,
        ));
    }
    let parent = output_path.parent().ok_or_else(|| {
        rpc_error(
            RPC_INVALID_PARAMS,
            codes::INVALID_OUTPUT_PATH,
            format!(
                "output_path \"{}\" has no parent directory",
                output_path.display()
            ),
            Value::Null,
        )
    })?;
    // Refuse to create parent directories: an absolute path beneath a
    // non-existent directory could be an attacker-controlled prefix on a
    // shared host. Caller must pre-create the destination directory.
    if !parent.as_os_str().is_empty() && !parent.is_dir() {
        return Err(rpc_error(
            RPC_INVALID_PARAMS,
            codes::INVALID_OUTPUT_PATH,
            format!(
                "output_path parent directory does not exist: {}",
                parent.display()
            ),
            Value::Null,
        ));
    }
    Ok(output_path)
}

/// Top-level entry point used by the JSON-RPC dispatcher.
///
/// Returns the contract-shaped success result or a structured `RpcError`. The
/// success result fills the response with `output_path`, `window_id`, pixel
/// dimensions, `scale_factor`, `byte_size`, the chosen `backend`, and the
/// `tcc_denied` flag.
///
/// `scale_factor` is `null` when the capture's two axes disagree on the ratio
/// against the window's logical bounds — the PNG is then not the window, so
/// the number would not describe it. See `compute_scale_factor`.
pub(crate) async fn handle_screenshot(params: Option<&Value>) -> Result<Value, RpcError> {
    let req = parse_request(params)?;

    #[cfg(target_os = "macos")]
    {
        tokio::task::spawn_blocking(move || run_macos(req))
            .await
            .map_err(|join_err| {
                rpc_error(
                    RPC_INTERNAL_ERROR,
                    codes::CAPTURE_FAILED,
                    format!("screenshot capture task did not complete: {join_err}"),
                    Value::Null,
                )
            })?
    }

    #[cfg(not(target_os = "macos"))]
    {
        // The validators above always run so non-macOS callers see contract
        // violations (bad format, relative path) ahead of the platform error.
        let _ = req;
        Err(rpc_error(
            RPC_INTERNAL_ERROR,
            codes::PERMISSION_DENIED,
            "screenshot is only available on macOS in this release",
            Value::Null,
        ))
    }
}

#[cfg(target_os = "macos")]
fn run_macos(req: ScreenshotRequest) -> Result<Value, RpcError> {
    use super::{
        EnumeratedLayerZeroWindows, ScreenshotBackend, ScreenshotError, WindowBounds,
        enumerate_layer_zero_windows, selected_backend,
    };

    let ScreenshotRequest {
        window_id,
        output_path,
    } = req;

    // Single pass over the on-screen window list collects both the
    // `available_windows` payload for WINDOW_NOT_FOUND and the target's
    // bounds — one CGWindowListCopyWindowInfo snapshot instead of two,
    // which also closes the TOCTOU between the two former calls.
    let EnumeratedLayerZeroWindows {
        available: discovered,
        target_bounds,
    } = enumerate_layer_zero_windows(window_id).map_err(|err| match err {
        ScreenshotError::PlatformUnsupported => rpc_error(
            RPC_INTERNAL_ERROR,
            codes::PERMISSION_DENIED,
            "screenshot is only available on macOS in this release",
            Value::Null,
        ),
        ScreenshotError::CaptureFailed { message } => rpc_error(
            RPC_INTERNAL_ERROR,
            codes::CAPTURE_FAILED,
            format!("failed to enumerate windows: {message}"),
            Value::Null,
        ),
    })?;

    if target_bounds.is_none() {
        return Err(rpc_error(
            RPC_INVALID_PARAMS,
            codes::WINDOW_NOT_FOUND,
            format!("window_id {window_id} not found"),
            json!({
                "available_windows": render_available_windows(&discovered),
            }),
        ));
    }

    let tmp_path = tmp_path_for(&output_path);

    let primary = selected_backend();
    let (backend, tcc_denied) = capture_with_fallback(primary, window_id, &tmp_path)?;

    let metadata = match read_png_metadata(&tmp_path) {
        Ok(m) => m,
        Err(err) => {
            cleanup_tmp(&tmp_path);
            return Err(err);
        }
    };

    let logical_bounds = target_bounds.unwrap_or(WindowBounds {
        width: 0.0,
        height: 0.0,
    });
    let scale_factor = compute_scale_factor(
        matches!(backend, ScreenshotBackend::CgWindowList),
        metadata.width,
        metadata.height,
        logical_bounds.width,
        logical_bounds.height,
    );

    if let Err(err) = std::fs::rename(&tmp_path, &output_path) {
        cleanup_tmp(&tmp_path);
        return Err(rpc_error(
            RPC_INTERNAL_ERROR,
            codes::CAPTURE_FAILED,
            format!(
                "failed to rename {} to {}: {err}",
                tmp_path.display(),
                output_path.display()
            ),
            Value::Null,
        ));
    }

    // The other `ScreenshotBackend` variants never reach this point because
    // the dispatcher in `capture_with_fallback` already maps them to error
    // responses. The match is exhaustive so a future backend variant forces
    // an explicit decision here instead of silently flowing into a fallback.
    let backend_label = match backend {
        ScreenshotBackend::ScreencaptureProbe => "screencapture",
        ScreenshotBackend::CgWindowList => "cgwindow",
        ScreenshotBackend::WkWebViewSnapshot | ScreenshotBackend::PlatformUnsupported => {
            unreachable!("capture_with_fallback returns Err for these variants")
        }
    };
    Ok(json!({
        "output_path": output_path.display().to_string(),
        "window_id": window_id,
        "width": metadata.width,
        "height": metadata.height,
        "scale_factor": scale_factor,
        "byte_size": metadata.byte_size,
        "backend": backend_label,
        "tcc_denied": tcc_denied,
    }))
}

#[cfg(target_os = "macos")]
fn render_available_windows(discovered: &[super::DiscoveredWindow]) -> Vec<Value> {
    discovered
        .iter()
        .map(|w| {
            json!({
                "window_id": w.window_id,
                "owner": w.owner,
                "title": w.title,
                "layer": w.layer,
            })
        })
        .collect()
}

#[cfg(target_os = "macos")]
struct PngMetadata {
    width: u32,
    height: u32,
    byte_size: u64,
}

/// Monotonic counter appended to the tmp filename so two concurrent captures
/// from the same process targeting the same final path never collide on disk.
#[cfg(target_os = "macos")]
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Compute the temp-write target alongside the final output path. Combining
/// the pid (cross-process disambiguator) with a process-local atomic counter
/// (intra-process disambiguator) guarantees a fresh tmp filename even when
/// multiple captures race on the same final path.
#[cfg(target_os = "macos")]
fn tmp_path_for(final_path: &Path) -> std::path::PathBuf {
    let mut name = final_path
        .file_name()
        .map(std::ffi::OsString::from)
        .unwrap_or_default();
    let counter = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    name.push(format!(".tmp.{}.{counter}.png", std::process::id()));
    final_path.with_file_name(name)
}

/// Remove the tmp file if it exists. Errors during cleanup do not propagate —
/// the caller is already returning the original capture failure — but they are
/// logged so a lingering tmp file does not vanish silently from operator view.
#[cfg(target_os = "macos")]
fn cleanup_tmp(path: &Path) {
    if let Err(err) = std::fs::remove_file(path)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(
            tmp_path = %path.display(),
            error = %err,
            "failed to remove tauri-pilot screenshot tmp file",
        );
    }
}

#[cfg(target_os = "macos")]
fn capture_with_fallback(
    primary: super::ScreenshotBackend,
    window_id: u32,
    tmp_path: &Path,
) -> Result<(super::ScreenshotBackend, bool), RpcError> {
    use super::{ScreenshotBackend, capture_cgwindow, capture_screencapture};

    match primary {
        ScreenshotBackend::ScreencaptureProbe => {
            match capture_screencapture(window_id, tmp_path) {
                Ok(()) => Ok((ScreenshotBackend::ScreencaptureProbe, false)),
                Err(primary_err) => {
                    // TCC was granted at probe time but the per-window capture
                    // still failed. Try the CGWindow fallback so the caller
                    // does not lose the artifact when permission was revoked
                    // between probe and call.
                    match capture_cgwindow(window_id, tmp_path) {
                        Ok(()) => Ok((ScreenshotBackend::CgWindowList, true)),
                        Err(fallback_err) => {
                            cleanup_tmp(tmp_path);
                            Err(rpc_error(
                                RPC_INTERNAL_ERROR,
                                codes::CAPTURE_FAILED,
                                format!(
                                    "screencapture failed ({primary_err}) and CGWindow fallback also failed ({fallback_err})"
                                ),
                                Value::Null,
                            ))
                        }
                    }
                }
            }
        }
        ScreenshotBackend::CgWindowList => match capture_cgwindow(window_id, tmp_path) {
            Ok(()) => Ok((ScreenshotBackend::CgWindowList, true)),
            Err(err) => {
                cleanup_tmp(tmp_path);
                Err(rpc_error(
                    RPC_INTERNAL_ERROR,
                    codes::CAPTURE_FAILED,
                    format!("CGWindow capture failed: {err}"),
                    Value::Null,
                ))
            }
        },
        ScreenshotBackend::WkWebViewSnapshot | ScreenshotBackend::PlatformUnsupported => {
            Err(rpc_error(
                RPC_INTERNAL_ERROR,
                codes::PERMISSION_DENIED,
                "screenshot is only available on macOS in this release",
                Value::Null,
            ))
        }
    }
}

#[cfg(target_os = "macos")]
fn read_png_metadata(path: &Path) -> Result<PngMetadata, RpcError> {
    let metadata = std::fs::metadata(path).map_err(|err| {
        rpc_error(
            RPC_INTERNAL_ERROR,
            codes::CAPTURE_FAILED,
            format!("failed to stat captured PNG {}: {err}", path.display()),
            Value::Null,
        )
    })?;
    let byte_size = metadata.len();

    // `ImageReader::into_dimensions` parses the PNG header (IHDR chunk) only —
    // a few hundred bytes — instead of decoding the entire raster. For a 4K
    // Retina capture this avoids ~33 MiB of pointless RGBA allocation just to
    // read width/height for the response payload.
    let reader = image::ImageReader::open(path).map_err(|err| {
        rpc_error(
            RPC_INTERNAL_ERROR,
            codes::CAPTURE_FAILED,
            format!("failed to open captured PNG {}: {err}", path.display()),
            Value::Null,
        )
    })?;
    let (width, height) = reader.into_dimensions().map_err(|err| {
        rpc_error(
            RPC_INTERNAL_ERROR,
            codes::CAPTURE_FAILED,
            format!(
                "failed to read PNG dimensions from {}: {err}",
                path.display()
            ),
            Value::Null,
        )
    })?;
    Ok(PngMetadata {
        width,
        height,
        byte_size,
    })
}

/// Divide a pixel count by its logical (point) counterpart.
///
/// Rounds to 2 decimal places to absorb the float noise `CGWindowBounds`
/// carries, so a capture that really is the window lands on the same number
/// from both axes instead of `2.0` against `2.00000003`. Returns `None` when
/// the division is not a finite positive number.
#[cfg(any(target_os = "macos", test))]
fn rounded_ratio(pixels: u32, logical: f64) -> Option<f64> {
    let ratio = f64::from(pixels) / logical;
    if ratio.is_finite() && ratio > 0.0 {
        Some((ratio * 100.0).round() / 100.0)
    } else {
        None
    }
}

/// Derive the capture's scale factor from its pixel dimensions and the
/// window's logical bounds.
///
/// Both axes must agree on the ratio. A PNG that is not exactly the window
/// fails that test, which is what #149 asks for: with screen recording denied
/// the `CGWindowList` fallback hands back a screen-sized image, and a screen's
/// aspect is not the window's, so the two ratios part ways (4344/1512 = 2.87
/// across, 2744/982 = 2.79 down). Pixel-diff harnesses tag artifacts by this
/// value, so an absent scale beats a plausible wrong one.
///
/// The geometry decides, not `tcc_denied` — that flag reports a permission
/// state, and `capture_with_fallback` also raises it after a *granted* probe
/// whose per-window `screencapture` call failed, where the fallback image is
/// the window at 1x and the scale was derivable. The primary backend passes
/// `screencapture -o` so its own PNG is not padded by the window's shadow.
///
/// Two agreeing ratios are not proof on their own, so the capture's provenance
/// gates them: `nominal_resolution` says the PNG came from `capture_cgwindow`,
/// which asks CoreGraphics for `kCGWindowImageNominalResolution` and therefore
/// can only legitimately produce 1:1. Anything else from that backend is not
/// the window — under TCC denial CoreGraphics ignores the flags and hands back
/// the screen, which would otherwise slip through whenever its aspect matches
/// the window's. Reading the display's backing scale from `CGDisplayMode`
/// would remove the inference entirely.
///
/// Falls back to `Some(1.0)` when the logical bounds are unknown or zero so
/// the response never carries NaN or Inf.
#[cfg(any(target_os = "macos", test))]
fn compute_scale_factor(
    nominal_resolution: bool,
    pixel_width: u32,
    pixel_height: u32,
    logical_width: f64,
    logical_height: f64,
) -> Option<f32> {
    if logical_width <= 0.0 || logical_height <= 0.0 {
        return Some(1.0);
    }
    let horizontal = rounded_ratio(pixel_width, logical_width)?;
    let vertical = rounded_ratio(pixel_height, logical_height)?;
    // Both sides are a whole number of hundredths, so half a hundredth apart
    // already means two different numbers.
    if (horizontal - vertical).abs() > 0.005 {
        return None;
    }
    if nominal_resolution && (horizontal - 1.0).abs() > 0.005 {
        return None;
    }
    // Truncation to f32 is intentional: Retina scale factors land in a small
    // finite range (1.0 / 2.0) that f32 represents losslessly.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "Retina scale factors fit in f32 without loss"
    )]
    let scale_f32 = horizontal as f32;
    Some(scale_f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn err_data(err: &RpcError) -> &Value {
        err.data.as_ref().expect("error data present")
    }

    #[tokio::test]
    async fn rejects_missing_window_id() {
        let params = json!({"output_path": "/tmp/x.png"});
        let err = handle_screenshot(Some(&params))
            .await
            .expect_err("must reject missing window_id");
        assert_eq!(err.code, RPC_INVALID_PARAMS);
        assert!(
            err.message.contains("window_id"),
            "message must mention window_id, got: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn rejects_relative_output_path() {
        let params = json!({"window_id": 42_u32, "output_path": "relative/path.png"});
        let err = handle_screenshot(Some(&params))
            .await
            .expect_err("must reject relative output_path");
        assert_eq!(err.code, RPC_INVALID_PARAMS);
        assert_eq!(
            err_data(&err).get("error").and_then(Value::as_str),
            Some(codes::INVALID_OUTPUT_PATH)
        );
    }

    #[tokio::test]
    async fn rejects_nonexistent_parent_directory() {
        let params = json!({
            "window_id": 42_u32,
            "output_path": "/tauri-pilot-test-no-such-dir-9f3a/x.png",
        });
        let err = handle_screenshot(Some(&params))
            .await
            .expect_err("must reject missing parent dir");
        assert_eq!(err.code, RPC_INVALID_PARAMS);
        assert_eq!(
            err_data(&err).get("error").and_then(Value::as_str),
            Some(codes::INVALID_OUTPUT_PATH)
        );
    }

    #[tokio::test]
    async fn rejects_directory_output_path() {
        let params = json!({
            "window_id": 42_u32,
            "output_path": std::env::temp_dir().display().to_string(),
        });
        let err = handle_screenshot(Some(&params))
            .await
            .expect_err("must reject directory output_path");
        assert_eq!(err.code, RPC_INVALID_PARAMS);
        assert_eq!(
            err_data(&err).get("error").and_then(Value::as_str),
            Some(codes::INVALID_OUTPUT_PATH)
        );
    }

    #[tokio::test]
    async fn rejects_unsupported_format() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "tauri-pilot-test-screenshot-{}.png",
            std::process::id()
        ));
        let params = json!({
            "window_id": 42_u32,
            "output_path": path.display().to_string(),
            "format": "jpeg",
        });
        let err = handle_screenshot(Some(&params))
            .await
            .expect_err("must reject jpeg format");
        assert_eq!(err.code, RPC_INVALID_PARAMS);
        assert_eq!(
            err_data(&err).get("error").and_then(Value::as_str),
            Some(codes::UNSUPPORTED_FORMAT)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn tmp_path_keeps_png_extension_for_image_reader() {
        let path = Path::new("/tmp/tauri-pilot-shot.png");
        let tmp = tmp_path_for(path);
        assert_eq!(tmp.extension().and_then(|ext| ext.to_str()), Some("png"));
        assert!(
            tmp.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(".tmp."))
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn window_not_found_returns_available_list() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "tauri-pilot-test-screenshot-not-found-{}.png",
            std::process::id()
        ));
        // 0 is `kCGNullWindowID` per CoreGraphics and never identifies a real
        // window, so this is the most reliably-missing window_id we can pick
        // without touching the host's display state.
        let params = json!({
            "window_id": u32::MAX,
            "output_path": path.display().to_string(),
        });
        let err = handle_screenshot(Some(&params))
            .await
            .expect_err("must reject unknown window_id");
        assert_eq!(err.code, RPC_INVALID_PARAMS);
        assert_eq!(
            err_data(&err).get("error").and_then(Value::as_str),
            Some(codes::WINDOW_NOT_FOUND)
        );
        let list = err_data(&err)
            .get("available_windows")
            .and_then(Value::as_array)
            .expect("available_windows present");
        // On a headless CI runner the list may be empty; the contract only
        // requires the key, not a minimum length. Assert shape on any entry
        // that does come back so the discovery payload stays stable.
        if let Some(first) = list.first() {
            assert!(first.get("window_id").is_some());
            assert!(first.get("owner").is_some());
            assert!(first.get("title").is_some());
            assert!(first.get("layer").is_some());
        }
        // The test binary owns no on-screen window, so every entry here
        // belongs to another process and must come back title-less: this
        // payload now reaches stderr and the MCP client, and other apps'
        // document names, URLs and chat titles do not belong there.
        for window in list {
            assert_eq!(
                window.get("title").and_then(Value::as_str),
                Some(""),
                "a window owned by another process must not carry its title: {window}"
            );
        }
    }

    #[test]
    fn test_compute_scale_factor_screen_sized_capture_reports_no_scale() {
        // #149: with screen recording denied, `capture_with_fallback` drops to
        // `CGWindowList`, which returns a screen-sized image instead of the
        // window. A 16" panel at 2172x1372 points against a 1512x982 window
        // gives 2.87 across and 2.79 down — no display has either.
        assert_eq!(compute_scale_factor(true, 4344, 2744, 1512.0, 982.0), None);
    }

    #[test]
    fn test_compute_scale_factor_shadow_padded_capture_reports_no_scale() {
        // `screencapture` without `-o` pads the PNG with the window's drop
        // shadow, and by more below the window than beside it, so the two
        // ratios disagree even though permission was granted.
        assert_eq!(compute_scale_factor(false, 3248, 2188, 1512.0, 982.0), None);
    }

    #[test]
    fn test_compute_scale_factor_window_sized_capture_keeps_derived_scale() {
        // The derivation holds when the capture really is the window.
        assert_eq!(
            compute_scale_factor(false, 3024, 1964, 1512.0, 982.0),
            Some(2.0)
        );
    }

    #[test]
    fn test_compute_scale_factor_screen_sized_capture_at_window_aspect_reports_no_scale() {
        // The two ratios agree here — a 2172x1372 point screen against a
        // window at exactly half its size in both directions — so geometry
        // alone would hand back 4.0. `capture_cgwindow` asked for nominal
        // resolution, so anything but 1:1 means CoreGraphics did not honour
        // the flags and the PNG is not the window.
        assert_eq!(compute_scale_factor(true, 4344, 2744, 1086.0, 686.0), None);
    }

    #[test]
    fn test_compute_scale_factor_nominal_window_capture_reports_one() {
        // `capture_with_fallback` also reaches `capture_cgwindow` after a
        // granted probe whose per-window `screencapture` call failed. The PNG
        // is then the window at 1x, and 1.0 describes it.
        assert_eq!(
            compute_scale_factor(true, 1512, 982, 1512.0, 982.0),
            Some(1.0)
        );
    }

    #[test]
    fn test_compute_scale_factor_unknown_bounds_falls_back_to_one() {
        assert_eq!(compute_scale_factor(false, 3024, 1964, 0.0, 0.0), Some(1.0));
    }
}
