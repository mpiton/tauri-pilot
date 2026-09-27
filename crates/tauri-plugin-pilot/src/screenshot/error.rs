use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum ScreenshotError {
    // Never constructed today; the non-macOS IPC branch returns
    // `UNSUPPORTED_PLATFORM` directly without going through this enum. Kept so
    // the macOS dispatcher's exhaustive match stays honest if a future stub
    // ever surfaces an unsupported-platform error from the macOS code path.
    #[allow(
        dead_code,
        reason = "kept for the exhaustive match in the macOS dispatcher"
    )]
    #[error("native screenshot capture is unsupported on this platform")]
    PlatformUnsupported,
    #[error("native screenshot capture failed: {message}")]
    CaptureFailed { message: String },
}
