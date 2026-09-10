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
                                   │  WebView                    │
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

42 methods are available: `ping`, `windows.list`, `snapshot`, `diff`, `click`, `fill`, `type`, `press`, `select`, `check`, `scroll`, `drag`, `drop`, `eval`, `screenshot`, `text`, `html`, `value`, `attrs`, `visible`, `count`, `checked`, `wait`, `watch`, `navigate`, `url`, `title`, `state`, `ipc`, `console.getLogs`, `console.clear`, `network.getRequests`, `network.clear`, `storage.get`, `storage.set`, `storage.list`, `storage.clear`, `forms.dump`, `record.start`, `record.stop`, `record.status`, `record.add`.

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

## Multi-Window Support

The plugin supports Tauri apps with multiple windows. When a JSON-RPC request includes a `"window"` parameter, the plugin resolves the target window by label via `AppHandle::get_webview_window(label)`. If the label is present but invalid or doesn't match any open window, the request returns a "Window '{label}' not found" error — it does **not** fall back to the default. Without the parameter, it falls back to `"main"` then the first available window.

The `windows.list` method enumerates all open windows (label, URL, title), sorted by label for deterministic output. From the CLI, use `--window <label>` to target a specific window, or `tauri-pilot windows` to list them.

## Eval + Callback Pattern (ADR-001)

`webview.eval()` in Tauri v2 is **fire-and-forget** — it dispatches JS into the WebView but provides no return value. All methods that read from the page require a response, so a callback pattern is used:

1. The plugin wraps the target JS in a `try/catch` block that awaits the result
2. The wrapper sends `{id, result}` or `{id, error}` through the internal `__callback` IPC command
3. The IPC callback handler looks up the matching `oneshot::Sender` and resolves it
4. Rust awaits the oneshot channel with a 10-second timeout

The `EvalEngine` maintains:
- A `HashMap<u64, oneshot::Sender<Result<Value, String>>>` for in-flight requests
- An `AtomicU64` counter for request IDs

Handlers reach webviews only through the `Webviews` trait in `webview.rs`. Its Tauri implementation holds the `AppHandle` and resolves the target window on each request: the `--window` label, else `main`, else the first window by label. This allows targeting different windows across requests. Handler tests use an in-memory fake instead.

This makes every eval effectively async and type-safe from the Rust side.

### Origins without a callback

The bridge is injected into every page, but Tauri's ACL lets `__callback` through only from origins that hold the pilot permission: the app origin, plus any origin a capability lists in `remote.urls`. On any other origin the script runs and its result is dropped, and a denied call sends the plugin no signal. To find out which origins can answer, the bridge sends a hello on each page load: `__callback` with id `0`, which no eval request uses, and its `location.href` as the result. The `EvalEngine` records the origin (scheme, host and port) of each hello.

Before it sends a script, the handler reads the URL of the target window. When that page's origin never said hello, the command fails at once without running the script, instead of waiting 10 seconds, and the error names the origins that did. `navigate` still runs on such a page, since leaving is the way out, and it goes one step further. The old page answers before the webview leaves it, so for a destination origin with no hello yet, `navigate` waits up to 3 seconds for a hello from the new page and fails without one. Until the first hello arrives, the plugin cannot tell origins apart and every command takes the plain path.

Hellos are tracked per origin, not per window. A slow page allowed by `remote.urls` can miss the 3-second grace on its first visit; later commands succeed once its hello arrives. The plugin records the invoking webview's URL, not the hello payload.

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
│   │   ├── build.rs               # Tauri plugin build hook (permissions)
│   │   ├── src/
│   │   │   ├── lib.rs             # Plugin init, js_init_script, setup
│   │   │   ├── server.rs          # Unix socket server, accept loop
│   │   │   ├── protocol.rs        # Request, Response, RpcError
│   │   │   ├── handler.rs         # Dispatch method → handler
│   │   │   ├── eval.rs            # EvalEngine (callback pattern)
│   │   │   ├── webview.rs         # Webviews trait: target window, eval, focus
│   │   │   ├── diff.rs            # Snapshot diff (added/removed/changed)
│   │   │   ├── key.rs             # press command key-combo parser
│   │   │   ├── recorder.rs        # record/replay interaction capture
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
│           ├── style.rs           # owo-colors TTY-aware styling helpers
│           ├── scenario.rs        # TOML scenario runner + JUnit XML output
│           ├── mcp.rs             # Model Context Protocol stdio server
│           └── error.rs           # anyhow wrappers
```
