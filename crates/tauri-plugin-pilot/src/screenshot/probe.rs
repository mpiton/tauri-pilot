use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScreenshotBackend {
    /// Scaffolded path that returns to a working impl once the `WKWebView`
    /// `takeSnapshot:` wiring lands. The auto-probe never selects it today;
    /// it stays in the enum so callers can opt in explicitly when the
    /// follow-up milestone wires it.
    #[allow(dead_code, reason = "constructed in the WkWebView milestone")]
    WkWebViewSnapshot,
    CgWindowList,
    ScreencaptureProbe,
    /// Sentinel matched by the macOS dispatcher to surface a
    /// `PERMISSION_DENIED` IPC error. Never constructed today; the non-macOS
    /// IPC branch short-circuits before reaching the probe.
    #[allow(
        dead_code,
        reason = "kept for the exhaustive match in the macOS dispatcher"
    )]
    PlatformUnsupported,
}

static BACKEND: OnceLock<ScreenshotBackend> = OnceLock::new();

/// Return the cached screenshot backend selected for the current host.
#[must_use]
pub(crate) fn selected_backend() -> ScreenshotBackend {
    *BACKEND.get_or_init(probe_backend)
}

fn probe_backend() -> ScreenshotBackend {
    if screencapture_probe_allows_window_capture() {
        ScreenshotBackend::ScreencaptureProbe
    } else {
        ScreenshotBackend::CgWindowList
    }
}

/// Ask CoreGraphics whether the current process has screen-recording
/// permission via the documented `CGPreflightScreenCaptureAccess` entry point.
///
/// Replaces the previous `screencapture -l 0 /dev/null` probe — that shell-out
/// returned different exit codes across macOS releases and required forking a
/// child process every cold boot, while this call is in-process and is the
/// API Apple's own screen-recording prompt sits on top of.
fn screencapture_probe_allows_window_capture() -> bool {
    core_graphics::access::ScreenCaptureAccess.preflight()
}
