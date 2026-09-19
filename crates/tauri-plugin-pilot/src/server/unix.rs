use super::handle_connection;

use crate::error::Error;
use crate::eval::EvalEngine;
use crate::recorder::Recorder;
use crate::webview::Webviews;

use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UnixListener;

/// RAII guard that removes the socket file on drop.
///
/// Drop unlinks the pathname socket when the recorded inode still matches.
/// The plugin stores this guard in plugin state and drops it on
/// `RunEvent::Exit`, so a normal quit removes the file before tao calls
/// `process::exit`. A crash or `SIGKILL` still leaves the file; the next bind
/// treats `ConnectionRefused` as stale and unlinks it.
///
/// Stores the socket file's inode at bind time so it only unlinks its own
/// socket, not one created by an overlapping instance.
pub struct SocketGuard {
    path: std::path::PathBuf,
    inode: u64,
}

impl SocketGuard {
    /// Records the inode of the socket file just bound at `path`.
    ///
    /// Reads it from the path, as `Drop` does: `fstat` on the listener fd
    /// reports its sockfs inode, which never matches the socket file (#165).
    ///
    /// # Errors
    /// Returns an error if `path` cannot be stat'ed.
    fn new(path: &std::path::Path) -> std::io::Result<Self> {
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            path: path.to_path_buf(),
            inode: std::fs::metadata(path)?.ino(),
        })
    }
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

/// Returns true if `path` is a directory owned by the current user with no group/world permissions.
#[cfg(not(target_os = "android"))]
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
#[cfg(not(target_os = "android"))]
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
#[cfg(not(target_os = "android"))]
fn socket_dir() -> std::path::PathBuf {
    socket_dir_from(std::env::var_os("XDG_RUNTIME_DIR"))
}

/// Android uses a random per-instance abstract name; other Unix platforms use a path.
///
/// # Errors
/// Returns an error if OS randomness cannot be read or the address is invalid or too long.
pub fn socket_address(identifier: &str) -> std::io::Result<SocketAddr> {
    #[cfg(target_os = "android")]
    {
        use std::os::android::net::SocketAddrExt;

        SocketAddr::from_abstract_name(android_socket_name(identifier)?)
    }
    #[cfg(not(target_os = "android"))]
    SocketAddr::from_pathname(socket_dir().join(format!("tauri-pilot-{identifier}.sock")))
}

#[cfg(any(target_os = "android", test))]
fn android_socket_name(identifier: &str) -> std::io::Result<String> {
    use std::io::Read;

    let mut random = [0_u8; 8];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
    // The identifier is only a readable label; cap it to leave room for the random suffix.
    Ok(format!(
        "tauri-pilot-{identifier:.16}-{:016x}.sock",
        u64::from_ne_bytes(random)
    ))
}

/// Bind the socket using the **std** (sync) listener so this can be called
/// outside a tokio runtime (e.g. from Tauri plugin `setup`).
///
/// Returns the listener and a cleanup guard for pathname sockets. Abstract sockets
/// have no guard because the kernel releases their names when the listener closes.
///
/// Pathname bind serializes stale replacement with a sibling lock file so two
/// instances cannot steal the path. The lock is released before this returns.
/// The sibling `{socket}.lock` file may remain and is reused on the next bind.
///
/// # Errors
/// Rejects unnamed addresses. Propagates I/O errors from binding, the sibling
/// lock file, or socket configuration.
pub fn bind(
    address: &SocketAddr,
) -> Result<(std::os::unix::net::UnixListener, Option<SocketGuard>), Error> {
    if address.is_unnamed() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "tauri-pilot requires a named Unix socket",
        )
        .into());
    }
    match address.as_pathname() {
        Some(path) => bind_pathname(path).map(|(listener, guard)| (listener, Some(guard))),
        None => bind_abstract(address).map(|listener| (listener, None)),
    }
}

/// Abstract addresses use peer credentials, with no filesystem permissions or cleanup.
fn bind_abstract(address: &SocketAddr) -> Result<std::os::unix::net::UnixListener, Error> {
    let listener = std::os::unix::net::UnixListener::bind_addr(address)?;
    listener.set_nonblocking(true)?;
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        ?address,
        "tauri-pilot socket listening"
    );
    Ok(listener)
}

/// Exclusive flock for pathname bind. Closing the fd releases it, so the
/// file must live until `SocketGuard` has recorded the inode.
#[must_use]
struct BindLock {
    _file: std::fs::File,
}

/// Sibling of `socket_path` used only as a flock inode.
///
/// The suffix is appended to the full socket pathname (`foo.sock.lock`) so
/// it cannot collide with the socket file. The lock must not be the socket
/// itself: `unlink` + rebind replaces that inode and would drop the flock
/// mid-section. The socket directory is not used either (`/tmp` is
/// world-writable).
fn bind_lock_path(socket_path: &std::path::Path) -> std::path::PathBuf {
    let mut lock = socket_path.as_os_str().to_owned();
    lock.push(".lock");
    std::path::PathBuf::from(lock)
}

/// Removes a pathname socket and its sibling lock file.
#[cfg(test)]
pub(crate) fn cleanup_bind_files(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(bind_lock_path(path));
}

fn acquire_bind_lock(socket_path: &std::path::Path) -> std::io::Result<BindLock> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let path = bind_lock_path(socket_path);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        // Reuse a leftover lock file; flock is on the inode, not the contents.
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)?;
    let meta = file.metadata()?;
    // SAFETY: getuid() has no preconditions.
    let my_uid = unsafe { libc::getuid() };
    if !meta.is_file() || meta.uid() != my_uid || meta.nlink() != 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "bind lock must be a regular file owned by this user",
        ));
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.lock()?;
    Ok(BindLock { _file: file })
}

/// Applies owner-only permissions and removes stale pathname sockets only when
/// connecting confirms that no live listener remains.
///
/// An exclusive flock on a sibling lock file covers the stale probe, unlink,
/// rebind, and inode read so two starters cannot steal the path (#195). The
/// lock is released when this function returns, not for the server lifetime
/// (#152). Does not change the process umask around bind: umask is
/// per-process, and other threads would inherit a `0o177` mask (#172).
/// `set_permissions(0o600)` still runs after bind, and every connection is
/// checked against the peer UID.
fn bind_pathname(
    socket_path: &std::path::Path,
) -> Result<(std::os::unix::net::UnixListener, SocketGuard), Error> {
    let _lock = acquire_bind_lock(socket_path)?;
    let first_bind = std::os::unix::net::UnixListener::bind(socket_path);

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
                    std::os::unix::net::UnixListener::bind(socket_path)?
                }
                Err(e) => {
                    return Err(Error::Io(e));
                }
            }
        }
        Err(e) => return Err(Error::Io(e)),
    };
    // Taken before any later setup step can fail: an error there drops the
    // guard, and the file with it, while the listener is still open.
    let guard = SocketGuard::new(socket_path)?;

    // Restrict socket to owner-only access (defense-in-depth alongside XDG_RUNTIME_DIR).
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))?;

    // Must be non-blocking for tokio conversion
    listener.set_nonblocking(true)?;

    tracing::info!(version = env!("CARGO_PKG_VERSION"), path = %socket_path.display(), "tauri-pilot socket listening");
    Ok((listener, guard))
}

/// Run the accept loop on a pre-bound std listener. Converts to tokio internally.
///
/// The plugin passes `None` and holds `SocketGuard` in plugin state so Exit can
/// drop it (#194). Tests may still pass a guard so aborting the task unlinks.
pub async fn run(
    listener: std::os::unix::net::UnixListener,
    guard: Option<SocketGuard>,
    engine: EvalEngine,
    webviews: Arc<dyn Webviews>,
    recorder: Recorder,
) {
    let listener = match UnixListener::from_std(listener) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("failed to convert listener to tokio: {e}");
            return;
        }
    };
    // Declared after `listener` so it drops first. While the listener is open,
    // a starting instance sees the socket as live and leaves it alone, so it
    // cannot swap in its own between the guard's inode check and its unlink.
    let _guard = guard;
    if let Err(e) = accept_loop(&listener, engine, webviews, recorder).await {
        tracing::error!("socket server error: {e}");
    }
}

async fn accept_loop(
    listener: &UnixListener,
    engine: EvalEngine,
    webviews: Arc<dyn Webviews>,
    recorder: Recorder,
) -> Result<(), Error> {
    let ctx = Arc::new((engine, webviews, recorder));

    loop {
        let (stream, _addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::warn!("accept error: {e}");
                continue;
            }
        };

        // Authenticate kernel-provided credentials before reading requests.
        match stream.peer_cred() {
            Ok(cred) => {
                // SAFETY: getuid() is always safe to call; it has no preconditions.
                let my_uid = unsafe { libc::getuid() };
                #[cfg(target_os = "android")]
                let authorized = android_peer_allowed(cred.uid(), cred.gid(), my_uid);
                #[cfg(not(target_os = "android"))]
                let authorized = cred.uid() == my_uid;
                if !authorized {
                    tracing::warn!(
                        peer_uid = cred.uid(),
                        peer_gid = cred.gid(),
                        app_uid = my_uid,
                        "rejected unauthorized Unix peer"
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
            if let Err(e) = handle_connection(stream, &ctx.0, ctx.1.as_ref(), &ctx.2).await {
                tracing::warn!("connection error: {e}");
            }
        });
    }
}

/// Accepts the canonical app, root (0) and ADB shell (2000) UID/GID pairs.
/// Peers with a different primary group are outside this policy, matching
/// Chromium's Android `DevTools` credential check.
#[cfg(any(target_os = "android", test))]
fn android_peer_allowed(uid: u32, gid: u32, app_uid: u32) -> bool {
    uid == gid && (uid == app_uid || uid == 0 || uid == 2000)
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;
    use crate::protocol::Response;
    use crate::webview::fake::FakeWebviews;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    #[test]
    fn android_socket_name_fits_with_long_identifier() {
        let name = android_socket_name(&"a".repeat(200)).expect("socket name");
        assert!(name.len() <= 107, "abstract socket name is too long");
        #[cfg(target_os = "linux")]
        {
            use std::os::linux::net::SocketAddrExt;
            let address = SocketAddr::from_abstract_name(name).expect("valid abstract address");
            bind(&address).expect("bind abstract socket");
        }
    }

    #[test]
    fn android_peer_credentials() {
        let app_uid = 10123;
        for (uid, gid, allowed) in [
            (app_uid, app_uid, true),
            (0, 0, true),
            (2000, 2000, true),
            (app_uid, 0, false),
            (2000, 0, false),
            (10124, 10124, false),
        ] {
            assert_eq!(
                android_peer_allowed(uid, gid, app_uid),
                allowed,
                "{uid}:{gid}"
            );
        }
    }

    #[test]
    fn bind_rejects_unnamed_address() {
        let socket = std::os::unix::net::UnixDatagram::unbound().expect("unnamed socket");
        let address = socket.local_addr().expect("socket address");
        assert!(
            matches!(bind(&address), Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::InvalidInput)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn abstract_listener_is_nonblocking_without_file_guard() {
        use std::os::linux::net::SocketAddrExt;
        use std::os::unix::io::AsRawFd;

        let name = format!("tauri-pilot-abstract-test-{}", std::process::id());
        let address = SocketAddr::from_abstract_name(name).expect("abstract address");
        let (listener, guard) = bind(&address).expect("bind abstract socket");
        assert!(guard.is_none());
        // SAFETY: the listener owns a valid descriptor; F_GETFL only reads its flags.
        let flags = unsafe { libc::fcntl(listener.as_raw_fd(), libc::F_GETFL) };
        assert!(flags >= 0, "read listener flags");
        assert_ne!(flags & libc::O_NONBLOCK, 0);
    }

    fn unique_socket_path() -> PathBuf {
        let n = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        PathBuf::from(format!(
            "/tmp/tauri-pilot-test-{}-{n}.sock",
            std::process::id()
        ))
    }

    fn plant_stale_socket(path: &Path) {
        let stale = std::os::unix::net::UnixListener::bind(path).expect("plant stale socket");
        drop(stale);
        assert!(
            path.exists(),
            "stale socket file must remain after listener drop"
        );
    }

    fn path_reaches_listener(listener: &std::os::unix::net::UnixListener, path: &Path) -> bool {
        if std::os::unix::net::UnixStream::connect(path).is_err() {
            return false;
        }
        listener.accept().is_ok()
    }

    #[test]
    fn overlapping_stale_replaces_leave_one_reachable_winner() {
        for i in 0..200 {
            let socket = unique_socket_path();
            plant_stale_socket(&socket);
            let a_path = socket.clone();
            let b_path = socket.clone();
            let a = std::thread::spawn(move || {
                let address = SocketAddr::from_pathname(&a_path).expect("test socket address");
                bind(&address)
            });
            let b = std::thread::spawn(move || {
                let address = SocketAddr::from_pathname(&b_path).expect("test socket address");
                bind(&address)
            });
            let ra = a.join().expect("thread a");
            let rb = b.join().expect("thread b");

            let mut oks = Vec::new();
            for result in [ra, rb] {
                match result {
                    Ok(pair) => oks.push(pair),
                    Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::AddrInUse => {}
                    Err(e) => panic!("iteration {i}: unexpected bind error {e}"),
                }
            }
            let reachable = oks
                .iter()
                .filter(|(listener, _)| path_reaches_listener(listener, &socket))
                .count();
            let ok_count = oks.len();
            for (listener, guard) in oks {
                drop(guard);
                drop(listener);
            }
            cleanup_bind_files(&socket);
            assert_eq!(
                (ok_count, reachable),
                (1, 1),
                "iteration {i}: one bind must win and own the path"
            );
        }
    }

    #[test]
    fn live_socket_is_already_in_use_without_waiting() {
        let socket = unique_socket_path();
        let address = SocketAddr::from_pathname(&socket).expect("test socket address");
        let (listener, guard) = bind(&address).expect("first bind");
        let started = std::time::Instant::now();
        let second = bind(&address);
        let elapsed = started.elapsed();
        drop(guard);
        drop(listener);
        cleanup_bind_files(&socket);
        assert!(
            matches!(&second, Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::AddrInUse),
            "live socket must stay in use"
        );
        assert!(
            elapsed < std::time::Duration::from_millis(200),
            "second bind waited {elapsed:?}; lock must not outlive bind"
        );
    }

    #[test]
    fn leftover_bind_lock_file_does_not_block_bind() {
        use std::os::unix::fs::PermissionsExt;

        let socket = unique_socket_path();
        let lock_path = super::bind_lock_path(&socket);
        std::fs::write(&lock_path, b"").expect("plant leftover lock");
        std::fs::set_permissions(&lock_path, std::fs::Permissions::from_mode(0o666))
            .expect("widen leftover lock");
        let address = SocketAddr::from_pathname(&socket).expect("test socket address");
        let (listener, guard) = bind(&address).expect("bind with leftover lock file");
        let mode = std::fs::metadata(&lock_path)
            .expect("lock metadata")
            .permissions()
            .mode()
            & 0o777;
        drop(guard);
        drop(listener);
        cleanup_bind_files(&socket);
        assert_eq!(
            mode, 0o600,
            "reused leftover lock must be owner-only, got {mode:#o}"
        );
    }

    #[test]
    fn bind_releases_sibling_lock_before_returning() {
        let socket = unique_socket_path();
        let address = SocketAddr::from_pathname(&socket).expect("test socket address");
        let (listener, guard) = bind(&address).expect("bind test socket");
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(super::bind_lock_path(&socket))
            .expect("open sibling lock after bind");
        lock.try_lock()
            .expect("sibling lock must be free after bind returns");
        drop(guard);
        drop(listener);
        drop(lock);
        cleanup_bind_files(&socket);
    }

    #[test]
    fn bind_fails_when_sibling_lock_cannot_be_created() {
        let socket = unique_socket_path();
        std::fs::create_dir(super::bind_lock_path(&socket)).expect("lock path is a directory");
        let address = SocketAddr::from_pathname(&socket).expect("test socket address");
        let result = bind(&address);
        let bound = socket.exists();
        let _ = std::fs::remove_dir(super::bind_lock_path(&socket));
        cleanup_bind_files(&socket);
        assert!(
            result.is_err(),
            "must not bind if the sibling lock cannot be created"
        );
        assert!(!bound, "lock create failure must not leave a bound socket");
    }

    #[test]
    fn sibling_lock_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let socket = unique_socket_path();
        let address = SocketAddr::from_pathname(&socket).expect("test socket address");
        let (listener, guard) = bind(&address).expect("bind test socket");
        let mode = std::fs::metadata(super::bind_lock_path(&socket))
            .expect("lock metadata")
            .permissions()
            .mode()
            & 0o777;
        drop(guard);
        drop(listener);
        cleanup_bind_files(&socket);
        assert_eq!(mode, 0o600, "lock file must be owner-only, got {mode:#o}");
    }

    async fn start_test_server(path: &Path) -> tokio::task::JoinHandle<()> {
        let address = SocketAddr::from_pathname(path).expect("test socket address");
        let (listener, guard) = bind(&address).expect("bind test socket");
        let engine = EvalEngine::new();
        let handle = tokio::spawn(async move {
            run(
                listener,
                guard,
                engine,
                Arc::new(FakeWebviews::default()),
                Recorder::new(),
            )
            .await;
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        handle
    }

    #[tokio::test]
    async fn test_server_responds_ping_ok() {
        let socket = unique_socket_path();
        let handle = start_test_server(&socket).await;

        let stream = UnixStream::connect(&socket)
            .await
            .expect("connect test socket");
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        writer
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n")
            .await
            .expect("write ping request");
        writer.flush().await.expect("flush");

        let mut line = String::new();
        reader.read_line(&mut line).await.expect("read response");
        let resp: Response = serde_json::from_str(&line).expect("parse response");

        assert_eq!(resp.id, serde_json::json!(1));
        assert!(resp.error.is_none());
        let result = resp.result.expect("ping returns a result");
        assert_eq!(result["status"], serde_json::json!("ok"));
        assert_eq!(
            result["plugin_version"],
            serde_json::json!(env!("CARGO_PKG_VERSION"))
        );

        handle.abort();
        cleanup_bind_files(&socket);
    }

    #[tokio::test]
    async fn test_server_handles_invalid_json() {
        let socket = unique_socket_path();
        let handle = start_test_server(&socket).await;

        let stream = UnixStream::connect(&socket)
            .await
            .expect("connect test socket");
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        writer
            .write_all(b"not json\n")
            .await
            .expect("write invalid request");
        writer.flush().await.expect("flush");

        let mut line = String::new();
        reader.read_line(&mut line).await.expect("read response");
        let resp: Response = serde_json::from_str(&line).expect("parse response");

        assert_eq!(resp.id, serde_json::Value::Null);
        let err = resp.error.expect("error payload present");
        assert_eq!(err.code, -32700);

        handle.abort();
        cleanup_bind_files(&socket);
    }

    #[tokio::test]
    async fn test_server_reads_lines_up_to_one_mib() {
        // The CLI mirrors this limit as `MAX_REQUEST_LEN` and refuses longer
        // requests before sending them, so both sides must agree (#214).
        let socket = unique_socket_path();
        let handle = start_test_server(&socket).await;

        let stream = UnixStream::connect(&socket)
            .await
            .expect("connect test socket");
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        // A ping padded with spaces, which the server trims, to `len` bytes.
        let ping = |id: u32, len: usize| {
            let req = format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"ping"}}"#);
            format!("{req}{}\n", " ".repeat(len - req.len() - 1))
        };

        writer
            .write_all(ping(1, 1_048_576).as_bytes())
            .await
            .expect("write 1 MiB line");
        writer.flush().await.expect("flush");
        let mut line = String::new();
        reader.read_line(&mut line).await.expect("read response");
        let resp: Response = serde_json::from_str(&line).expect("parse response");
        assert_eq!(resp.id, serde_json::json!(1));
        assert!(resp.error.is_none(), "a 1 MiB line is read");

        writer
            .write_all(ping(2, 1_048_577).as_bytes())
            .await
            .expect("write longer line");
        writer.flush().await.expect("flush");
        line.clear();
        reader.read_line(&mut line).await.expect("read response");
        let resp: Response = serde_json::from_str(&line).expect("parse response");
        assert_eq!(resp.id, serde_json::Value::Null);
        assert_eq!(resp.error.expect("error payload present").code, -32700);
        line.clear();
        let n = reader.read_line(&mut line).await.expect("read EOF");
        assert_eq!(n, 0, "the server hangs up after a longer line");

        handle.abort();
        cleanup_bind_files(&socket);
    }

    #[tokio::test]
    async fn test_server_handles_multiple_requests() {
        let socket = unique_socket_path();
        let handle = start_test_server(&socket).await;

        let stream = UnixStream::connect(&socket)
            .await
            .expect("connect test socket");
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        for i in 1..=3 {
            let req = format!("{{\"jsonrpc\":\"2.0\",\"id\":{i},\"method\":\"test\"}}\n");
            writer
                .write_all(req.as_bytes())
                .await
                .expect("write request");
            writer.flush().await.expect("flush");

            let mut line = String::new();
            reader.read_line(&mut line).await.expect("read response");
            let resp: Response = serde_json::from_str(&line).expect("parse response");
            assert_eq!(resp.id, serde_json::json!(i));
        }

        handle.abort();
        cleanup_bind_files(&socket);
    }

    #[test]
    fn test_socket_dir_from_returns_xdg_runtime_dir_when_set_and_private() {
        use std::os::unix::fs::PermissionsExt;
        // Create a private temp dir (0o700) to simulate a valid XDG_RUNTIME_DIR.
        let dir = std::env::temp_dir().join(format!("tauri-pilot-xdg-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .expect("set dir permissions");
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
        let address = SocketAddr::from_pathname(&socket).expect("test socket address");
        let (listener, guard) = bind(&address).expect("bind test socket");
        let meta = std::fs::metadata(&socket).expect("socket metadata");
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "socket must be owner-only (0o600), got {mode:#o}"
        );
        drop(listener);
        drop(guard);
        cleanup_bind_files(&socket);
    }

    #[test]
    fn socket_guard_unlinks_only_its_own_socket() {
        // #165: the guard compared the path's inode with the listener fd's
        // sockfs inode. The two never match, so it never removed the socket.
        let socket = unique_socket_path();
        let address = SocketAddr::from_pathname(&socket).expect("test socket address");

        let (listener, guard) = bind(&address).expect("bind test socket");
        drop(listener);
        drop(guard);
        let own_left = socket.exists();
        cleanup_bind_files(&socket);
        assert!(!own_left, "guard must unlink the socket it bound");

        // Another instance re-bound the path: that socket is not ours to remove.
        // `_listener` must outlive the rebind: it pins the unlinked socket's
        // inode. Closed early, ext4 hands that inode to the new socket and the
        // guard deletes it.
        let (_listener, guard) = bind(&address).expect("bind test socket");
        std::fs::remove_file(&socket).expect("unlink bound socket");
        let _other = std::os::unix::net::UnixListener::bind(&socket).expect("rebind socket path");
        drop(guard);
        let other_kept = socket.exists();
        cleanup_bind_files(&socket);
        assert!(other_kept, "guard must not unlink a socket it did not bind");
    }

    // `/proc/self/status` exists on Linux only; macOS/BSD return None.
    fn proc_umask() -> Option<u32> {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        status.lines().find_map(|line| {
            line.strip_prefix("Umask:")
                .and_then(|rest| u32::from_str_radix(rest.trim(), 8).ok())
        })
    }

    fn wait_flag(flag: &AtomicBool) {
        while !flag.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
    }

    #[test]
    fn bind_does_not_restrict_directory_mode_on_other_threads() {
        // #172: umask(0o177) around bind made concurrent create_dir
        // produce 0o600 directories (no execute bit) and then EACCES.
        let scratch = std::env::temp_dir().join(format!(
            "tauri-pilot-umask-{}-{}",
            std::process::id(),
            TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&scratch).expect("scratch dir");
        let probe = scratch.join("probe");
        std::fs::create_dir(&probe).expect("probe dir");
        let probe_mode = std::fs::metadata(&probe)
            .expect("probe metadata")
            .permissions()
            .mode()
            & 0o777;
        let _ = std::fs::remove_dir(&probe);

        let baseline_umask = proc_umask();

        let stop = Arc::new(AtomicBool::new(false));
        let ready = Arc::new(AtomicBool::new(false));
        let mkdir_ok = Arc::new(AtomicU32::new(0));
        let restricted_mode = Arc::new(AtomicU32::new(u32::MAX));
        let drifted_umask = Arc::new(AtomicU32::new(u32::MAX));
        let observer = {
            let stop = Arc::clone(&stop);
            let ready = Arc::clone(&ready);
            let mkdir_ok = Arc::clone(&mkdir_ok);
            let restricted_mode = Arc::clone(&restricted_mode);
            let drifted_umask = Arc::clone(&drifted_umask);
            let scratch = scratch.clone();
            std::thread::spawn(move || {
                ready.store(true, Ordering::Release);
                let mut n = 0u32;
                while !stop.load(Ordering::Acquire) {
                    if let (Some(base), Some(now)) = (baseline_umask, proc_umask())
                        && now != base
                    {
                        drifted_umask.store(now, Ordering::Relaxed);
                    }
                    let dir = scratch.join(format!("d{n}"));
                    n = n.wrapping_add(1);
                    if std::fs::create_dir(&dir).is_ok() {
                        mkdir_ok.fetch_add(1, Ordering::Relaxed);
                        if let Ok(meta) = std::fs::metadata(&dir) {
                            let mode = meta.permissions().mode() & 0o777;
                            if mode & 0o111 == 0 {
                                restricted_mode.store(mode, Ordering::Relaxed);
                            }
                        }
                        let _ = std::fs::remove_dir(&dir);
                    }
                }
            })
        };

        wait_flag(&ready);
        for _ in 0..500 {
            let socket = unique_socket_path();
            let address = SocketAddr::from_pathname(&socket).expect("test socket address");
            let (listener, guard) = bind(&address).expect("bind test socket");
            drop(guard);
            drop(listener);
            cleanup_bind_files(&socket);
        }

        stop.store(true, Ordering::Release);
        observer.join().expect("observer");
        let created = mkdir_ok.load(Ordering::Relaxed);
        let mode = restricted_mode.load(Ordering::Relaxed);
        let _ = std::fs::remove_dir_all(&scratch);
        assert!(created > 0, "observer never created a directory");
        if probe_mode & 0o111 != 0 {
            assert_eq!(
                mode,
                u32::MAX,
                "bind must not restrict umask; concurrent mkdir got {mode:#o}"
            );
        }
        // Umask sampling needs `/proc`; skip the drift half off Linux.
        #[cfg(target_os = "linux")]
        {
            let umask_now = drifted_umask.load(Ordering::Relaxed);
            assert_eq!(
                umask_now,
                u32::MAX,
                "process umask changed to {umask_now:#o} during bind"
            );
        }
    }
}
