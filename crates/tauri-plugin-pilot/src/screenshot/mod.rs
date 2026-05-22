#[cfg(target_os = "macos")]
pub(crate) mod cgwindow;
#[cfg(target_os = "macos")]
pub(crate) mod error;
pub(crate) mod ipc;
#[cfg(target_os = "macos")]
pub(crate) mod native;
#[cfg(target_os = "macos")]
pub(crate) mod probe;
#[cfg(target_os = "macos")]
pub(crate) mod screencapture;
#[cfg(target_os = "macos")]
pub(crate) mod window_id;

#[cfg(target_os = "macos")]
pub(crate) use cgwindow::capture_cgwindow;
#[cfg(target_os = "macos")]
pub(crate) use error::ScreenshotError;
pub(crate) use ipc::handle_screenshot;
#[cfg(target_os = "macos")]
pub(crate) use probe::{ScreenshotBackend, selected_backend};
#[cfg(target_os = "macos")]
pub(crate) use screencapture::capture_screencapture;
#[cfg(target_os = "macos")]
pub(crate) use window_id::{
    DiscoveredWindow, EnumeratedLayerZeroWindows, WindowBounds, enumerate_layer_zero_windows,
};
