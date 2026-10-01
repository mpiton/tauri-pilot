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

44 methods are available: `ping`, `windows.list`, `snapshot`, `diff`, `click`, `fill`, `type`, `press`, `select`, `check`, `scroll`, `drag`, `drop`, `eval`, `screenshot`, `screenshot_native`, `text`, `html`, `value`, `attrs`, `visible`, `count`, `checked`, `wait`, `watch`, `navigate`, `url`, `title`, `state`, `ipc`, `console.getLogs`, `console.clear`, `network.getRequests`, `network.clear`, `storage.get`, `storage.set`, `storage.list`, `storage.delete`, `storage.clear`, `forms.dump`, `record.start`, `record.stop`, `record.status`, `record.add`.

## Element Reference System

The `snapshot` method assigns stable short references (`e1`, `e2`, ...) to every DOM element in the page:

- Refs are stored in a `Map` inside the JS bridge
- Refs are **reset on each new snapshot** — they are not persistent across calls
- Always take a fresh snapshot before issuing actions to avoid stale refs
- In CLI commands, refs may carry the `@` prefix (`@e1`) or be used bare (`e1`), exactly as `snapshot` prints them

```bash
# Typical workflow
tauri-pilot snapshot          # assigns e1, e2, ... to current DOM
tauri-pilot click @e3         # clicks the element with ref e3
tauri-pilot fill @e5 "hello"  # fills the input at e5
```

## Multi-Window Support

The plugin supports Tauri apps with multiple windows. When a JSON-RPC request includes a `"window"` parameter, the plugin resolves the target window by label via `AppHandle::get_webview_window(label)`. If the label is present but invalid or doesn't match any open window, the request returns a "Window '{label}' not found" error — it does **not** fall back to the default. Without the parameter, it falls back to `"main"` then the first available window.

The `windows.list` method enumerates all open windows (label, URL, title; title omitted when the window has none, e.g. Android/iOS), sorted by label for deterministic output. From the CLI, use `--window <label>` to target a specific window, or `tauri-pilot windows` to list them.

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

The bridge is injected into every page, but Tauri's ACL lets `__callback` through only from origins that hold the pilot permission: the app origin, plus any origin a capability lists in `remote.urls`. On any other origin a denied `__callback` sends the plugin no signal. To find out which origins can answer, the bridge sends a hello on each page load: `__callback` with id `0`, which no eval request uses, and its `location.href` as the result. The `EvalEngine` records the origin (scheme, host and port) of each hello.

Before it sends a script, the handler reads the URL of the target window. When that page's origin never said hello, the command fails at once without running the script, instead of waiting 10 seconds, and the error names the origins that did.

`eval` only queues the script, so a navigate or redirect can replace the page after that check. The wrapper compares `location`'s protocol, hostname and port (or the hostless href) to the URL the command was aimed at, not `location.origin`, which is `"null"` for `file:` and custom schemes. A mismatch returns without running the command. If the window URL already moved after eval, the RPC fails immediately; a later timeout is reclassified the same way. The error says the command was pinned to the checked origin and the page changed, not that the new page has no bridge.

`navigate` is not origin-pinned: it is the way back from a foreign page, including when a redirect is already in flight. Relative destinations are resolved against the URL read before eval, so they cannot pick up a page that moved. On a page with no hello, `navigate` still runs. The old page answers before the webview leaves it, so whenever the destination is a new document, `navigate` waits for a hello from it that is newer than the navigation: up to 3 seconds for an origin with no hello yet, 10 seconds for one that already said hello, and fails without one. Those limits start after up to 3 seconds for the page being left to answer, so the worst case is up to about 13 seconds. On a same-origin navigate, a hello from another bridged origin the window was redirected to also counts. A `javascript:` URL or a fragment-only change keeps the document and resolves on the page's callback. Until the first hello arrives, the plugin cannot tell origins apart and every command takes the plain path, with one exception. A `navigate` sent before the first hello, as at app startup, first waits for the page's callback. A page only runs an eval after its init script, which sends the hello at its end without awaiting it, so the hello is normally recorded by then. When the destination is a new document, `navigate` then waits for the destination's hello as above (up to 10 seconds for an origin that already said hello, up to 3 seconds for one that has not, failing without it), counting from the hellos recorded when the callback was recorded, so a destination hello that lands before the handler wakes up still counts. That path gives the callback up to 10 seconds, then the hello up to 10 seconds, so up to about 20 seconds. This baseline assumes the page's callback is recorded before the destination's hello; in practice it lands well before the destination loads, and if it does not, `navigate` fails after the timeout rather than reporting a false ok. If no hello at all, from any origin, was recorded by the callback (the hello is still in flight, since nothing orders the two requests, or it was dropped), it resolves on the callback, as before.

Each hello is recorded under its origin and under the label of the webview that sent it. The origin answers whether a page can call back; `navigate` waits for a hello from its target window, so another window on the same origin cannot end the wait early. A slow page allowed by `remote.urls` can miss the 3-second grace on its first visit; later commands succeed once its hello arrives. The plugin records the invoking webview's URL, not the hello payload.

## JS Bridge Structure

The JS bridge is compiled into the plugin binary via `include_str!("../js/bridge.js")` and injected into every WebView at boot through `js_init_script()`. It is available before any frontend framework code runs.

Key internals:

- **Snapshot**: Uses a manual recursive traversal over `node.children` to walk the DOM. A `ROLE_MAP` maps implicit HTML element roles (e.g. `<button>` → `"button"`, `<a>` → `"link"`) for elements without an explicit ARIA role. Unmapped hosts that are still interactive (`draggable="true"`, contenteditable, `onclick`, `tabindex`) get a fallback role (`textbox` or `generic`) so they receive a ref; plain layout `div`s are not added to `ROLE_MAP`.
- **Actions**: Dispatch realistic DOM event sequences — `focus → mousedown → mouseup → click` — ensuring compatibility with React, Vue, and other frameworks that rely on synthetic events.
- **`fill` / `type`**: On `<input>` and `<textarea>`, write through the element's own prototype `value` setter so React sees the change. `fill` on `<select>` uses the same value-then-label matcher as `select` and throws if no option matches; `type` rejects `<select>`. On contenteditable hosts, select the target's contents (not the whole document) and call `insertText` on the element's `ownerDocument`. If that fails, assign `textContent`, which does not update Tiptap/ProseMirror. Anything else throws. A reported `ok` means the value landed. Read contenteditable results with `text` / `assert text`, not `value`.
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
