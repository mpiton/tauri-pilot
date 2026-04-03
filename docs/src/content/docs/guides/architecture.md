---
title: Architecture
description: How tauri-pilot works internally — Unix socket, JSON-RPC protocol, JS bridge, and the eval+callback pattern.
---

This page explains how the different components of tauri-pilot fit together and the design decisions behind them.

## Overview

```
┌──────────────┐   Unix Socket    ┌─────────────────────────────┐
│  tauri-pilot  │ ◄──────────────► │  tauri-plugin-pilot (Rust)  │
│  (CLI)        │   JSON-RPC       │  embedded in your app       │
└──────────────┘                   │                             │
                                   │  ┌─────────────────────┐   │
                                   │  │  JS Bridge (injected)│   │
                                   │  │  window.__PILOT__    │   │
                                   │  └─────────────────────┘   │
                                   │  WebView (WebKitGTK)        │
                                   └─────────────────────────────┘
```

Three components:

1. **CLI** (`tauri-pilot`): A standalone Rust binary that connects to the socket, serializes commands as JSON-RPC, and formats the output for the terminal or for machine consumption.
2. **Plugin** (`tauri-plugin-pilot`): Embedded in your Tauri app (debug builds only). Starts a Unix socket server at boot, accepts connections, and routes incoming requests to the appropriate handler.
3. **JS Bridge**: Vanilla JS injected into the WebView via `js_init_script()` at startup. Exposes `window.__PILOT__` with snapshot, action, and read methods that the plugin calls through WebView eval.

## Unix Socket Protocol

Communication happens over a Unix socket at `/tmp/tauri-pilot-{identifier}.sock`.

Messages are **newline-delimited JSON-RPC 2.0** — each message ends with `\n`. This framing makes the protocol compatible with `socat` and `nc` for manual debugging:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"ping"}' | socat - UNIX-CONNECT:/tmp/tauri-pilot-myapp.sock
```

## JSON-RPC Message Format

Three message types:

```json
// Request
{"jsonrpc":"2.0","id":1,"method":"ping"}

// With params
{"jsonrpc":"2.0","id":2,"method":"click","params":{"ref":"e3"}}

// Response (success)
{"jsonrpc":"2.0","id":1,"result":{"status":"ok"}}

// Response (error)
{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"Method not found"}}
```

The protocol is implemented with three hand-rolled serde structs (~50 lines total) — no external JSON-RPC crate is used.

| Struct | Fields |
|--------|--------|
| `Request` | `jsonrpc`, `id`, `method`, `params?` |
| `Response` | `jsonrpc`, `id`, `result?`, `error?` |
| `RpcError` | `code`, `message`, `data?` |

23 methods are available: `ping`, `snapshot`, `click`, `fill`, `type`, `press`, `select`, `check`, `scroll`, `eval`, `screenshot`, `text`, `html`, `value`, `attrs`, `wait`, `navigate`, `url`, `title`, `state`, `ipc`, `console.getLogs`, `console.clear`.

## Element Reference System

The `snapshot` method assigns stable short references (`e1`, `e2`, ...) to every DOM element in the page:

- Refs are stored in a `Map` inside the JS bridge
- Refs are **reset on each new snapshot** — they are not persistent across calls
- Always take a fresh snapshot before issuing actions to avoid stale refs
- In CLI commands, refs use the `@` prefix: `@e1`, `@e2`, etc.

```bash
# Typical workflow
tauri-pilot snapshot          # assigns e1, e2, ... to current DOM
tauri-pilot click @e3         # clicks the element with ref e3
tauri-pilot fill @e5 "hello"  # fills the input at e5
```

## Eval + Callback Pattern (ADR-001)

`webview.eval()` in Tauri v2 is **fire-and-forget** — it dispatches JS into the WebView but provides no return value. All methods that read from the page require a response, so a callback pattern is used:

1. The plugin wraps the target JS in a `try/catch` block
2. On completion, the JS invokes `window.__TAURI_INTERNALS__.invoke('plugin:pilot|__callback', {id, result})`
3. The plugin's IPC handler for `__callback` looks up the matching `oneshot::Sender` and resolves it
4. Rust awaits the oneshot channel with a 10-second timeout

The `EvalEngine` maintains:
- A `HashMap<u64, oneshot::Sender<Result<Value, String>>>` for in-flight requests
- An `AtomicU64` counter for request IDs

This makes every eval effectively async and type-safe from the Rust side.

## JS Bridge Structure

The JS bridge is compiled into the plugin binary via `include_str!("../js/bridge.js")` and injected into every WebView at boot through `js_init_script()`. It is available before any frontend framework code runs.

Key internals:

- **Snapshot**: Uses a manual recursive traversal over `node.children` to walk the DOM. A `ROLE_MAP` maps implicit HTML element roles (e.g. `<button>` → `"button"`, `<a>` → `"link"`) for elements without an explicit ARIA role.
- **Actions**: Dispatch realistic DOM event sequences — `focus → mousedown → mouseup → click` — ensuring compatibility with React, Vue, and other frameworks that rely on synthetic events.
- **`fill`**: Uses the native `HTMLInputElement` value setter via `Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set` to trigger React's synthetic change events correctly.
- **Console capture**: Monkey-patches `console.log/warn/error/info`, stores entries in a 500-entry ring buffer with `id`, `timestamp`, `level`, `args`, and `source`. Exposed via `consoleLogs(options)` and `clearLogs()`.

## Project Structure

```
tauri-pilot/
├── Cargo.toml                     # workspace
├── crates/
│   ├── tauri-plugin-pilot/
│   │   ├── src/
│   │   │   ├── lib.rs             # Plugin init, js_init_script, setup
│   │   │   ├── server.rs          # Unix socket server, accept loop
│   │   │   ├── protocol.rs        # Request, Response, RpcError
│   │   │   ├── handler.rs         # Dispatch method → handler
│   │   │   ├── eval.rs            # EvalEngine (callback pattern)
│   │   │   └── error.rs           # thiserror types
│   │   └── js/
│   │       └── bridge.js          # JS bridge (included via include_str!)
│   └── tauri-pilot-cli/
│       └── src/
│           ├── main.rs            # Entry point, tokio::main
│           ├── cli.rs             # Clap definitions
│           ├── client.rs          # Unix socket client
│           ├── protocol.rs        # Request, Response
│           ├── output.rs          # Formatters text/JSON
│           └── error.rs           # anyhow wrappers
```
