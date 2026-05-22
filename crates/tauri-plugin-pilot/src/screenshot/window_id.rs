use super::ScreenshotError;

/// Minimal description of a layer-0 on-screen window, used to populate the
/// `available_windows` payload returned with `WINDOW_NOT_FOUND` errors so
/// callers can pick the right `window_id` without a second round-trip.
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub(crate) struct DiscoveredWindow {
    pub(crate) window_id: u32,
    pub(crate) owner: String,
    pub(crate) title: String,
    pub(crate) layer: i32,
}

/// Logical (in points, not pixels) bounds of a window, used to derive
/// `scale_factor` by comparing against the captured PNG's pixel dimensions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct WindowBounds {
    pub(crate) width: f64,
    pub(crate) height: f64,
}

/// Result of a single-pass walk over the on-screen window list. Bundles the
/// truncated `available_windows` payload that powers `WINDOW_NOT_FOUND` errors
/// with the bounds of the requested target (when it is a layer-0 window).
pub(crate) struct EnumeratedLayerZeroWindows {
    pub(crate) available: Vec<DiscoveredWindow>,
    pub(crate) target_bounds: Option<WindowBounds>,
}

/// Hard cap on the `available_windows` list returned in `WINDOW_NOT_FOUND`
/// errors. A host with many open windows can otherwise produce a multi-KB
/// payload; 20 is enough to spot a typo or wrong owner without bloating the
/// JSON-RPC frame.
pub(crate) const AVAILABLE_WINDOWS_CAP: usize = 20;

/// Walk the on-screen window list once and collect both the layer-0 windows
/// available for capture (capped at [`AVAILABLE_WINDOWS_CAP`]) and the bounds
/// of `target` when it is itself a layer-0 window.
///
/// Doing this in one pass — instead of separate `list_layer_zero_windows` +
/// `get_window_bounds` calls — avoids a second `CGWindowListCopyWindowInfo`
/// snapshot and removes a TOCTOU window where the target could vanish between
/// the discovery list and the bounds lookup.
///
/// # Errors
///
/// Returns [`ScreenshotError::CaptureFailed`] when `CoreGraphics` returns null.
pub(crate) fn enumerate_layer_zero_windows(
    target: u32,
) -> Result<EnumeratedLayerZeroWindows, ScreenshotError> {
    use core_foundation::array::CFArray;
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::geometry::CGRect;
    use core_graphics::window::{
        CGWindowListCopyWindowInfo, kCGNullWindowID, kCGWindowBounds, kCGWindowLayer,
        kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly, kCGWindowName,
        kCGWindowNumber, kCGWindowOwnerName,
    };

    // SAFETY: `CGWindowListCopyWindowInfo` is a thread-safe CoreGraphics entry
    // point that requires only a connection to the WindowServer — which the
    // host app already holds for any UI to render. Passing `kCGNullWindowID`
    // with `OnScreenOnly` is the documented "give me every on-screen window"
    // form. The returned CFArrayRef follows create-rule ownership: the caller
    // is responsible for `CFRelease`, which `wrap_under_create_rule` arranges
    // via `Drop`.
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
    // SAFETY: `raw` is a non-null CFArrayRef just produced under create-rule
    // ownership (see comment above). `wrap_under_create_rule` takes that
    // ownership without retaining again, so the `Drop` impl on the wrapper
    // performs the matching `CFRelease`.
    let windows: CFArray<CFDictionary<CFString, CFType>> =
        unsafe { TCFType::wrap_under_create_rule(raw) };

    // SAFETY: every `kCGWindow*` symbol is a `CFStringRef` extern static
    // exported by CoreGraphics, owned by the framework. `wrap_under_get_rule`
    // retains the existing reference so our handle is balanced when it drops.
    let owner_key = unsafe { CFString::wrap_under_get_rule(kCGWindowOwnerName) };
    let title_key = unsafe { CFString::wrap_under_get_rule(kCGWindowName) };
    let layer_key = unsafe { CFString::wrap_under_get_rule(kCGWindowLayer) };
    let number_key = unsafe { CFString::wrap_under_get_rule(kCGWindowNumber) };
    let bounds_key = unsafe { CFString::wrap_under_get_rule(kCGWindowBounds) };

    let mut available: Vec<DiscoveredWindow> = Vec::new();
    let mut target_bounds: Option<WindowBounds> = None;

    for dict in windows.iter() {
        let layer = dict
            .find(&layer_key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|v| v.to_i32())
            .unwrap_or(-1);
        if layer != 0 {
            continue;
        }
        let number_i64 = dict
            .find(&number_key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|v| v.to_i64())
            .unwrap_or(0);
        let Ok(window_id) = u32::try_from(number_i64) else {
            continue;
        };
        if window_id == 0 {
            continue;
        }
        let owner = dict
            .find(&owner_key)
            .and_then(|v| v.downcast::<CFString>())
            .map(|v| v.to_string())
            .unwrap_or_default();
        let title = dict
            .find(&title_key)
            .and_then(|v| v.downcast::<CFString>())
            .map(|v| v.to_string())
            .unwrap_or_default();

        if window_id == target && target_bounds.is_none() {
            // CGWindowList stores the bounds rect as a `{Width,Height,X,Y}`
            // dict rather than a CGRect struct; CoreGraphics ships
            // `CGRectMakeWithDictionaryRepresentation` for exactly this case
            // so we do not have to walk the keys by hand.
            if let Some(bounds_dict) = dict
                .find(&bounds_key)
                .and_then(|v| v.downcast::<CFDictionary>())
                && let Some(rect) = CGRect::from_dict_representation(&bounds_dict)
            {
                target_bounds = Some(WindowBounds {
                    width: rect.size.width,
                    height: rect.size.height,
                });
            }
        }

        if available.len() < AVAILABLE_WINDOWS_CAP {
            available.push(DiscoveredWindow {
                window_id,
                owner,
                title,
                layer,
            });
        } else if target_bounds.is_some() {
            // List is full and we already have what the success path needs;
            // continuing would just walk the rest of the array for nothing.
            break;
        }
    }

    Ok(EnumeratedLayerZeroWindows {
        available,
        target_bounds,
    })
}
