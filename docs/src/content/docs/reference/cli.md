---
title: CLI Reference
description: Complete reference for all tauri-pilot-cli commands, options, and JSON-RPC protocol examples.
---

`tauri-pilot` is a command-line client that communicates with the `tauri-plugin-pilot` server running inside your Tauri application over a Unix socket.

## Global Options

These options can be used with any command.

| Option | Description |
|--------|-------------|
| `--socket <path>` | Explicit path to the Unix socket. Auto-detected if omitted. Env: `TAURI_PILOT_SOCKET` |
| `--window <label>` | Target a specific window by label. Env: `TAURI_PILOT_WINDOW`. Default: `main`, falls back to first available |
| `--json` | Output JSON instead of human-readable text |

### Socket Auto-Detection

When `--socket` is not specified, the CLI resolves the socket in this priority order:

1. `--socket <path>` — explicit flag (highest priority)
2. `$TAURI_PILOT_SOCKET` — environment variable
3. Glob `/tmp/tauri-pilot-*.sock` → most recently modified file (by mtime)

### Window Targeting

The `--window <label>` option (or `TAURI_PILOT_WINDOW` env var) selects which window all commands operate on:

```bash
tauri-pilot --window settings snapshot    # snapshot the settings window
tauri-pilot click @e3 --window main       # click in the main window
TAURI_PILOT_WINDOW=settings tauri-pilot snapshot
```

If `--window` is not specified, the CLI targets the `main` window and falls back to the first available window. If the specified window label does not exist, the command exits with an error.

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
✓ ok
```

---

### `mcp`

Start a Model Context Protocol server over stdio. MCP-compatible agents can use
this server to call tauri-pilot tools natively instead of spawning a CLI process
for each interaction.

```bash
tauri-pilot mcp
```

**Configuration:**

```json
{
  "mcpServers": {
    "tauri-pilot": {
      "command": "tauri-pilot",
      "args": ["mcp"]
    }
  }
}
```

Use global flags before `mcp` to pin the server to a socket or default window:

```json
{
  "mcpServers": {
    "tauri-pilot": {
      "command": "tauri-pilot",
      "args": ["--socket", "/tmp/tauri-pilot-myapp.sock", "--window", "main", "mcp"]
    }
  }
}
```

The MCP server exposes tools for the CLI's app-facing commands, including
`snapshot`, `diff`, `click`, `fill`, `type`, `press`, `select`, `check`, `scroll`,
`drag`, `drop`, `text`, `html`, `value`, `attrs`, `eval`, `ipc`, `screenshot`,
`navigate`, `url`, `title`, `wait`, `watch`, `logs`, `network`, `storage_*`,
`forms`, `assert_*`, `record_*`, and `replay`.

The server starts even if no Tauri app is currently running. Each tool call
resolves and connects to the tauri-pilot Unix socket lazily, using `--socket`,
`TAURI_PILOT_SOCKET`, or the normal socket auto-detection rules.

---

### `windows`

List all open windows with their label, URL, and title.

```bash
tauri-pilot windows
```

**Example:**

```bash
$ tauri-pilot windows
main      http://localhost:1420/dashboard  PR Dashboard
settings  http://localhost:1420/settings   Settings
about     http://localhost:1420/about      About
```

**JSON-RPC example:**

```json
// Request
{"jsonrpc":"2.0","id":1,"method":"windows.list","params":{}}

// Response
{"jsonrpc":"2.0","id":1,"result":{"windows":[
  {"label":"main","url":"http://localhost:1420/dashboard","title":"PR Dashboard"},
  {"label":"settings","url":"http://localhost:1420/settings","title":"Settings"},
  {"label":"about","url":"http://localhost:1420/about","title":"About"}
]}}
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
| `--save <file>` | Save the snapshot to a JSON file for later comparison with `diff --ref` |

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

### `diff`

Compare the current page state with a previous snapshot and show only the differences. Massive token savings for AI agents that currently re-read the entire tree after each interaction.

```bash
tauri-pilot diff [OPTIONS]
```

**Options:**

| Option | Description |
|--------|-------------|
| `--ref <file>` | Diff against a saved snapshot file instead of the last in-memory snapshot |
| `-i`, `--interactive` | Only include interactive elements in the new snapshot |
| `-s`, `--selector <sel>` | Scope the new snapshot to a CSS selector |
| `-d`, `--depth <n>` | Maximum tree depth to traverse |

**Output format:**

```bash
+ button "Submit" [ref=e8]              # added
- button "Loading..." [ref=e3]          # removed
~ textbox "Search PRs" [ref=e2] value: "" → "workspace"  # changed
```

**Example:**

```bash
# Take a snapshot, interact, then diff
$ tauri-pilot snapshot -i
$ tauri-pilot fill @e2 "workspace"
$ tauri-pilot click @e3
$ tauri-pilot diff -i
~ textbox "Search PRs" [ref=e2] value: "" → "workspace"

# Save and diff against a file
$ tauri-pilot snapshot -i --save before.snap
$ tauri-pilot fill @e2 "workspace"
$ tauri-pilot diff -i --ref before.snap

# No changes
$ tauri-pilot diff -i
No changes detected.
```

**How matching works:**

Elements are matched between snapshots by `(role, name, depth)` — not by ref ID, since refs reset on every snapshot. For duplicate elements sharing the same identity, position order is used as a tiebreaker.

**JSON-RPC example:**

```json
// Request (diff vs last snapshot)
{"jsonrpc":"2.0","id":1,"method":"diff","params":{"interactive":true}}

// Request (diff vs saved reference)
{"jsonrpc":"2.0","id":1,"method":"diff","params":{"interactive":true,"reference":{"elements":[...]}}}

// Response
{"jsonrpc":"2.0","id":1,"result":{
  "added": [{"ref":"e8","role":"button","name":"Submit","depth":1}],
  "removed": [{"ref":"e3","role":"button","name":"Loading...","depth":1}],
  "changed": [{"old":{"ref":"e2","role":"textbox","name":"Search PRs","value":"","depth":1},
               "new":{"ref":"e2","role":"textbox","name":"Search PRs","value":"workspace","depth":1},
               "changes":["value"]}]
}}
```

---

### `assert`

One-step verification of element state, text, or URL. Returns exit code 0 with `ok` on success, exit code 1 with a clear error message on failure. Designed to reduce AI agent round-trips and token usage.

```bash
tauri-pilot assert <subcommand> [args...]
```

**Subcommands:**

| Subcommand | Arguments | Description |
|------------|-----------|-------------|
| `text` | `<target> <expected>` | Assert exact text content match |
| `visible` | `<target>` | Assert element is visible |
| `hidden` | `<target>` | Assert element is hidden |
| `value` | `<target> <expected>` | Assert input/textarea/select value |
| `count` | `<selector> <expected>` | Assert number of elements matching CSS selector |
| `checked` | `<target>` | Assert checkbox/radio is checked |
| `contains` | `<target> <expected>` | Assert text contains substring |
| `url` | `<expected>` | Assert current URL contains substring |

**Examples:**

```bash
# Take a snapshot first (refs reset each time)
$ tauri-pilot snapshot -i

# Exact text match
$ tauri-pilot assert text @e1 "Dashboard"
✓ ok

# Element visibility
$ tauri-pilot assert visible @e3
✓ ok

# Check input value
$ tauri-pilot assert value @e2 "workspace"
FAIL: expected value "workspace", got ""

# Count elements by CSS selector
$ tauri-pilot assert count ".list-item" 5
✓ ok

# Checkbox state
$ tauri-pilot assert checked @e4
FAIL: element is not checked

# Partial text match
$ tauri-pilot assert contains @e1 "Dash"
✓ ok

# URL check
$ tauri-pilot assert url "/dashboard"
✓ ok
```

> **Note:** Element refs (`@e1`, `@e2`, …) require a prior `snapshot` call. Always take a fresh snapshot before using refs in assertions.

**Exit codes:**

| Code | Meaning |
|------|---------|
| `0` | Assertion passed |
| `1` | Assertion failed — error message on stderr |

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

### `drag`

Drag an element to another element or by a pixel offset. Dispatches the full HTML5 drag event sequence: `mousedown` → `dragstart` → `dragleave` → `dragenter` → `dragover` → `drop` → `dragend`.

```bash
tauri-pilot drag <source> [target] [OPTIONS]
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `<source>` | Element to drag (ref, selector, or coordinates) |
| `[target]` | Element to drop onto (mutually exclusive with `--offset`) |

**Options:**

| Option | Description |
|--------|-------------|
| `--offset <X,Y>` | Drag by pixel offset instead of to an element (mutually exclusive with `[target]`) |

**Examples:**

```bash
# Drag a card to a column (kanban board)
tauri-pilot drag "#card-1" "#col-done"

# Drag by ref (after snapshot)
tauri-pilot drag @e5 @e8

# Drag a slider thumb by pixel offset
tauri-pilot drag "#slider-thumb" --offset "150,0"

# Drag to coordinates
tauri-pilot drag @e5 "400,200"
```

**JSON-RPC example:**

```json
// Element-to-element drag
{"jsonrpc":"2.0","id":1,"method":"drag","params":{"source":{"ref":"e5"},"target":{"ref":"e8"}}}

// Offset drag
{"jsonrpc":"2.0","id":1,"method":"drag","params":{"source":{"selector":"#thumb"},"offset":{"x":150,"y":0}}}
```

---

### `drop`

Simulate a file drop on an element. Reads files from disk, base64-encodes them, and creates `DataTransfer` + `File` objects in the WebView. Useful for testing file upload zones, import features, and drag-from-OS scenarios.

```bash
tauri-pilot drop <target> --file <path> [--file <path>...]
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `<target>` | Element to drop files onto (ref, selector, or coordinates) |

**Options:**

| Option | Description |
|--------|-------------|
| `--file <path>` | File to drop (required, can be repeated for multiple files) |

**Limits:** 50 MB per file, 100 MB total payload.

**Examples:**

```bash
# Drop a single file
tauri-pilot drop "#file-zone" --file ./photo.png

# Drop multiple files
tauri-pilot drop @e3 --file ./doc.pdf --file ./data.csv
```

**JSON-RPC example:**

```json
{"jsonrpc":"2.0","id":1,"method":"drop","params":{
  "selector":"#file-zone",
  "files":[{"name":"photo.png","type":"image/png","data":"iVBORw0KGgo..."}]
}}
```

---

### `watch`

Watch for DOM mutations using `MutationObserver`. Blocks until changes are detected (or timeout), then returns a summary of what changed. Useful for waiting on async UI updates without polling snapshots.

```bash
tauri-pilot watch [OPTIONS]
```

**Options:**

| Option | Description |
|--------|-------------|
| `--selector <sel>` | Scope observation to a subtree matching this CSS selector |
| `--timeout <ms>` | Maximum wait time in milliseconds (default: 10000) |
| `--stable <ms>` | Wait until DOM is stable (no new mutations) for N ms (default: 300) |

**Examples:**

```bash
# Wait for any DOM change
tauri-pilot watch

# Watch a specific subtree
tauri-pilot watch --selector "#results-list"

# Short timeout
tauri-pilot watch --timeout 3000

# Wait for DOM to settle after animations
tauri-pilot watch --stable 500
```

**JSON-RPC example:**

```json
{"jsonrpc":"2.0","id":1,"method":"watch","params":{"selector":"#results","timeout":5000,"stable":300}}
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

Capture the current WebView as a PNG using the injected `html-to-image` bridge.

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

### `url`

Get the current page URL.

```bash
tauri-pilot url
```

**Example:**

```bash
$ tauri-pilot url
http://localhost:1420/dashboard
```

---

### `title`

Get the current page title.

```bash
tauri-pilot title
```

**Example:**

```bash
$ tauri-pilot title
PR Dashboard
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

### `logs`

Display or stream captured console logs (`console.log`, `console.warn`, `console.error`, `console.info`).

The JS bridge monkey-patches the browser console methods and stores entries in a 500-entry ring buffer with timestamp, level, serialized arguments, and source location.

```bash
tauri-pilot logs [OPTIONS]
```

**Options:**

| Option | Description |
|--------|-------------|
| `--level <level>` | Filter by log level: `log`, `info`, `warn`, `error` |
| `--last <n>` | Show only the last N entries |
| `-f`, `--follow` | Continuously poll for new logs (500ms interval) |
| `--clear` | Flush the ring buffer |

**Examples:**

```bash
# Show all captured logs
$ tauri-pilot logs
[14:32:01.123] log App initialized
[14:32:01.456] warn Deprecated API call
[14:32:02.789] ✗ error Failed to fetch: NetworkError

# Filter by level
$ tauri-pilot logs --level error
[14:32:02.789] ✗ error Failed to fetch: NetworkError

# Last 5 entries
$ tauri-pilot logs --last 5

# Stream logs in real-time
$ tauri-pilot logs --follow

# Stream errors as NDJSON (one JSON object per line, compatible with jq)
$ tauri-pilot logs --follow --level error --json

# Clear the buffer
$ tauri-pilot logs --clear
✓ cleared
```

**JSON output format:**

```json
[
  {
    "id": 1,
    "timestamp": 1712073600000,
    "level": "error",
    "args": ["Failed to fetch:", "NetworkError: 500"],
    "source": "app.js:42"
  }
]
```

**JSON-RPC examples:**

```json
// Get logs filtered by level
{"jsonrpc":"2.0","id":1,"method":"console.getLogs","params":{"level":"error","last":10}}

// Clear buffer
{"jsonrpc":"2.0","id":2,"method":"console.clear"}
```

---

### `network`

Display or stream captured network requests (`fetch` and `XMLHttpRequest`).

The JS bridge monkey-patches `fetch` and `XMLHttpRequest` and stores entries in a 200-entry ring buffer with timestamp, method, URL, status code, duration, and error details.

```bash
tauri-pilot network [OPTIONS]
```

**Options:**

| Option | Description |
|--------|-------------|
| `--filter <pattern>` | Filter by URL substring match |
| `--failed` | Show only failed requests (4xx/5xx and network errors) |
| `--last <n>` | Show only the last N entries |
| `-f`, `--follow` | Continuously poll for new requests (500ms interval) |
| `--clear` | Flush the ring buffer |

**Examples:**

```bash
# Show all captured requests
$ tauri-pilot network
[14:32:01.123] GET  https://api.github.com/repos  200  125ms
[14:32:02.456] POST https://api.github.com/graphql  200  340ms
[14:32:03.789] GET  https://api.github.com/rate_limit  403  12ms

# Filter by URL
$ tauri-pilot network --filter graphql

# Show only failures
$ tauri-pilot network --failed

# Stream requests in real-time
$ tauri-pilot network --follow

# Stream as NDJSON (compatible with jq)
$ tauri-pilot network --follow --json

# Clear the buffer
$ tauri-pilot network --clear
✓ cleared
```

**JSON-RPC examples:**

```json
// Get requests filtered by URL
{"jsonrpc":"2.0","id":1,"method":"network.getRequests","params":{"filter":"graphql","last":10}}

// Clear buffer
{"jsonrpc":"2.0","id":2,"method":"network.clear"}
```

---

### `storage`

Read and write browser storage (`localStorage` or `sessionStorage`) from the CLI. Useful for AI agents inspecting persisted state, auth tokens, or modifying app configuration during testing.

```bash
tauri-pilot storage <subcommand> [OPTIONS]
```

**Subcommands:**

| Subcommand | Arguments | Description |
|------------|-----------|-------------|
| `get` | `<key>` | Read a single key |
| `set` | `<key> <value>` | Write a key-value pair |
| `list` | | Dump all key-value pairs |
| `clear` | | Clear all storage |

**Options:**

| Option | Description |
|--------|-------------|
| `--session` | Use `sessionStorage` instead of `localStorage` |

**Examples:**

```bash
# Read a key from localStorage
$ tauri-pilot storage get "auth_token"
eyJhbGciOiJIUzI1NiJ9...

# Write a key
$ tauri-pilot storage set "theme" "dark"
✓ ok

# List all localStorage entries
$ tauri-pilot storage list
auth_token = eyJhbGciOiJIUzI1NiJ9...
theme      = dark
locale     = en

# Clear localStorage
$ tauri-pilot storage clear
✓ cleared

# Use sessionStorage instead
$ tauri-pilot storage --session list
$ tauri-pilot storage --session get "csrf_token"

# JSON output
$ tauri-pilot storage list --json
[{"key":"auth_token","value":"eyJ..."},{"key":"theme","value":"dark"}]
```

**JSON-RPC examples:**

```json
// Get a key
{"jsonrpc":"2.0","id":1,"method":"storage.get","params":{"key":"auth_token","session":false}}

// Set a key
{"jsonrpc":"2.0","id":2,"method":"storage.set","params":{"key":"theme","value":"dark","session":false}}

// List all
{"jsonrpc":"2.0","id":3,"method":"storage.list","params":{"session":false}}

// Clear
{"jsonrpc":"2.0","id":4,"method":"storage.clear","params":{"session":false}}
```

---

### `forms`

Dump all form fields on the page in a single command. Useful for AI agents inspecting form state, pre-filled values, or verifying form structure without calling `value` on each input individually.

```bash
tauri-pilot forms [OPTIONS]
```

**Options:**

| Option | Description |
|--------|-------------|
| `--selector <css>` | Target a specific form by CSS selector |

**Notes:**

- Password fields display `[redacted]` in human-readable output (raw values are available in `--json` mode)
- Output is limited to 100 forms and 500 fields per form; a truncation warning appears if exceeded
- The `--selector` must match a `<form>` element; other elements are rejected with an error

**Examples:**

```bash
# Dump all forms on the page
$ tauri-pilot forms

# Target a specific form
$ tauri-pilot forms --selector "#login-form"

# JSON output
$ tauri-pilot forms --json
```

**JSON-RPC:**

```jsonc
// Dump all forms
{"jsonrpc":"2.0","id":1,"method":"forms.dump"}

// Dump specific form
{"jsonrpc":"2.0","id":2,"method":"forms.dump","params":{"selector":"#login-form"}}
```

---

### `record`

Record user interactions for later replay.

#### `record start`

Start recording interactions. All subsequent actions (click, fill, type, etc.) will be captured.

```bash
tauri-pilot record start
```

#### `record stop`

Stop recording and save captured interactions to a JSON file.

```bash
tauri-pilot record stop --output test.json
```

| Option | Description |
|--------|-------------|
| `--output`, `-o` | Output file path (JSON format, required) |

#### `record status`

Check if recording is currently active.

```bash
tauri-pilot record status
```

---

### `replay`

Replay a previously recorded session.

```bash
tauri-pilot replay test.json
```

Export as a shell script instead of replaying:

```bash
tauri-pilot replay test.json --export sh
```

| Option | Description |
|--------|-------------|
| `--export` | Export format instead of replaying (supported: `sh`) |

#### Output format

Recordings are stored as JSON arrays:

```json
[
  {"action": "click", "ref": "e3", "timestamp": 0},
  {"action": "fill", "ref": "e2", "value": "test", "timestamp": 1200}
]
```

#### JSON-RPC examples

```json
// Start recording
{"jsonrpc":"2.0","id":1,"method":"record.start","params":{}}

// Stop recording
{"jsonrpc":"2.0","id":2,"method":"record.stop","params":{}}

// Check recording status
{"jsonrpc":"2.0","id":3,"method":"record.status","params":{}}

// Add an explicit entry (e.g., assertion from CLI)
{"jsonrpc":"2.0","id":4,"method":"record.add","params":{"action":"click","ref":"e3","timestamp":0}}
```

:::note
`replay` sends recorded actions over the socket for execution. `--export sh` is fully local — it generates a shell script without connecting to the plugin.
:::

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
