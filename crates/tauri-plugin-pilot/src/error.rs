/// Plugin-level errors for tauri-pilot.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An I/O error occurred (socket, file, etc.).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// A JSON serialization/deserialization error occurred.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}
