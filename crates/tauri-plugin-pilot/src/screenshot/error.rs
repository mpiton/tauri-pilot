use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum ScreenshotError {
    #[error("native screenshot capture is unsupported on this platform")]
    PlatformUnsupported,
    #[error("native screenshot capture failed: {message}")]
    CaptureFailed { message: String },
}
