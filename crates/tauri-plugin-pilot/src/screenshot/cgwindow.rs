use std::path::Path;

use super::ScreenshotError;

/// Capture a window's pixels via `CGWindowListCreateImage` and write a PNG to `path`.
///
/// # Output color
///
/// CoreGraphics returns the captured buffer as **premultiplied BGRA**. This
/// function demultiplies into **straight RGBA** before encoding, so the PNG
/// on disk holds an unassociated alpha channel — what downstream pixel-diff
/// and image-processing pipelines expect by default. Pixels with `alpha == 0`
/// are written as transparent black.
///
/// # Errors
///
/// Returns [`ScreenshotError::CaptureFailed`] when `CoreGraphics` returns no
/// image, the raw buffer is too small for the reported dimensions, the
/// dimensions do not fit a `u32`, or the PNG encoder fails to write the file.
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

    // Fail fast on dimensions that the PNG encoder can't accept before any
    // large allocation is attempted.
    let width_u32 = u32::try_from(width).map_err(|_| ScreenshotError::CaptureFailed {
        message: "width does not fit in u32".to_owned(),
    })?;
    let height_u32 = u32::try_from(height).map_err(|_| ScreenshotError::CaptureFailed {
        message: "height does not fit in u32".to_owned(),
    })?;

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
        // `y * bytes_per_row` cannot overflow `usize`: `y < height` and the
        // resulting offset is bounded above by `src.len()`, which is a `usize`
        // by construction (CFData length). The two `checked_mul`s above guard
        // the only unconstrained arithmetic (`Vec::with_capacity(total)`).
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
            // CoreGraphics returns premultiplied BGRA. Demultiply into
            // straight RGBA so downstream pixel-diff and image-processing
            // pipelines see a normal alpha channel; `alpha == 0` collapses to
            // transparent black per the standard convention.
            let blue = px[0];
            let green = px[1];
            let red = px[2];
            let alpha = px[3];
            let (red_out, green_out, blue_out) = if alpha == 0 {
                (0u8, 0u8, 0u8)
            } else if alpha == 255 {
                (red, green, blue)
            } else {
                let alpha_u16 = u16::from(alpha);
                let demul = |channel: u8| {
                    let value = (u16::from(channel) * 255 + alpha_u16 / 2) / alpha_u16;
                    value.min(255) as u8
                };
                (demul(red), demul(green), demul(blue))
            };
            rgba.extend_from_slice(&[red_out, green_out, blue_out, alpha]);
        }
    }

    image::save_buffer(path, &rgba, width_u32, height_u32, image::ColorType::Rgba8).map_err(|err| {
        ScreenshotError::CaptureFailed {
            message: format!("write cgwindow png {}: {err}", path.display()),
        }
    })
}
