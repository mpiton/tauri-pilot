---
title: CLI Reference
description: Complete reference for all tauri-pilot-cli commands, options, and JSON-RPC protocol examples.
---

# CLI Reference

`tauri-pilot-cli` is a command-line client that communicates with the `tauri-plugin-pilot` server running inside your Tauri application over a Unix socket.

## Global Options

These options can be used with any command.

| Option | Description |
|--------|-------------|
| `--socket <path>` | Explicit path to the Unix socket. Auto-detected if omitted. Env: `TAURI_PILOT_SOCKET` |
| `--json` | Output JSON instead of human-readable text |

### Socket Auto-Detection

When `--socket` is not specified, the CLI resolves the socket in this priority order:

1. `--socket <path>` — explicit flag (highest priority)
2. `$TAURI_PILOT_SOCKET` — environment variable
3. Glob `/tmp/tauri-pilot-*.sock` → most recently modified file (by mtime)

## Target Syntax

Many commands accept a `<target>` argument that identifies a DOM element. Three formats are supported:

| Format | Example | Description |
|--------|---------|-------------|
| Element ref | `@e1` | Reference from the last `snapshot` call |
| CSS selector | `#submit-btn` or `.class` | Standard CSS selector |
| Coordinates | `100,200` | Raw x,y screen coordinates |

> **Note:** Element refs (`@e1`, `@e2`, …) are reset on every `snapshot` call. Always take a fresh snapshot before using refs.

---

## Commands

### `ping`

Health check. Verifies the plugin server is reachable and responding.

```bash
tauri-pilot ping
```

**Example:**

```bash
$ tauri-pilot ping
pong
```

---

### `snapshot`

Capture the current accessibility tree of the WebView and assign stable element refs (`e1`, `e2`, …).

```bash
tauri-pilot snapshot [OPTIONS]
```

**Options:**

| Option | Description |
|--------|-------------|
| `-i`, `--interactive` | Only include interactive elements (buttons, inputs, links, etc.) |
| `-s`, `--selector <sel>` | Scope the snapshot to the subtree matching this CSS selector |
| `-d`, `--depth <n>` | Maximum tree depth to traverse |

**Example:**

```bash
$ tauri-pilot snapshot --interactive
e1  heading   "PR Dashboard"
e2  textbox   "Search PRs"       value=""
e3  button    "Refresh"
```

**JSON-RPC example:**

```json
// Request
{"jsonrpc":"2.0","id":1,"method":"snapshot","params":{"interactive":true}}

// Response
{"jsonrpc":"2.0","id":1,"result":{"elements":[
  {"ref":"e1","role":"heading","name":"PR Dashboard","depth":0},
  {"ref":"e2","role":"textbox","name":"Search PRs","depth":1,"value":""},
  {"ref":"e3","role":"button","name":"Refresh","depth":1}
]}}
```

---

### `click`

Simulate a realistic click on an element (dispatches focus → mousedown → mouseup → click events).

```bash
tauri-pilot click <target>
```

**Example:**

```bash
tauri-pilot click @e3
tauri-pilot click "#submit-btn"
tauri-pilot click 100,200
```

---

### `fill`

Clear an input field and type a new value. Uses the native setter to trigger synthetic React events.

```bash
tauri-pilot fill <target> <value>
```

**Example:**

```bash
tauri-pilot fill @e2 "my-feature-branch"
tauri-pilot fill "#search" "open issues"
```

---

### `type`

Type text into a focused element without clearing existing content first.

```bash
tauri-pilot type <target> <text>
```

**Example:**

```bash
tauri-pilot type @e2 " additional text"
```

---

### `press`

Send a keyboard event to the focused element.

```bash
tauri-pilot press <key>
```

Common keys: `Enter`, `Tab`, `Escape`, `ArrowDown`, `ArrowUp`, `Backspace`, `Space`.

**Example:**

```bash
tauri-pilot press Enter
tauri-pilot press Tab
tauri-pilot press Escape
```

---

### `select`

Select an option in a `<select>` dropdown by value.

```bash
tauri-pilot select <target> <value>
```

**Example:**

```bash
tauri-pilot select "#status-filter" "open"
tauri-pilot select @e5 "closed"
```

---

### `check`

Toggle a checkbox (check if unchecked, uncheck if checked).

```bash
tauri-pilot check <target>
```

**Example:**

```bash
tauri-pilot check "#remember-me"
tauri-pilot check @e7
```

---

### `scroll`

Scroll the page or a specific element.

```bash
tauri-pilot scroll <direction> [amount] [OPTIONS]
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `<direction>` | `up`, `down`, `left`, or `right` |
| `[amount]` | Scroll distance in pixels (default: 300) |

**Options:**

| Option | Description |
|--------|-------------|
| `--ref <ref>` | Element to scroll (defaults to the page) |

**Example:**

```bash
tauri-pilot scroll down 500
tauri-pilot scroll up --ref @e4
```

---

### `text`

Get the text content of an element.

```bash
tauri-pilot text <target>
```

**Example:**

```bash
$ tauri-pilot text @e1
PR Dashboard
```

---

### `html`

Get the innerHTML of an element, or the full document HTML if no target is given.

```bash
tauri-pilot html [target]
```

**Example:**

```bash
tauri-pilot html @e1
tauri-pilot html "#main-content"
tauri-pilot html
```

---

### `value`

Get the current value of an input, textarea, or select element.

```bash
tauri-pilot value <target>
```

**Example:**

```bash
$ tauri-pilot value "#search"
my-feature-branch
```

---

### `attrs`

Get all HTML attributes of an element as key-value pairs.

```bash
tauri-pilot attrs <target>
```

**Example:**

```bash
$ tauri-pilot attrs @e3
id        = submit-btn
class     = btn btn-primary
disabled  = false
type      = button
```

---

### `eval`

Execute arbitrary JavaScript in the WebView context and return the result.

```bash
tauri-pilot eval <script>
```

**Example:**

```bash
$ tauri-pilot eval "document.title"
PR Dashboard

$ tauri-pilot eval "window.location.pathname"
/dashboard
```

---

### `ipc`

Call a Tauri IPC command (registered with `tauri::Builder`) and return the response.

```bash
tauri-pilot ipc <command> [OPTIONS]
```

**Options:**

| Option | Description |
|--------|-------------|
| `--args <json>` | JSON object of arguments to pass to the command |

**Example:**

```bash
tauri-pilot ipc get_prs
tauri-pilot ipc create_pr --args '{"title":"Fix bug","branch":"fix/issue-42"}'
```

---

### `screenshot`

Capture the current WebView as a PNG image using the injected `html-to-image` bridge.

```bash
tauri-pilot screenshot [path] [OPTIONS]
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `[path]` | Output file path. Prints base64 to stdout if omitted |

**Options:**

| Option | Description |
|--------|-------------|
| `--selector <sel>` | Capture only the element matching this CSS selector |

**Example:**

```bash
tauri-pilot screenshot ./dashboard.png
tauri-pilot screenshot --selector "#main-panel" ./panel.png
tauri-pilot screenshot  # prints base64 PNG to stdout
```

---

### `navigate`

Change the WebView URL.

```bash
tauri-pilot navigate <url>
```

**Example:**

```bash
tauri-pilot navigate "http://localhost:1420/settings"
tauri-pilot navigate "/"
```

---

### `state`

Get the current page state: URL, title, viewport dimensions, and scroll position.

```bash
tauri-pilot state
```

**Example:**

```bash
$ tauri-pilot state
url       http://localhost:1420/dashboard
title     PR Dashboard
viewport  1280x800
scroll    0,0
```

---

### `wait`

Wait for an element to appear (or disappear) in the DOM.

```bash
tauri-pilot wait [target] [OPTIONS]
```

**Options:**

| Option | Description |
|--------|-------------|
| `--selector <sel>` | CSS selector to wait for (alternative to positional `[target]`) |
| `--gone` | Wait for the element to disappear instead of appear |
| `--timeout <ms>` | Maximum wait time in milliseconds (default: 10000) |

**Example:**

```bash
tauri-pilot wait "@e3"
tauri-pilot wait --selector "#loading-spinner" --gone
tauri-pilot wait --selector ".toast-success" --timeout 5000
```

---

## JSON-RPC Protocol

The CLI communicates with the plugin over a Unix socket using a hand-rolled JSON-RPC 2.0 protocol with newline-delimited framing (`\n`).

You can interact directly with the socket using `socat` or `nc` for debugging:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}' | socat - UNIX-CONNECT:/tmp/tauri-pilot-com.myapp.sock
```

**Request structure:**

```json
{"jsonrpc":"2.0","id":1,"method":"<method>","params":{...}}
```

**Success response:**

```json
{"jsonrpc":"2.0","id":1,"result":{...}}
```

**Error response:**

```json
{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"Method not found"}}
```
