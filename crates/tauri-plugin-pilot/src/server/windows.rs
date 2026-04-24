use super::{EvalFn, FocusFn, ListWindowsFn, handle_connection};

use crate::error::Error;
use crate::eval::EvalEngine;
#[allow(unused_imports)]
use crate::protocol::Response;
use crate::recorder::Recorder;

use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::ffi::c_void;
use std::mem;
use std::mem::MaybeUninit;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
#[allow(unused_imports)]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, PSID};
use windows::Win32::Security::{
    ACL, ACL_REVISION, AddAccessAllowedAce, EqualSid, GetLengthSid, GetTokenInformation,
    InitializeAcl, InitializeSecurityDescriptor, PSECURITY_DESCRIPTOR, RevertToSelf,
    SECURITY_ATTRIBUTES, SetSecurityDescriptorDacl, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows::Win32::System::Pipes::ImpersonateNamedPipeClient;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken,
};

pub fn socket_path(identifier: &str) -> PathBuf {
    PathBuf::from(format!(r"\\.\pipe\tauri-pilot-{identifier}"))
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct InstanceEntry {
    pub pipe: String,
    pub pid: u32,
    pub created_at: u64,
}

fn instances_dir() -> std::io::Result<PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "LOCALAPPDATA environment variable is not set or empty",
            )
        })?;
    Ok(PathBuf::from(local_app_data)
        .join("tauri-pilot")
        .join("instances"))
}

fn instance_file_path(identifier: &str) -> std::io::Result<PathBuf> {
    let dir = instances_dir()?;
    // The instances directory sits under %LOCALAPPDATA%, which already inherits
    // user-only ACLs from the user profile, so no extra DACL is needed here.
    // `create_dir_all` is recursive (unlike `CreateDirectoryW`) and is a no-op
    // when the directory already exists.
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(format!("{identifier}.json")))
}

fn atomic_write_instance(path: &Path, entry: &InstanceEntry) -> std::io::Result<()> {
    let json = serde_json::to_string(entry)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn register_instance(identifier: &str, pipe_path: &Path) -> std::io::Result<()> {
    let path = instance_file_path(identifier)?;
    let entry = InstanceEntry {
        pipe: pipe_path.to_string_lossy().into_owned(),
        pid: std::process::id(),
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    atomic_write_instance(&path, &entry)
}

fn unregister_instance(identifier: &str) -> std::io::Result<()> {
    let path = instance_file_path(identifier)?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

#[allow(dead_code)] // used by `discover_instances`, kept for symmetry with the CLI resolver
fn is_pid_alive(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: OpenProcess is a Win32 call that returns a HANDLE or an error.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) };
    match handle {
        Ok(h) => {
            // SAFETY: `h` is a valid handle returned by OpenProcess.
            let alive = unsafe {
                let mut exit_code: u32 = 0;
                GetExitCodeProcess(h, &raw mut exit_code).is_ok()
                    && exit_code == STILL_ACTIVE.0 as u32
            };
            // SAFETY: closing the handle we just opened.
            unsafe {
                let _ = CloseHandle(h);
            };
            alive
        }
        Err(_) => false,
    }
}

#[allow(dead_code)] // public helper for future CLI integration; not currently called
pub(crate) fn discover_instances() -> std::io::Result<Vec<InstanceEntry>> {
    let dir = instances_dir()?;
    let mut instances = Vec::new();
    if !dir.exists() {
        return Ok(instances);
    }
    let entries = std::fs::read_dir(&dir)?;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "skipping corrupted instance file");
                continue;
            }
        };
        let info: InstanceEntry = match serde_json::from_str(&content) {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "skipping malformed instance file");
                continue;
            }
        };
        if !is_pid_alive(info.pid) {
            tracing::debug!(pid = info.pid, path = %path.display(), "skipping stale instance");
            continue;
        }
        instances.push(info);
    }
    Ok(instances)
}

#[allow(dead_code)] // public helper for future CLI integration; not currently called
pub(crate) fn find_newest_instance() -> std::io::Result<Option<InstanceEntry>> {
    let instances = discover_instances()?;
    Ok(instances.into_iter().max_by_key(|i| i.created_at))
}

pub struct RegistryGuard {
    identifier: String,
}

impl Drop for RegistryGuard {
    fn drop(&mut self) {
        if let Err(e) = unregister_instance(&self.identifier) {
            tracing::warn!(identifier = %self.identifier, error = %e, "failed to remove registry entry");
        } else {
            tracing::info!(identifier = %self.identifier, "registry entry removed");
        }
    }
}

// ---------------------------------------------------------------------------
// Security: restrict the named pipe to the creating user only (DACL-only)
// ---------------------------------------------------------------------------

/// Owns a raw allocation backing an ACL with the correct layout.
struct AclBuffer {
    ptr: *mut ACL,
    layout: Layout,
}

impl AclBuffer {
    fn as_ptr(&self) -> *mut ACL {
        self.ptr
    }
}

impl Drop for AclBuffer {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: `ptr` was allocated by `alloc_zeroed` with the same `layout`
            // and has not been freed yet.
            unsafe {
                dealloc(self.ptr.cast::<u8>(), self.layout);
            }
        }
    }
}

/// RAII wrapper for a Win32 `HANDLE` that closes it on drop.
struct OwnedHandle(HANDLE);

impl OwnedHandle {
    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if self.0.0 != 0 {
            // SAFETY: `self.0` was returned by a successful `Open*Token` call
            // and has not been closed yet.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// Owns the buffers backing a [`SECURITY_ATTRIBUTES`].
struct SecurityAttributesGuard {
    /// Backing storage for the `SECURITY_DESCRIPTOR`; must outlive the pipe creation.
    _sd: Box<MaybeUninit<windows::Win32::Security::SECURITY_DESCRIPTOR>>,
    /// Backing storage for the ACL; must outlive the pipe creation.
    _acl: AclBuffer,
    /// Backing storage for the SID (referenced by the ACE we add to the ACL).
    _sid_buf: Vec<u8>,
    /// The token handle used to obtain the SID.
    _token: OwnedHandle,
}

/// Opens the current process token for reading the creator's SID.
fn open_process_token() -> std::io::Result<OwnedHandle> {
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle that does not need closing.
    let process = unsafe { GetCurrentProcess() };
    let mut token = HANDLE(0);
    // SAFETY: `process` is a valid pseudo-handle; `token` points to stack-local storage.
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) }
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(OwnedHandle(token))
}

/// Opens the current thread's impersonation token. Must be called after
/// [`ImpersonateNamedPipeClient`] so the thread is impersonating the peer.
fn open_thread_impersonation_token() -> std::io::Result<OwnedHandle> {
    // SAFETY: `GetCurrentThread` returns a pseudo-handle that does not need closing.
    let thread = unsafe { GetCurrentThread() };
    let mut token = HANDLE(0);
    // SAFETY: `thread` is a valid pseudo-handle; `token` points to stack-local storage.
    // `OpenThreadToken` reads the impersonation token from the thread, which is the
    // client's token after `ImpersonateNamedPipeClient`.
    unsafe { OpenThreadToken(thread, TOKEN_QUERY, true, &raw mut token) }
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(OwnedHandle(token))
}

/// Returns the SID owned by `token`, along with the backing buffer that the SID
/// pointer references. The caller MUST keep the returned `Vec<u8>` alive for as
/// long as the pointer is dereferenced (fix for the prior use-after-free).
fn get_user_sid(token: &OwnedHandle) -> std::io::Result<(Vec<u8>, PSID)> {
    let mut return_length = 0u32;
    // First call is expected to fail with ERROR_INSUFFICIENT_BUFFER — it just
    // writes the required size into `return_length`.
    // SAFETY: `token` is a valid handle; `return_length` points to stack storage.
    unsafe {
        let _ = GetTokenInformation(token.raw(), TokenUser, None, 0, &raw mut return_length);
    }

    if return_length == 0 {
        return Err(std::io::Error::other(
            "GetTokenInformation returned zero size",
        ));
    }

    let mut buf = vec![0u8; return_length as usize];
    // SAFETY: `buf` is a valid, sized byte buffer; `token` is a valid handle.
    unsafe {
        GetTokenInformation(
            token.raw(),
            TokenUser,
            Some(buf.as_mut_ptr().cast::<c_void>()),
            return_length,
            &raw mut return_length,
        )
    }
    .map_err(|e| std::io::Error::other(e.to_string()))?;

    // SAFETY: `buf` holds a valid `TOKEN_USER` laid out by the kernel with the
    // correct alignment for `TOKEN_USER` (padded by `GetTokenInformation`). The
    // `Sid` pointer it contains references memory inside `buf`, which we keep
    // alive by returning the buffer to the caller.
    #[allow(clippy::cast_ptr_alignment)]
    let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    Ok((buf, sid))
}

/// Checks whether the connected client's SID matches the current user's SID.
/// Returns `false` only if we proved they are different. Any failure along the
/// way is treated as "matches" so that DACL (which is the primary defence)
/// remains the source of truth and this serves purely as a defence-in-depth check.
fn client_sid_matches_current_user(pipe: &NamedPipeServer) -> bool {
    // SAFETY: `pipe.as_raw_handle()` returns the kernel handle for the pipe server.
    if unsafe { ImpersonateNamedPipeClient(HANDLE(pipe.as_raw_handle() as isize)) }.is_err() {
        tracing::warn!("failed to impersonate named pipe client");
        return true;
    }

    // Open the client's token from the thread (NOT the process).
    let client_token = match open_thread_impersonation_token() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "failed to open thread impersonation token");
            // SAFETY: we are still impersonating; revert before returning.
            unsafe {
                let _ = RevertToSelf();
            }
            return true;
        }
    };

    let client_sid_result = get_user_sid(&client_token);

    // Revert impersonation as soon as we have read the client SID (or failed to).
    // SAFETY: we called `ImpersonateNamedPipeClient` above; this undoes it.
    unsafe {
        let _ = RevertToSelf();
    }

    let (_client_buf, client_sid) = match client_sid_result {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "failed to read client SID");
            return true;
        }
    };

    let our_token = match open_process_token() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "failed to open process token");
            return true;
        }
    };

    let (_our_buf, our_sid) = match get_user_sid(&our_token) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "failed to read own SID");
            return true;
        }
    };

    // SAFETY: both SID pointers are backed by `_client_buf` and `_our_buf`, which
    // stay alive for the duration of this call.
    unsafe { EqualSid(client_sid, our_sid) }.is_ok()
}

/// Allocates and initializes the ACL granting the given SID access.
/// Fixes the prior heap overflow: the layout now matches the `acl_size` we pass
/// to `InitializeAcl`, not `sizeof::<ACL>()`.
fn build_acl(user_sid: PSID) -> std::io::Result<AclBuffer> {
    // SAFETY: `user_sid` points to a valid SID owned by the caller's buffer.
    let sid_length = unsafe { GetLengthSid(user_sid) } as usize;

    // ACL header (8) + ACE header (4) + ACE access mask (4) + SID, rounded up
    // to a DWORD boundary. This is exactly what `InitializeAcl` will expect.
    let acl_size = (8 + 4 + 4 + sid_length + 3) & !3;

    let layout = Layout::from_size_align(acl_size, mem::align_of::<ACL>())
        .map_err(|e| std::io::Error::other(format!("invalid ACL layout: {e}")))?;

    // SAFETY: `layout` has a non-zero size and an alignment that matches `ACL`'s
    // alignment requirement (enforced by `from_size_align` above).
    #[allow(clippy::cast_ptr_alignment)]
    let ptr = unsafe { alloc_zeroed(layout) }.cast::<ACL>();
    if ptr.is_null() {
        // Do NOT dealloc a null pointer — it is UB. Just return.
        return Err(std::io::Error::other("failed to allocate ACL"));
    }

    // Wrap immediately so any ? below runs the destructor.
    let buffer = AclBuffer { ptr, layout };

    // SAFETY: `ptr` is valid, aligned, zeroed memory of exactly `acl_size` bytes.
    // `acl_size` fits in u32: ACL header (8) + ACE header (4) + access mask (4)
    // + SID (max ~68 bytes) rounded up to DWORD is well under 64 KB.
    unsafe {
        InitializeAcl(
            buffer.as_ptr(),
            u32::try_from(acl_size).expect("ACL size fits in u32"),
            ACL_REVISION,
        )
    }
    .map_err(|e| std::io::Error::other(e.to_string()))?;

    // SAFETY: `buffer.as_ptr()` is a freshly-initialised ACL with room for this ACE
    // (our `acl_size` accounted for the SID length), and `user_sid` is valid.
    unsafe {
        AddAccessAllowedAce(
            buffer.as_ptr(),
            ACL_REVISION,
            (GENERIC_READ | GENERIC_WRITE).0,
            user_sid,
        )
    }
    .map_err(|e| std::io::Error::other(e.to_string()))?;

    Ok(buffer)
}

/// Allocates a `SECURITY_DESCRIPTOR` and attaches the given ACL as its DACL.
fn build_security_descriptor(
    acl: &AclBuffer,
) -> std::io::Result<Box<MaybeUninit<windows::Win32::Security::SECURITY_DESCRIPTOR>>> {
    let sd_box = Box::new(MaybeUninit::<windows::Win32::Security::SECURITY_DESCRIPTOR>::uninit());
    let sd_ptr = PSECURITY_DESCRIPTOR(sd_box.as_ptr() as *mut c_void);

    // SAFETY: `sd_ptr` points to properly aligned, writable storage for a
    // `SECURITY_DESCRIPTOR`; `InitializeSecurityDescriptor` will initialise it.
    unsafe { InitializeSecurityDescriptor(sd_ptr, 1) }
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    // SAFETY: `sd_ptr` has just been initialised; `acl.as_ptr()` is a valid ACL.
    unsafe { SetSecurityDescriptorDacl(sd_ptr, true, Some(acl.as_ptr()), false) }
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    Ok(sd_box)
}

fn create_user_only_security_attributes()
-> std::io::Result<(SECURITY_ATTRIBUTES, SecurityAttributesGuard)> {
    let token = open_process_token()?;
    let (sid_buf, user_sid) = get_user_sid(&token)?;
    let acl = build_acl(user_sid)?;
    let sd = build_security_descriptor(&acl)?;

    let sd_ptr = PSECURITY_DESCRIPTOR(sd.as_ptr() as *mut c_void);

    let sa = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(mem::size_of::<SECURITY_ATTRIBUTES>())
            .expect("SECURITY_ATTRIBUTES size must fit in u32"),
        lpSecurityDescriptor: sd_ptr.0,
        bInheritHandle: windows::Win32::Foundation::BOOL(0),
    };

    let guard = SecurityAttributesGuard {
        _sd: sd,
        _acl: acl,
        _sid_buf: sid_buf,
        _token: token,
    };

    Ok((sa, guard))
}

// ---------------------------------------------------------------------------

pub fn bind(pipe_path: &Path) -> Result<(NamedPipeServer, RegistryGuard), Error> {
    // Refuse to downgrade security. If the DACL setup fails we fail hard rather
    // than creating a pipe with the default (broader) DACL.
    let (mut sa, _sec_guard) = create_user_only_security_attributes().map_err(Error::from)?;

    // SAFETY: `sa` and its backing buffers (owned by `_sec_guard`) are valid for
    // the duration of this call. The kernel copies the security descriptor, so
    // `_sec_guard` may be dropped after the pipe is created.
    let server = unsafe {
        ServerOptions::new()
            .first_pipe_instance(true)
            .pipe_mode(tokio::net::windows::named_pipe::PipeMode::Byte)
            .create_with_security_attributes_raw(pipe_path, (&raw mut sa).cast::<c_void>())
    }
    .map_err(Error::from)?;

    tracing::info!(path = %pipe_path.display(), "tauri-pilot named pipe listening");

    let identifier = pipe_path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("tauri-pilot-"))
        .unwrap_or("unknown")
        .to_string();

    register_instance(&identifier, pipe_path)?;
    let guard = RegistryGuard { identifier };

    Ok((server, guard))
}

pub async fn run(
    first_server: NamedPipeServer,
    guard: RegistryGuard,
    engine: EvalEngine,
    eval_fn: Option<EvalFn>,
    list_fn: Option<ListWindowsFn>,
    focus_fn: Option<FocusFn>,
    recorder: Recorder,
) {
    let identifier = guard.identifier.clone();
    if let Err(e) = accept_loop(
        first_server,
        &identifier,
        engine,
        eval_fn,
        list_fn,
        focus_fn,
        recorder,
    )
    .await
    {
        tracing::error!("named pipe server error: {e}");
    }
}

async fn accept_loop(
    first_server: NamedPipeServer,
    identifier: &str,
    engine: EvalEngine,
    eval_fn: Option<EvalFn>,
    list_fn: Option<ListWindowsFn>,
    focus_fn: Option<FocusFn>,
    recorder: Recorder,
) -> Result<(), Error> {
    let ctx = Arc::new((engine, eval_fn, list_fn, focus_fn, recorder));
    let mut server = first_server;
    let pipe_path = socket_path(identifier);

    loop {
        // A failure here means the current pipe is genuinely dead (handle closed,
        // etc.) — propagate it so the supervising task can react.
        server.connect().await?;

        // Build the next pipe instance. Transient failures (e.g. `ERROR_PIPE_BUSY`
        // or handle exhaustion) must NOT take the whole server down: log and
        // retry after a short back-off. Silent downgrade to default DACL is
        // refused — we'd rather drop a connection than weaken security.
        let next_server = loop {
            // Build SA, create the pipe, and drop the guard buffers in one scope so
            // no `*mut ACL` or `*mut c_void` crosses the retry `.await` below —
            // `CreateNamedPipe` copies the security descriptor by the time it returns.
            let create_result: std::io::Result<NamedPipeServer> = (|| {
                let (mut sa, _sec_guard) = create_user_only_security_attributes()?;
                // SAFETY: `sa` and its backing buffers are valid for this call.
                unsafe {
                    ServerOptions::new()
                        .pipe_mode(tokio::net::windows::named_pipe::PipeMode::Byte)
                        .create_with_security_attributes_raw(
                            &pipe_path,
                            (&raw mut sa).cast::<c_void>(),
                        )
                }
                .map_err(std::io::Error::other)
            })();

            match create_result {
                Ok(s) => break s,
                Err(e) => {
                    tracing::warn!(
                        path = %pipe_path.display(),
                        error = %e,
                        "transient failure creating next pipe instance, retrying"
                    );
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        };

        let current = server;
        server = next_server;

        if !client_sid_matches_current_user(&current) {
            tracing::warn!("client SID does not match current user, closing connection");
            continue;
        }

        let ctx = Arc::clone(&ctx);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(
                current,
                &ctx.0,
                ctx.1.as_ref(),
                ctx.2.as_ref(),
                ctx.3.as_ref(),
                &ctx.4,
            )
            .await
            {
                tracing::warn!("connection error: {e}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;
    use tokio::net::windows::named_pipe::ClientOptions;

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn unique_pipe_path() -> PathBuf {
        let n = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!("tauri-pilot-test-{}-{n}", std::process::id());
        PathBuf::from(format!(r"\\.\pipe\{name}"))
    }

    async fn start_test_server(path: &PathBuf) -> tokio::task::JoinHandle<()> {
        let (listener, guard) = bind(path).expect("bind test pipe");
        let engine = EvalEngine::new();
        let handle = tokio::spawn(async move {
            run(listener, guard, engine, None, None, None, Recorder::new()).await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        handle
    }

    #[tokio::test]
    #[serial]
    async fn test_server_responds_ping_ok() {
        let pipe = unique_pipe_path();
        let handle = start_test_server(&pipe).await;

        let client = ClientOptions::new().open(&pipe).unwrap();
        let (reader, mut writer) = tokio::io::split(client);
        let mut reader = BufReader::new(reader);

        writer
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n")
            .await
            .unwrap();
        writer.flush().await.unwrap();

        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let resp: Response = serde_json::from_str(&line).unwrap();

        assert_eq!(resp.id, serde_json::json!(1));
        assert!(resp.error.is_none());
        assert_eq!(resp.result, Some(serde_json::json!({"status": "ok"})));

        handle.abort();
        let _ = handle.await;
    }

    #[tokio::test]
    #[serial]
    async fn test_server_handles_invalid_json() {
        let pipe = unique_pipe_path();
        let handle = start_test_server(&pipe).await;

        let client = ClientOptions::new().open(&pipe).unwrap();
        let (reader, mut writer) = tokio::io::split(client);
        let mut reader = BufReader::new(reader);

        writer.write_all(b"not json\n").await.unwrap();
        writer.flush().await.unwrap();

        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let resp: Response = serde_json::from_str(&line).unwrap();

        assert_eq!(resp.id, serde_json::Value::Null);
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32700);

        handle.abort();
        let _ = handle.await;
    }

    #[tokio::test]
    #[serial]
    async fn test_server_handles_multiple_requests() {
        let pipe = unique_pipe_path();
        let handle = start_test_server(&pipe).await;

        let client = ClientOptions::new().open(&pipe).unwrap();
        let (reader, mut writer) = tokio::io::split(client);
        let mut reader = BufReader::new(reader);

        for i in 1..=3 {
            let req = format!("{{\"jsonrpc\":\"2.0\",\"id\":{i},\"method\":\"test\"}}\n");
            writer.write_all(req.as_bytes()).await.unwrap();
            writer.flush().await.unwrap();

            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let resp: Response = serde_json::from_str(&line).unwrap();
            assert_eq!(resp.id, serde_json::json!(i));
        }

        handle.abort();
        let _ = handle.await;
    }

    #[tokio::test]
    #[serial]
    #[cfg(windows)]
    async fn test_bound_pipe_carries_user_only_dacl() {
        use windows::Win32::Foundation::LocalFree;
        use windows::Win32::Security::Authorization::{GetSecurityInfo, SE_KERNEL_OBJECT};
        use windows::Win32::Security::{
            ACL_SIZE_INFORMATION, AclSizeInformation, DACL_SECURITY_INFORMATION, GetAclInformation,
        };

        let pipe = unique_pipe_path();
        let (server, guard) = bind(&pipe).expect("bind test pipe");

        let raw_handle = server.as_raw_handle();
        let handle = HANDLE(raw_handle as isize);

        // Retrieve the DACL from the freshly-bound pipe and assert:
        //   - the DACL pointer is non-NULL (the pipe is not running with a NULL DACL),
        //   - the DACL contains exactly one ACE (our owner-only ACE).
        let mut dacl_ptr: *mut ACL = std::ptr::null_mut();
        let mut sd_ptr = PSECURITY_DESCRIPTOR::default();
        unsafe {
            GetSecurityInfo(
                handle,
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&raw mut dacl_ptr),
                None,
                Some(&raw mut sd_ptr),
            )
        }
        .expect("GetSecurityInfo must succeed on a bound pipe");
        assert!(!dacl_ptr.is_null(), "bound pipe must carry a non-NULL DACL");

        let mut info = ACL_SIZE_INFORMATION::default();
        unsafe {
            GetAclInformation(
                dacl_ptr,
                (&raw mut info).cast::<c_void>(),
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        }
        .expect("GetAclInformation must succeed");
        assert_eq!(
            info.AceCount, 1,
            "bound pipe DACL must contain exactly one ACE (owner-only)"
        );

        // SAFETY: `sd_ptr` was allocated by `GetSecurityInfo`; documented contract
        // requires the caller to release it with `LocalFree`. `dacl_ptr` points
        // into the same allocation and must not be freed separately.
        unsafe {
            let _ = LocalFree(windows::Win32::Foundation::HLOCAL(sd_ptr.0));
        }

        drop(server);
        drop(guard);
    }
}
