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
✓ ok
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
