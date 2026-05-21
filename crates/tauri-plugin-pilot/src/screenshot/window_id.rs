use super::ScreenshotError;

/// Find a `CGWindowID` by owner name (exact match) and an optional title substring.
///
/// Walks the on-screen window list, skipping desktop elements, and returns the
/// first layer-0 window whose owner equals `owner` and whose title contains
/// `title` (when provided).
///
/// # Errors
///
/// Returns [`ScreenshotError::PlatformUnsupported`] off macOS, and
/// [`ScreenshotError::CaptureFailed`] when the CoreGraphics window list call
/// returns null or no layer-0 window matches the filter.
#[cfg(target_os = "macos")]
pub(crate) fn get_window_id(owner: &str, title: Option<&str>) -> Result<u32, ScreenshotError> {
    use core_foundation::array::CFArray;
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::window::{
        CGWindowListCopyWindowInfo, kCGNullWindowID, kCGWindowLayer,
        kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly, kCGWindowName,
        kCGWindowNumber, kCGWindowOwnerName,
    };

    let raw = unsafe {
        CGWindowListCopyWindowInfo(
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
            kCGNullWindowID,
        )
    };
    if raw.is_null() {
        return Err(ScreenshotError::CaptureFailed {
            message: "CGWindowListCopyWindowInfo returned null".to_owned(),
        });
    }
    let windows: CFArray<CFDictionary<CFString, CFType>> =
        unsafe { TCFType::wrap_under_create_rule(raw) };

    let owner_key = unsafe { CFString::wrap_under_get_rule(kCGWindowOwnerName) };
    let title_key = unsafe { CFString::wrap_under_get_rule(kCGWindowName) };
    let layer_key = unsafe { CFString::wrap_under_get_rule(kCGWindowLayer) };
    let number_key = unsafe { CFString::wrap_under_get_rule(kCGWindowNumber) };

    for idx in 0..windows.len() {
        let Some(dict) = windows.get(idx) else {
            continue;
        };
        let owner_value = dict
            .find(&owner_key)
            .and_then(|v| v.downcast::<CFString>())
            .map(|v| v.to_string())
            .unwrap_or_default();
        let title_value = dict
            .find(&title_key)
            .and_then(|v| v.downcast::<CFString>())
            .map(|v| v.to_string())
            .unwrap_or_default();
        let layer = dict
            .find(&layer_key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|v| v.to_i32())
            .unwrap_or(-1);
        let number_i64 = dict
            .find(&number_key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|v| v.to_i64())
            .unwrap_or(0);

        if owner_value != owner {
            continue;
        }
        if let Some(needle) = title
            && !title_value.contains(needle)
        {
            continue;
        }
        if layer != 0 {
            continue;
        }
        let Ok(number) = u32::try_from(number_i64) else {
            continue;
        };
        if number == 0 {
            continue;
        }
        return Ok(number);
    }
    Err(ScreenshotError::CaptureFailed {
        message: format!("no layer-0 window matched owner={owner}"),
    })
}

/// Non-macOS stub for the window-id resolver.
///
/// # Errors
///
/// Always returns [`ScreenshotError::PlatformUnsupported`].
#[cfg(not(target_os = "macos"))]
pub(crate) fn get_window_id(_owner: &str, _title: Option<&str>) -> Result<u32, ScreenshotError> {
    Err(ScreenshotError::PlatformUnsupported)
}
