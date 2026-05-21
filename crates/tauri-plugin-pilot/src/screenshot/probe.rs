use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScreenshotBackend {
    /// Scaffolded path that returns to a working impl once the `WKWebView`
    /// `takeSnapshot:` wiring lands. The auto-probe never selects it today;
    /// it stays in the enum so callers can opt in explicitly when the
    /// follow-up milestone wires it.
    #[allow(dead_code)]
    WkWebViewSnapshot,
    CgWindowList,
    ScreencaptureProbe,
    PlatformUnsupported,
}

static BACKEND: OnceLock<ScreenshotBackend> = OnceLock::new();

/// Return the cached screenshot backend selected for the current host.
#[must_use]
pub(crate) fn selected_backend() -> ScreenshotBackend {
    *BACKEND.get_or_init(probe_backend)
}

fn probe_backend() -> ScreenshotBackend {
    #[cfg(target_os = "macos")]
    {
        if screencapture_probe_allows_window_capture() {
            ScreenshotBackend::ScreencaptureProbe
        } else {
            ScreenshotBackend::CgWindowList
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        ScreenshotBackend::PlatformUnsupported
    }
}

#[cfg(target_os = "macos")]
fn screencapture_probe_allows_window_capture() -> bool {
    std::process::Command::new("screencapture")
        .args(["-l", "0", "/dev/null"])
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_backend_is_closed_unsupported_variant() {
        assert_eq!(
            super::selected_backend(),
            super::ScreenshotBackend::PlatformUnsupported
        );
    }
}
