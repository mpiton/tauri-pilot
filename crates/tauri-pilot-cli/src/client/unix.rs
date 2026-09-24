use anyhow::{Context, Result};
use std::path::Path;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

/// Opens the Unix socket and splits it into its read and write halves.
pub async fn connect(path: &Path) -> Result<(OwnedReadHalf, OwnedWriteHalf)> {
    let stream = UnixStream::connect(path)
        .await
        .with_context(|| format!("Cannot connect to socket: {}", path.display()))?;
    Ok(stream.into_split())
}
