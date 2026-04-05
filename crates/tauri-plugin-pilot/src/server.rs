use crate::error::Error;
use crate::eval::EvalEngine;
use crate::handler;
use crate::protocol::{Request, Response};
use crate::recorder::Recorder;

use std::os::unix::fs::PermissionsExt;
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

/// Returns true if `path` is a directory owned by the current user with no group/world permissions.
fn is_private_dir(path: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match std::fs::metadata(path) {
        Ok(m) => {
            // SAFETY: getuid() has no preconditions.
            let my_uid = unsafe { libc::getuid() };
            m.is_dir() && m.uid() == my_uid && m.mode().trailing_zeros() >= 6
        }
        Err(_) => false,
    }
}

/// Core implementation — accepts the XDG value directly so tests can call it without mutating
/// the process environment.
fn socket_dir_from(xdg: Option<std::ffi::OsString>) -> std::path::PathBuf {
    if let Some(val) = xdg.filter(|v| !v.is_empty()) {
        let path = std::path::PathBuf::from(&val);
        if is_private_dir(&path) {
            return path;
        }
        tracing::warn!(
            path = %path.display(),
            "XDG_RUNTIME_DIR is not a private directory, falling back to /tmp"
        );
    }
    std::path::PathBuf::from("/tmp")
}

/// Returns the directory for the socket file.
/// Prefers `$XDG_RUNTIME_DIR` when it is a private directory (owned by current user, no
/// group/world access). Falls back to `/tmp` with a warning if the directory is not private.
fn socket_dir() -> std::path::PathBuf {
    socket_dir_from(std::env::var_os("XDG_RUNTIME_DIR"))
}

/// Build the full socket path for the given app identifier.
pub(crate) fn socket_path(identifier: &str) -> std::path::PathBuf {
    socket_dir().join(format!("tauri-pilot-{identifier}.sock"))
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
    // SAFETY: umask is always safe to call; we restore the old mask immediately.
    let old_mask = unsafe { libc::umask(0o177) };
    let first_bind = std::os::unix::net::UnixListener::bind(socket_path);
    // SAFETY: restoring the umask we just saved.
    unsafe { libc::umask(old_mask) };

    let listener = match first_bind {
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
                    // SAFETY: umask is always safe to call; we restore the old mask immediately.
                    let old_mask = unsafe { libc::umask(0o177) };
                    let retry_bind = std::os::unix::net::UnixListener::bind(socket_path);
                    // SAFETY: restoring the umask we just saved.
                    unsafe { libc::umask(old_mask) };
                    retry_bind?
                }
                Err(e) => {
                    return Err(Error::Io(e));
                }
            }
        }
        Err(e) => return Err(Error::Io(e)),
    };

    // Restrict socket to owner-only access (defense-in-depth alongside XDG_RUNTIME_DIR).
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))?;

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
    recorder: Recorder,
) {
    let listener = match UnixListener::from_std(std_listener) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("failed to convert listener to tokio: {e}");
            return;
        }
    };
    if let Err(e) = accept_loop(listener, engine, eval_fn, list_fn, recorder).await {
        tracing::error!("socket server error: {e}");
    }
}

async fn accept_loop(
    listener: UnixListener,
    engine: EvalEngine,
    eval_fn: Option<EvalFn>,
    list_fn: Option<ListWindowsFn>,
    recorder: Recorder,
) -> Result<(), Error> {
    let ctx = Arc::new((engine, eval_fn, list_fn, recorder));

    loop {
        let (stream, _addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::warn!("accept error: {e}");
                continue;
            }
        };
        // Verify the connecting process belongs to the same user.
        match stream.peer_cred() {
            Ok(cred) => {
                // SAFETY: getuid() is always safe to call; it has no preconditions.
                let my_uid = unsafe { libc::getuid() };
                if cred.uid() != my_uid {
                    tracing::warn!(
                        peer_uid = cred.uid(),
                        expected_uid = my_uid,
                        "rejected connection from different user"
                    );
                    continue;
                }
            }
            Err(e) => {
                tracing::warn!("failed to get peer credentials: {e}");
                continue;
            }
        }
        let ctx = Arc::clone(&ctx);
        tokio::spawn(async move {
            if let Err(e) =
                handle_connection(stream, &ctx.0, ctx.1.as_ref(), ctx.2.as_ref(), &ctx.3).await
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
    recorder: &Recorder,
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
            Ok(req) => dispatch_request(&req, engine, eval_fn, list_fn, recorder).await,
            Err(e) => Response::error(0, -32700, format!("Parse error: {e}")),
        };

        let mut resp_bytes = serde_json::to_vec(&response)?;
        resp_bytes.push(b'\n');
        writer.write_all(&resp_bytes).await?;
        writer.flush().await?;
    }

    Ok(())
}

async fn dispatch_request(
    req: &Request,
    engine: &EvalEngine,
    eval_fn: Option<&EvalFn>,
    list_fn: Option<&ListWindowsFn>,
    recorder: &Recorder,
) -> Response {
    match handler::dispatch(
        &req.method,
        req.params.as_ref(),
        engine,
        eval_fn,
        list_fn,
        recorder,
    )
    .await
    {
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
        let handle =
            tokio::spawn(
                async move { run(listener, guard, engine, None, None, Recorder::new()).await },
            );
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

    #[test]
    fn test_socket_dir_from_returns_xdg_runtime_dir_when_set_and_private() {
        use std::os::unix::fs::PermissionsExt;
        // Create a private temp dir (0o700) to simulate a valid XDG_RUNTIME_DIR.
        let dir = std::env::temp_dir().join(format!("tauri-pilot-xdg-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let result = socket_dir_from(Some(dir.as_os_str().to_owned()));
        let _ = std::fs::remove_dir(&dir);
        assert_eq!(result, dir);
    }

    #[test]
    fn test_socket_dir_from_falls_back_to_tmp_when_none() {
        let result = socket_dir_from(None);
        assert_eq!(result, std::path::PathBuf::from("/tmp"));
    }

    #[test]
    fn test_socket_dir_from_falls_back_to_tmp_when_empty() {
        let result = socket_dir_from(Some(std::ffi::OsString::new()));
        assert_eq!(result, std::path::PathBuf::from("/tmp"));
    }

    #[tokio::test]
    async fn test_bind_socket_has_mode_0o600() {
        use std::os::unix::fs::PermissionsExt;
        let socket = unique_socket_path();
        let (listener, guard) = bind(&socket).expect("bind test socket");
        let meta = std::fs::metadata(&socket).expect("socket metadata");
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "socket must be owner-only (0o600), got {mode:#o}"
        );
        drop(listener);
        drop(guard);
    }
}
