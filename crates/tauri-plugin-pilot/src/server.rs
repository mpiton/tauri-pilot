use crate::error::Error;
use crate::eval::EvalEngine;
use crate::handler;
use crate::protocol::{Request, Response};

use std::os::unix::io::AsRawFd;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

/// A function that evaluates JS in the webview.
/// The first argument is an optional window label (`None` means "use default window").
pub(crate) type EvalFn = Arc<dyn Fn(Option<&str>, String) -> Result<(), String> + Send + Sync>;

/// A function that lists all available webview windows and returns their metadata.
pub(crate) type ListWindowsFn = Arc<dyn Fn() -> serde_json::Value + Send + Sync>;

/// RAII guard that removes the socket file on drop (normal shutdown or panic).
/// Stores the inode at bind time so it only unlinks its own socket, not one
/// created by an overlapping instance.
pub(crate) struct SocketGuard {
    path: std::path::PathBuf,
    inode: u64,
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        use std::os::unix::fs::MetadataExt;
        // Only unlink if the on-disk inode still matches ours
        if let Ok(meta) = std::fs::metadata(&self.path)
            && meta.ino() == self.inode
        {
            let _ = std::fs::remove_file(&self.path);
            tracing::info!(path = %self.path.display(), "socket removed");
        }
    }
}

/// Get inode from a raw file descriptor via `fstat`.
/// This is race-free: it queries the kernel FD, not the filesystem path.
fn inode_from_raw_fd(fd: std::os::unix::io::RawFd) -> u64 {
    // SAFETY: fstat only reads from a valid fd and writes to our stack buffer.
    unsafe {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if libc::fstat(fd, stat.as_mut_ptr()) == 0 {
            stat.assume_init().st_ino
        } else {
            0
        }
    }
}

/// Bind the socket using the **std** (sync) listener so this can be called
/// outside a tokio runtime (e.g. from Tauri plugin `setup`).
///
/// Tries bind first; only removes stale files on `AddrInUse` after verifying
/// no live server is listening.
/// Returns a std listener and a [`SocketGuard`] that cleans up on drop.
pub(crate) fn bind(
    socket_path: &std::path::Path,
) -> Result<(std::os::unix::net::UnixListener, SocketGuard), Error> {
    let listener = match std::os::unix::net::UnixListener::bind(socket_path) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            // Probe: if a live server answers, the socket is truly in use.
            // Only treat ConnectionRefused as "stale" — other errors (e.g.
            // PermissionDenied) should propagate rather than blindly unlinking.
            match std::os::unix::net::UnixStream::connect(socket_path) {
                Ok(_) => {
                    return Err(Error::Io(std::io::Error::new(
                        std::io::ErrorKind::AddrInUse,
                        format!("socket already in use: {}", socket_path.display()),
                    )));
                }
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                    // Stale socket from a crashed process — safe to remove and retry.
                    let _ = std::fs::remove_file(socket_path);
                    std::os::unix::net::UnixListener::bind(socket_path)?
                }
                Err(e) => {
                    return Err(Error::Io(e));
                }
            }
        }
        Err(e) => return Err(Error::Io(e)),
    };

    // Must be non-blocking for tokio conversion
    listener.set_nonblocking(true)?;

    tracing::info!(path = %socket_path.display(), "tauri-pilot socket listening");
    let inode = inode_from_raw_fd(listener.as_raw_fd());

    Ok((
        listener,
        SocketGuard {
            path: socket_path.to_path_buf(),
            inode,
        },
    ))
}

/// Run the accept loop on a pre-bound std listener. Converts to tokio internally.
/// The `_guard` is held for its `Drop` cleanup.
pub(crate) async fn run(
    std_listener: std::os::unix::net::UnixListener,
    _guard: SocketGuard,
    engine: EvalEngine,
    eval_fn: Option<EvalFn>,
    list_fn: Option<ListWindowsFn>,
) {
    let listener = match UnixListener::from_std(std_listener) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("failed to convert listener to tokio: {e}");
            return;
        }
    };
    if let Err(e) = accept_loop(listener, engine, eval_fn, list_fn).await {
        tracing::error!("socket server error: {e}");
    }
}

async fn accept_loop(
    listener: UnixListener,
    engine: EvalEngine,
    eval_fn: Option<EvalFn>,
    list_fn: Option<ListWindowsFn>,
) -> Result<(), Error> {
    let ctx = Arc::new((engine, eval_fn, list_fn));

    loop {
        let (stream, _addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::warn!("accept error: {e}");
                continue;
            }
        };
        let ctx = Arc::clone(&ctx);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, &ctx.0, ctx.1.as_ref(), ctx.2.as_ref()).await
            {
                tracing::warn!("connection error: {e}");
            }
        });
    }
}

async fn handle_connection(
    stream: UnixStream,
    engine: &EvalEngine,
    eval_fn: Option<&EvalFn>,
    list_fn: Option<&ListWindowsFn>,
) -> Result<(), Error> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Request>(trimmed) {
            Ok(req) if req.jsonrpc != "2.0" => Response::error(
                req.id,
                -32600,
                "Invalid JSON-RPC version (expected \"2.0\")",
            ),
            Ok(req) => dispatch(&req, engine, eval_fn, list_fn).await,
            Err(e) => Response::error(0, -32700, format!("Parse error: {e}")),
        };

        let mut resp_bytes = serde_json::to_vec(&response)?;
        resp_bytes.push(b'\n');
        writer.write_all(&resp_bytes).await?;
        writer.flush().await?;
    }

    Ok(())
}

async fn dispatch(
    req: &Request,
    engine: &EvalEngine,
    eval_fn: Option<&EvalFn>,
    list_fn: Option<&ListWindowsFn>,
) -> Response {
    match handler::dispatch(&req.method, req.params.as_ref(), engine, eval_fn, list_fn).await {
        Ok(result) => Response::success(req.id, result),
        Err(rpc_err) => Response {
            jsonrpc: "2.0".to_owned(),
            id: req.id,
            result: None,
            error: Some(rpc_err),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn unique_socket_path() -> PathBuf {
        let n = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        PathBuf::from(format!(
            "/tmp/tauri-pilot-test-{}-{n}.sock",
            std::process::id()
        ))
    }

    async fn start_test_server(path: &PathBuf) -> tokio::task::JoinHandle<()> {
        let (listener, guard) = bind(path).expect("bind test socket");
        let engine = EvalEngine::new();
        let handle = tokio::spawn(async move { run(listener, guard, engine, None, None).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        handle
    }

    #[tokio::test]
    async fn test_server_responds_ping_ok() {
        let socket = unique_socket_path();
        let handle = start_test_server(&socket).await;

        let stream = UnixStream::connect(&socket).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        writer
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n")
            .await
            .unwrap();
        writer.flush().await.unwrap();

        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let resp: Response = serde_json::from_str(&line).unwrap();

        assert_eq!(resp.id, 1);
        assert!(resp.error.is_none());
        assert_eq!(resp.result, Some(serde_json::json!({"status": "ok"})));

        handle.abort();
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn test_server_handles_invalid_json() {
        let socket = unique_socket_path();
        let handle = start_test_server(&socket).await;

        let stream = UnixStream::connect(&socket).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        writer.write_all(b"not json\n").await.unwrap();
        writer.flush().await.unwrap();

        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let resp: Response = serde_json::from_str(&line).unwrap();

        assert_eq!(resp.id, 0);
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32700);

        handle.abort();
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn test_server_handles_multiple_requests() {
        let socket = unique_socket_path();
        let handle = start_test_server(&socket).await;

        let stream = UnixStream::connect(&socket).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        for i in 1..=3 {
            let req = format!("{{\"jsonrpc\":\"2.0\",\"id\":{i},\"method\":\"test\"}}\n");
            writer.write_all(req.as_bytes()).await.unwrap();
            writer.flush().await.unwrap();

            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let resp: Response = serde_json::from_str(&line).unwrap();
            assert_eq!(resp.id, i);
        }

        handle.abort();
        let _ = std::fs::remove_file(&socket);
    }
}
