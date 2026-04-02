mod error;

pub use error::Error;

/// Initialize the tauri-pilot plugin.
///
/// # Panics
///
/// This function does not panic.
#[must_use]
pub fn init<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("pilot").build()
}
