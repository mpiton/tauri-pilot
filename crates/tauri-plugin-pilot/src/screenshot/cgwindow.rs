use std::path::Path;

use super::ScreenshotError;

/// Capture a window's pixels via `CGWindowListCreateImage` and write a PNG to `path`.
///
/// # Errors
///
/// Returns [`ScreenshotError::PlatformUnsupported`] off macOS, and
/// [`ScreenshotError::CaptureFailed`] when CoreGraphics returns no image,
/// the raw buffer is too small for the reported dimensions, the dimensions do
/// not fit a `u32`, or the PNG encoder fails to write the file.
#[cfg(target_os = "macos")]
pub(crate) fn capture_cgwindow(window_id: u32, path: &Path) -> Result<(), ScreenshotError> {
    use core_graphics::geometry::{CGPoint, CGRect, CGSize};
    use core_graphics::window::{
        create_image, kCGWindowImageBoundsIgnoreFraming, kCGWindowImageNominalResolution,
        kCGWindowListOptionIncludingWindow,
    };

    let rect = CGRect::new(
        &CGPoint::new(0.0, 0.0),
        &CGSize::new(f64::INFINITY, f64::INFINITY),
    );
    let cg_image = create_image(
        rect,
        kCGWindowListOptionIncludingWindow,
        window_id,
        kCGWindowImageBoundsIgnoreFraming | kCGWindowImageNominalResolution,
    )
    .ok_or_else(|| ScreenshotError::CaptureFailed {
        message: "CGWindowListCreateImage returned null".to_owned(),
    })?;

    let width = cg_image.width();
    let height = cg_image.height();
    let bytes_per_row = cg_image.bytes_per_row();
    let data = cg_image.data();
    let src = data.bytes();

    let row_bytes = width
        .checked_mul(4)
        .ok_or_else(|| ScreenshotError::CaptureFailed {
            message: "row size overflow".to_owned(),
        })?;
    let total = row_bytes
        .checked_mul(height)
        .ok_or_else(|| ScreenshotError::CaptureFailed {
            message: "buffer size overflow".to_owned(),
        })?;
    let mut rgba: Vec<u8> = Vec::with_capacity(total);
    for y in 0..height {
        let row_start = y * bytes_per_row;
        let row_end =
            row_start
                .checked_add(row_bytes)
                .ok_or_else(|| ScreenshotError::CaptureFailed {
                    message: "CGImage row offset overflow".to_owned(),
                })?;
        if row_end > src.len() {
            return Err(ScreenshotError::CaptureFailed {
                message: "CGImage row out of bounds".to_owned(),
            });
        }
        let row = &src[row_start..row_end];
        for px in row.chunks_exact(4) {
            // BGRA -> RGBA
            rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }

    let w = u32::try_from(width).map_err(|_| ScreenshotError::CaptureFailed {
        message: "width does not fit in u32".to_owned(),
    })?;
    let h = u32::try_from(height).map_err(|_| ScreenshotError::CaptureFailed {
        message: "height does not fit in u32".to_owned(),
    })?;

    image::save_buffer(path, &rgba, w, h, image::ColorType::Rgba8).map_err(|err| {
        ScreenshotError::CaptureFailed {
            message: format!("write cgwindow png {}: {err}", path.display()),
        }
    })
}

/// Non-macOS stub for the CGWindow capture path.
///
/// # Errors
///
/// Always returns [`ScreenshotError::PlatformUnsupported`].
#[cfg(not(target_os = "macos"))]
pub(crate) fn capture_cgwindow(_window_id: u32, _path: &Path) -> Result<(), ScreenshotError> {
    Err(ScreenshotError::PlatformUnsupported)
}
