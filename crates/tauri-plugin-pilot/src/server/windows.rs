use super::{EvalFn, FocusFn, ListWindowsFn, handle_connection};

use crate::error::Error;
use crate::eval::EvalEngine;
#[allow(unused_imports)]
use crate::protocol::Response;
use crate::recorder::Recorder;

use std::alloc::{alloc_zeroed, Layout};
use std::ffi::c_void;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[allow(unused_imports)]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Security::{
    ACL, ACL_REVISION, AddAccessAllowedAce, EqualSid, GetLengthSid, GetTokenInformation,
    ImpersonateNamedPipeClient, InitializeAcl, InitializeSecurityDescriptor,
    PSECURITY_DESCRIPTOR, RevertToSelf, SECURITY_ATTRIBUTES,
    SetSecurityDescriptorDacl, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentThread, OpenProcessToken};

pub fn socket_path(identifier: &str) -> PathBuf {
    PathBuf::from(format!(r"\\.\pipe\tauri-pilot-{identifier}"))
}

#[derive(serde::Serialize, serde::Deserialize)]
struct InstanceEntry {
    pipe: String,
    pid: u32,
    created_at: u64,
}

fn instances_dir() -> std::io::Result<PathBuf> {
    let local_app_data =
        std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "LOCALAPPDATA environment variable is not set or empty",
            )
        })?;
    Ok(PathBuf::from(local_app_data).join("tauri-pilot").join("instances"))
}

fn instance_file_path(identifier: &str) -> std::io::Result<PathBuf> {
    let dir = instances_dir()?;
    if !dir.exists() {
        let wide_path: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let mut sa = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(mem::size_of::<SECURITY_ATTRIBUTES>())
                .expect("SECURITY_ATTRIBUTES size must fit in u32"),
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: windows::Win32::Foundation::BOOL(0),
        };
        let sd = windows::Win32::Security::SECURITY_DESCRIPTOR {};
        sa.lpSecurityDescriptor = &sd as *const _ as *mut _;
        unsafe {
            windows::Win32::FileSystem::CreateDirectoryW(
                windows::core::PCWSTR::from_raw(wide_path.as_ptr()),
                Some(&sa),
            )
        }
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::PermissionDenied, e))?;
    }
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

fn is_pid_alive(pid: u32) -> bool {
    use windows::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) };
    match handle {
        Ok(h) => {
            let alive = unsafe {
                let mut exit_code: u32 = 0;
                GetExitCodeProcess(h, &mut exit_code).is_ok() && exit_code == STILL_ACTIVE.0
            };
            unsafe { let _ = CloseHandle(h) };
            alive
        }
        Err(_) => false,
    }
}

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

/// Owns the buffers backing a [`SECURITY_ATTRIBUTES`] and closes the token handle.
struct SecurityAttributesGuard {
    sd: Box<MaybeUninit<windows::Win32::Security::SECURITY_DESCRIPTOR>>,
    acl: Box<MaybeUninit<ACL>>,
    token: HANDLE,
}

impl Drop for SecurityAttributesGuard {
    fn drop(&mut self) {
        unsafe {
            if self.token.0 != 0 {
                let _ = CloseHandle(self.token);
            }
        }
    }
}

fn get_current_user_sid(
    token: HANDLE,
) -> std::io::Result<(*mut windows::Win32::Security::SID, HANDLE)> {
    let mut return_length = 0u32;
    let _ = GetTokenInformation(token, TokenUser, None, 0, &raw mut return_length);
    let mut token_user_buf = vec![0u8; return_length as usize];
    GetTokenInformation(
        token,
        TokenUser,
        Some(token_user_buf.as_mut_ptr().cast::<c_void>()),
        return_length,
        &raw mut return_length,
    )
    .map_err(|e| std::io::Error::other(e.to_string()))?;
    let token_user = &*token_user_buf.as_ptr().cast::<TOKEN_USER>();
    Ok((token_user.User.Sid, token))
}

fn client_sid_matches_current_user(pipe: &NamedPipeServer) -> bool {
    // Impersonate the client to get its token.
    if ImpersonateNamedPipeClient(pipe.as_raw_handle() as _).is_err() {
        tracing::warn!("failed to impersonate named pipe client");
        return true; // Allow connection if impersonation fails (DACL is primary)
    }

    // Get the impersonation token from the current thread.
    let thread = unsafe { GetCurrentThread() };
    let mut impersonation_token = HANDLE(0);
    if unsafe { OpenProcessToken(thread, TOKEN_QUERY, &raw mut impersonation_token) }.is_err() {
        tracing::warn!("failed to open thread token for impersonation");
        let _ = RevertToSelf();
        return true;
    }

    // Get the client's SID.
    let client_sid = unsafe {
        let mut return_length = 0u32;
        let _ = GetTokenInformation(impersonation_token, TokenUser, None, 0, &raw mut return_length);
        if return_length == 0 {
            let _ = CloseHandle(impersonation_token);
            let _ = RevertToSelf();
            return true;
        }
        let mut token_user_buf = vec![0u8; return_length as usize];
        if GetTokenInformation(
            impersonation_token,
            TokenUser,
            Some(token_user_buf.as_mut_ptr().cast::<c_void>()),
            return_length,
            &raw mut return_length,
        ).is_err() {
            let _ = CloseHandle(impersonation_token);
            let _ = RevertToSelf();
            return true;
        }
        let token_user = &*token_user_buf.as_ptr().cast::<TOKEN_USER>();
        token_user.User.Sid
    };

    // Revert to self before getting our own SID.
    let _ = RevertToSelf();
    let _ = CloseHandle(impersonation_token);

    // Get current process token and compare SIDs.
    let matches = unsafe {
        let process = GetCurrentProcess();
        let mut token = HANDLE(0);
        if OpenProcessToken(process, TOKEN_QUERY, &raw mut token).is_err() {
            return true;
        }
        let mut return_length = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &raw mut return_length);
        if return_length == 0 {
            let _ = CloseHandle(token);
            return true;
        }
        let mut token_user_buf = vec![0u8; return_length as usize];
        let result = GetTokenInformation(
            token,
            TokenUser,
            Some(token_user_buf.as_mut_ptr().cast::<c_void>()),
            return_length,
            &raw mut return_length,
        );
        if result.is_err() {
            let _ = CloseHandle(token);
            return true;
        }
        let token_user = &*token_user_buf.as_ptr().cast::<TOKEN_USER>();
        let our_sid = token_user.User.Sid;
        let _ = CloseHandle(token);
        EqualSid(client_sid, our_sid).as_bool()
    };

    matches
}

fn create_security_descriptor_dacl(
    user_sid: *mut windows::Win32::Security::SID,
) -> std::io::Result<(
    Box<MaybeUninit<windows::Win32::Security::SECURITY_DESCRIPTOR>>,
    Box<MaybeUninit<ACL>>,
)> {
    // Allocate ACL with proper alignment for ACL structure.
    // SAFETY: Layout::new::<ACL>() guarantees alignment required by ACL.
    // alloc_zeroed initializes memory to zero, which is valid for ACL.
    let acl_layout = Layout::new::<ACL>();
    let sid_length = GetLengthSid(user_sid) as usize;
    let acl_size = (8 + 4 + 4 + sid_length + 3) & !3; // DWORD-aligned
    let acl_ptr = unsafe { alloc_zeroed(acl_layout) as *mut ACL };

    // Initialize ACL or abort if allocation failed.
    if acl_ptr.is_null() {
        std::alloc::dealloc(acl_ptr as *mut u8, acl_layout);
        return Err(std::io::Error::other("failed to allocate ACL"));
    }

    // SAFETY: acl_ptr is valid, non-null, and properly aligned for ACL.
    // The size calculation ensures DWORD alignment (4-byte boundary).
    unsafe {
        InitializeAcl(acl_ptr, acl_size as u32, ACL_REVISION)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        AddAccessAllowedAce(acl_ptr, ACL_REVISION, (GENERIC_READ | GENERIC_WRITE).0, user_sid)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
    }

    // Allocate security descriptor with proper alignment.
    // SAFETY: Box::new(MaybeUninit::uninit()) provides properly aligned,
    // uninitialized memory for SECURITY_DESCRIPTOR.
    let sd_box = Box::new(MaybeUninit::<windows::Win32::Security::SECURITY_DESCRIPTOR>::uninit());
    let sd_ptr = PSECURITY_DESCRIPTOR(sd_box.as_ptr() as *mut c_void);

    // SAFETY: sd_ptr points to properly aligned, valid memory for SECURITY_DESCRIPTOR.
    unsafe {
        InitializeSecurityDescriptor(sd_ptr, 1)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        SetSecurityDescriptorDacl(sd_ptr, true, Some(acl_ptr), false)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
    }

    // Wrap ACL pointer in Box for RAII cleanup.
    let acl_box = unsafe { Box::from_raw(acl_ptr) };
    Ok((sd_box, acl_box))
}

fn create_user_only_security_attributes()
-> std::io::Result<(SECURITY_ATTRIBUTES, SecurityAttributesGuard)> {
    unsafe {
        let process = GetCurrentProcess();
        let mut token = HANDLE(0);
        OpenProcessToken(process, TOKEN_QUERY, &raw mut token)
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        // Guard created immediately so CloseHandle is called on any early return.
        let mut guard = SecurityAttributesGuard {
            sd: Box::new(MaybeUninit::uninit()),
            acl: Box::new(MaybeUninit::uninit()),
            token,
        };

        let (user_sid, _token) = get_current_user_sid(guard.token)?;
        let (sd, acl) = create_security_descriptor_dacl(user_sid)?;
        guard.sd = sd;
        guard.acl = acl;

        // SAFETY: sd box is valid, uninitialized memory that InitializeSecurityDescriptor
        // will write to. PSECURITY_DESCRIPTOR is *mut SECURITY_DESCRIPTOR.
        let sd_ptr = PSECURITY_DESCRIPTOR(guard.sd.as_ptr() as *mut c_void);

        let sa = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(mem::size_of::<SECURITY_ATTRIBUTES>())
                .expect("SECURITY_ATTRIBUTES size must fit in u32"),
            lpSecurityDescriptor: sd_ptr.0,
            bInheritHandle: windows::Win32::Foundation::BOOL(0),
        };

        Ok((sa, guard))
    }
}

// ---------------------------------------------------------------------------

pub fn bind(pipe_path: &Path) -> Result<(NamedPipeServer, RegistryGuard), Error> {
    let server = match create_user_only_security_attributes() {
        Ok((mut sa, _sec_guard)) => {
            // SAFETY: `sa` and its backing buffers (owned by `_sec_guard`) are valid for
            // the duration of this call.  The kernel copies the security descriptor, so
            // `_sec_guard` may be dropped after the pipe is created.
            unsafe {
                ServerOptions::new()
                    .first_pipe_instance(true)
                    .pipe_mode(tokio::net::windows::named_pipe::PipeMode::Byte)
                    .create_with_security_attributes_raw(pipe_path, (&raw mut sa).cast::<c_void>())
            }
        }
        Err(e) => {
            tracing::warn!(
                path = %pipe_path.display(),
                error = %e,
                "failed to create secure pipe, falling back to default security"
            );
            ServerOptions::new()
                .first_pipe_instance(true)
                .pipe_mode(tokio::net::windows::named_pipe::PipeMode::Byte)
                .create(pipe_path)
        }
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
        server.connect().await?;

        let next_server = match create_user_only_security_attributes() {
            Ok((mut sa, _sec_guard)) => unsafe {
                ServerOptions::new()
                    .pipe_mode(tokio::net::windows::named_pipe::PipeMode::Byte)
                    .create_with_security_attributes_raw(&pipe_path, (&raw mut sa).cast::<c_void>())
            },
            Err(e) => {
                tracing::warn!("failed to create secure pipe: {e}, falling back to default security");
                ServerOptions::new()
                    .pipe_mode(tokio::net::windows::named_pipe::PipeMode::Byte)
                    .create(&pipe_path)
            }
        }
        .map_err(Error::from)?;

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
}
