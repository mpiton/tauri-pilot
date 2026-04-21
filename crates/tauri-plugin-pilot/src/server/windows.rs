use super::{EvalFn, FocusFn, ListWindowsFn, handle_connection};

use crate::error::Error;
use crate::eval::EvalEngine;
#[allow(unused_imports)]
use crate::protocol::Response;
use crate::recorder::Recorder;

use std::ffi::c_void;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[allow(unused_imports)]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Security::{
    ACL, ACL_REVISION, AddAccessAllowedAce, GetLengthSid, GetTokenInformation, InitializeAcl,
    InitializeSecurityDescriptor, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
    SetSecurityDescriptorDacl, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

pub fn socket_path(identifier: &str) -> PathBuf {
    PathBuf::from(format!(r"\\.\pipe\tauri-pilot-{identifier}"))
}

fn registry_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .map(|p| p.join("tauri-pilot"))
}

fn registry_path() -> Option<PathBuf> {
    registry_dir().map(|d| d.join("instances.json"))
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Registry {
    instances: std::collections::BTreeMap<String, InstanceEntry>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct InstanceEntry {
    pipe: String,
    pid: u32,
    created_at: u64,
}

fn read_registry(path: &Path) -> Registry {
    match std::fs::read_to_string(path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
        Err(_) => Registry::default(),
    }
}

fn write_registry(path: &Path, reg: &Registry) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(reg)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &data)?;
    std::fs::rename(&tmp, path)
}

fn register_instance(identifier: &str, pipe_path: &Path) -> std::io::Result<()> {
    let Some(reg_path) = registry_path() else {
        return Ok(());
    };
    let mut reg = read_registry(&reg_path);
    reg.instances.insert(
        identifier.to_string(),
        InstanceEntry {
            pipe: pipe_path.to_string_lossy().into_owned(),
            pid: std::process::id(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        },
    );
    write_registry(&reg_path, &reg)
}

fn unregister_instance(identifier: &str) {
    let Some(reg_path) = registry_path() else {
        return;
    };
    let mut reg = read_registry(&reg_path);
    reg.instances.remove(identifier);
    let _ = write_registry(&reg_path, &reg);
}

pub struct RegistryGuard {
    identifier: String,
}

impl Drop for RegistryGuard {
    fn drop(&mut self) {
        unregister_instance(&self.identifier);
        tracing::info!(identifier = %self.identifier, "registry entry removed");
    }
}

// ---------------------------------------------------------------------------
// Security: restrict the named pipe to the creating user only (DACL-only)
// ---------------------------------------------------------------------------

/// Owns the buffers backing a [`SECURITY_ATTRIBUTES`] and closes the token handle.
/// Must outlive the `SECURITY_ATTRIBUTES` pointer passed to Windows APIs.
#[allow(dead_code)]
struct SecurityAttributesGuard {
    sd: Vec<u8>,
    acl: Vec<u8>,
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

/// Build a [`SECURITY_ATTRIBUTES`] whose DACL grants `GENERIC_READ | GENERIC_WRITE`
/// **only** to the current user.  No other principals receive access.
///
/// The returned guard must stay alive for as long as the `SECURITY_ATTRIBUTES` is
/// passed to Windows APIs (the kernel copies the security descriptor, so the guard
/// can be dropped immediately after the pipe is created).
#[allow(clippy::cast_ptr_alignment)]
fn create_user_only_security_attributes()
-> std::io::Result<(SECURITY_ATTRIBUTES, SecurityAttributesGuard)> {
    unsafe {
        // 1. Open the current process token (TOKEN_QUERY only).
        let process = GetCurrentProcess();
        let mut token = HANDLE(0);
        OpenProcessToken(process, TOKEN_QUERY, &raw mut token)
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        // 2. Retrieve TokenUser information.
        //    First call: discover required buffer size.
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
        let user_sid = token_user.User.Sid;

        // 3. Allocate and initialise an ACL with a single ACE for the user.
        let sid_length = GetLengthSid(user_sid) as usize;
        // ACL header (8) + ACE header (4) + access-mask (4) + SID
        let acl_size = 8 + 4 + 4 + sid_length;
        // Align to DWORD boundary.
        let acl_size = (acl_size + 3) & !3;

        let mut acl_buf = vec![0u8; acl_size];
        let acl_ptr = acl_buf.as_mut_ptr().cast::<ACL>();

        InitializeAcl(acl_ptr, acl_size as u32, ACL_REVISION)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        AddAccessAllowedAce(
            acl_ptr,
            ACL_REVISION,
            (GENERIC_READ | GENERIC_WRITE).0 as u32,
            user_sid,
        )
        .map_err(|e| std::io::Error::other(e.to_string()))?;

        // 4. Initialise an absolute security descriptor and attach the DACL.
        let mut sd_buf = vec![0u8; 40];
        let sd_ptr = PSECURITY_DESCRIPTOR(sd_buf.as_mut_ptr() as *mut c_void);

        InitializeSecurityDescriptor(sd_ptr, 1)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        SetSecurityDescriptorDacl(sd_ptr, true, Some(acl_ptr), false)
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        // 5. Build SECURITY_ATTRIBUTES pointing into our buffers.
        let sa = SECURITY_ATTRIBUTES {
            nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd_buf.as_mut_ptr() as *mut c_void,
            bInheritHandle: windows::Win32::Foundation::BOOL(0),
        };

        let guard = SecurityAttributesGuard {
            sd: sd_buf,
            acl: acl_buf,
            token,
        };

        Ok((sa, guard))
    }
}

// ---------------------------------------------------------------------------

pub fn bind(pipe_path: &Path) -> Result<(NamedPipeServer, RegistryGuard), Error> {
    let (mut sa, _sec_guard) = create_user_only_security_attributes()?;

    // SAFETY: `sa` and its backing buffers (owned by `_sec_guard`) are valid for
    // the duration of this call.  The kernel copies the security descriptor, so
    // `_sec_guard` may be dropped after the pipe is created.
    let server = unsafe {
        ServerOptions::new()
            .first_pipe_instance(true)
            .pipe_mode(tokio::net::windows::named_pipe::PipeMode::Byte)
            .create_with_security_attributes_raw(pipe_path, &raw mut sa as *mut c_void)?
    };

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

        let next_server = ServerOptions::new()
            .pipe_mode(tokio::net::windows::named_pipe::PipeMode::Byte)
            .create(&pipe_path)?;

        let current = server;
        server = next_server;

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

        assert_eq!(resp.id, 1);
        assert!(resp.error.is_none());
        assert_eq!(resp.result, Some(serde_json::json!({"status": "ok"})));

        handle.abort();
    }

    #[tokio::test]
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

        assert_eq!(resp.id, 0);
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32700);

        handle.abort();
    }

    #[tokio::test]
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
            assert_eq!(resp.id, i);
        }

        handle.abort();
    }
}
