use crate::error::Error;
use crate::eval::EvalEngine;
use crate::handler;
use crate::protocol::{Request, Response};
use crate::recorder::Recorder;

use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// A function that evaluates JS in the webview.
/// The first argument is an optional window label (`None` means "use default window").
pub(crate) type EvalFn = Arc<dyn Fn(Option<&str>, String) -> Result<(), String> + Send + Sync>;

/// A function that lists all available webview windows and returns their metadata.
pub(crate) type ListWindowsFn = Arc<dyn Fn() -> serde_json::Value + Send + Sync>;

/// A function that requests focus for a webview window.
/// `None` means "default window" (same resolution as `EvalFn`).
/// Used before native key injection so synthesised OS events reach the right window.
pub(crate) type FocusFn = Arc<dyn Fn(Option<&str>) -> Result<(), String> + Send + Sync>;

pub(crate) async fn handle_connection<S>(
    stream: S,
    engine: &EvalEngine,
    eval_fn: Option<&EvalFn>,
    list_fn: Option<&ListWindowsFn>,
    focus_fn: Option<&FocusFn>,
    recorder: &Recorder,
) -> Result<(), Error>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(&mut reader);
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
            Ok(req) => dispatch_request(&req, engine, eval_fn, list_fn, focus_fn, recorder).await,
            Err(e) => Response::error(0, -32700, format!("Parse error: {e}")),
        };

        let mut resp_bytes = serde_json::to_vec(&response)?;
        resp_bytes.push(b'\n');
        writer.write_all(&resp_bytes).await?;
        writer.flush().await?;
    }

    Ok(())
}

pub(crate) async fn dispatch_request(
    req: &Request,
    engine: &EvalEngine,
    eval_fn: Option<&EvalFn>,
    list_fn: Option<&ListWindowsFn>,
    focus_fn: Option<&FocusFn>,
    recorder: &Recorder,
) -> Response {
    match handler::dispatch(
        &req.method,
        req.params.as_ref(),
        engine,
        eval_fn,
        list_fn,
        focus_fn,
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

#[cfg(unix)]
pub mod unix;
#[cfg(windows)]
pub mod windows;

#[cfg(unix)]
pub use unix::{SocketGuard, bind, run, socket_path};
#[cfg(windows)]
pub use windows::{bind, run, socket_path};
