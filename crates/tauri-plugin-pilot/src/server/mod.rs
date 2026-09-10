use crate::error::Error;
use crate::eval::EvalEngine;
use crate::handler;
use crate::protocol::{Request, Response};
use crate::recorder::Recorder;
use crate::webview::Webviews;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

pub(crate) async fn handle_connection<S>(
    stream: S,
    engine: &EvalEngine,
    webviews: &dyn Webviews,
    recorder: &Recorder,
) -> Result<(), Error>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    const MAX_LINE_LENGTH: usize = 1_048_576;

    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(&mut reader);
    let mut line = String::new();

    loop {
        line.clear();
        // Bound the per-line read so a peer flooding bytes without a newline
        // can't OOM us. `Take<R>` re-implements `AsyncBufRead`, so `read_line`
        // still works through it; constructing a fresh `Take` per iteration
        // resets the remaining-byte budget for each line.
        let n = (&mut reader)
            .take(MAX_LINE_LENGTH as u64 + 1)
            .read_line(&mut line)
            .await?;
        if n == 0 {
            break;
        }

        if line.len() > MAX_LINE_LENGTH {
            let response = Response::error(
                serde_json::Value::Null,
                -32700,
                "Request line exceeds maximum length",
            );
            let mut resp_bytes = serde_json::to_vec(&response)?;
            resp_bytes.push(b'\n');
            writer.write_all(&resp_bytes).await?;
            writer.flush().await?;
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Request>(trimmed) {
            Ok(req) if req.jsonrpc != "2.0" => Response::error(
                serde_json::Value::Number(req.id.into()),
                -32600,
                "Invalid JSON-RPC version (expected \"2.0\")",
            ),
            Ok(req) => dispatch_request(&req, engine, webviews, recorder).await,
            Err(e) => Response::error(serde_json::Value::Null, -32700, format!("Parse error: {e}")),
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
    webviews: &dyn Webviews,
    recorder: &Recorder,
) -> Response {
    match handler::dispatch(&req.method, req.params.as_ref(), engine, webviews, recorder).await {
        Ok(result) => Response::success(req.id, result),
        Err(rpc_err) => Response {
            jsonrpc: "2.0".to_owned(),
            id: serde_json::Value::from(req.id),
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
pub use unix::{bind, run, socket_address};
// Windows binds inside `run` (#115), so `bind` is internal to the windows module
// and is not re-exported — only `run` and `socket_path` are used by plugin setup.
#[cfg(windows)]
pub use windows::{run, socket_path};
