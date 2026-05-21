pub(crate) mod cgwindow;
pub(crate) mod error;
pub(crate) mod native;
pub(crate) mod probe;
pub(crate) mod screencapture;
pub(crate) mod window_id;

pub(crate) use cgwindow::capture_cgwindow;
pub(crate) use error::ScreenshotError;
pub(crate) use native::capture_wkwebview_png;
pub(crate) use probe::{ScreenshotBackend, selected_backend};
pub(crate) use screencapture::capture_screencapture;
pub(crate) use window_id::get_window_id;
