use std::path::Path;

use super::ScreenshotError;

/// Capture a window's pixels by shelling out to the `screencapture` system tool.
///
/// Invokes `screencapture -x -l <window_id> <path>` so the call is silent and
/// targets one window rather than the whole screen. Requires Screen Recording
/// permission in System Settings -> Privacy & Security; the first invocation
/// under a sandboxed host surfaces the macOS permission dialog.
///
/// # Errors
///
/// Returns [`ScreenshotError::PlatformUnsupported`] off macOS, and
/// [`ScreenshotError::CaptureFailed`] when the child process cannot be
/// spawned or exits with a non-zero status (TCC-denied, unknown window id,
/// or unwritable output path).
#[cfg(target_os = "macos")]
pub(crate) fn capture_screencapture(window_id: u32, path: &Path) -> Result<(), ScreenshotError> {
    use std::process::Command;

    let status = Command::new("screencapture")
        .arg("-x")
        .arg("-l")
        .arg(window_id.to_string())
        .arg(path)
        .status()
        .map_err(|err| ScreenshotError::CaptureFailed {
            message: format!("spawn screencapture: {err}"),
        })?;
    if !status.success() {
        return Err(ScreenshotError::CaptureFailed {
            message: format!("screencapture exited with {status}"),
        });
    }
    Ok(())
}

/// Non-macOS stub for the `screencapture` shell-out path.
///
/// # Errors
///
/// Always returns [`ScreenshotError::PlatformUnsupported`].
#[cfg(not(target_os = "macos"))]
pub(crate) fn capture_screencapture(_window_id: u32, _path: &Path) -> Result<(), ScreenshotError> {
    Err(ScreenshotError::PlatformUnsupported)
}
