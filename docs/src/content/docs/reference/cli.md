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
| `--window <label>` | Target a specific window by label. Env: `TAURI_PILOT_WINDOW`. Default: `main`, falls back to the first window by label |
| `--json` | Output JSON instead of human-readable text |
| `--rpc-timeout <secs>` | Seconds to wait for the app to answer before giving up (default `35`). `wait` and `watch` add their own `--timeout` on top. Env: `TAURI_PILOT_RPC_TIMEOUT` |

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

If `--window` is not specified, the CLI targets the `main` window and falls back to the first window by label. If the specified window label does not exist, the command exits with an error that lists the labels the app does have, so a typo does not cost a `windows` call.

## Target Syntax

Many commands accept a `<target>` argument that identifies a DOM element. Three formats are supported:

| Format | Example | Description |
|--------|---------|-------------|
| Element ref | `@e1` or `e1` | Reference from the last `snapshot` call; the `@` is optional |
| CSS selector | `#submit-btn` or `.class` | Standard CSS selector |
| Coordinates | `100,200` | Raw x,y screen coordinates |

> **Note:** Element refs (`@e1`, `@e2`, …) are reset on every `snapshot` call. Always take a fresh snapshot before using refs.

> **Note:** A bare `e<digits>` is always read as a ref, never as a CSS type selector. A page that really has an unknown `<e12>` tag must write that selector another way, e.g. `:is(e12)` or `wait --selector e12`.

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
`forms`, `assert_*`, `record_*`, `replay`, and `run`.

`pilot.run` executes a declarative TOML scenario. Pass `path` to a `.toml` file
or inline `content` (not both), optionally set `fail_fast` to override the file,
and read the JSON report (`ok`, counts, `summary`, `steps`). A failed step also
carries the absolute failure screenshot as `screenshot`, or the reason it could
not be written as `screenshot_error`. Unless `screenshots_dir` says otherwise,
screenshots go to an owner-only (`0700`) `tauri-pilot-failures-<uid>` directory
under `$XDG_RUNTIME_DIR`, or under the system temp directory when
`$XDG_RUNTIME_DIR` is unset or not private; on Windows `temp_dir()` is already
per-user, so the plain `tauri-pilot-failures` name is used there. The MCP
server's working directory belongs to whichever client spawned it, so it is not
a useful default, and a shared `/tmp` path would hand every other user on the
host whatever the screenshots happen to show. A finished run
including failed steps is a successful tool result with `ok` false; only parse,
step-key, I/O, connect, and timeout failures are tool errors. Step keys are
checked against the table under `run`, and `drop` and `ipc` are not scenario
actions, so they fail there as unknown actions. The tool also returns
`INVALID_PARAMS` for an empty `[[step]]` list, `eval` steps unless
`TAURI_PILOT_MCP_ENABLE_DANGEROUS_TOOLS` is set, `javascript:` navigate URLs,
and screenshot steps that set `path`. The CLI example
`docs/examples/login-flow.toml` includes a screenshot `path` and cannot be run
over MCP as written. JUnit XML is not written; the JSON report is the output.

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
| `-i`, `--interactive` | Only include interactive elements (native controls plus draggable, contenteditable, onclick, and tabindex hosts) |
| `-s`, `--selector <sel>` | Scope the snapshot to the subtree matching this CSS selector |
| `-d`, `--depth <n>` | Maximum tree depth to traverse |
| `--save <file>` | Save the snapshot to a JSON file for later comparison with `diff --ref` |

**Note on `--save` with `--json`:** the saved file holds the unmodified RPC payload (`{"elements":[…]}`) so it can be fed straight back into `diff --ref`. The `--json` payload printed to stdout additionally embeds a `"path"` field (`{"elements":[…],"path":"<file>"}`) so callers piping into `jq` / `python -c 'json.load(sys.stdin)'` can recover the saved location without parsing stderr. The two shapes are intentionally different.

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

Clear an `<input>`, `<textarea>`, `<select>`, or contenteditable element and
set a new value. Form controls use the native value setter so React and similar
frameworks see the change. On `<select>`, the value is matched like `select`
(option value, then visible label) and fill throws if nothing matches.

Contenteditable hosts (Tiptap, ProseMirror, and the like) are filled by
selecting the target's contents and calling `insertText` so the editor
document updates. If `insertText` is unavailable, fill assigns `textContent`
instead, which does not update those editors. Read the result with `text` /
`assert text`, not `value`.

Throws if the target cannot take a value (for example a plain `<div>`). A
reported `ok` means the value was written.

```bash
tauri-pilot fill <target> <value>
```

**Example:**

```bash
tauri-pilot fill @e2 "my-feature-branch"
tauri-pilot fill "#search" "open issues"
tauri-pilot fill ".ProseMirror" "hello from the editor"
```

---

### `type`

Type text into an `<input>`, `<textarea>`, or contenteditable element without
clearing existing content first. Same target rules as `fill`, except
`<select>` is rejected — use `fill` or `select`. Contenteditable typing also
tries `insertText` first and falls back to `textContent`; read the result
with `text` / `assert text`.

```bash
tauri-pilot type <target> <text>
```

**Example:**

```bash
tauri-pilot type @e2 " additional text"
```

---

### `press`

Inject keyboard events at the OS level via [`enigo`](https://crates.io/crates/enigo).
Events are `isTrusted=true` and reach DOM listeners and Tauri accelerators on
supported desktop platforms. Android and iOS have no native `press` backend;
use `fill` or `type` for text input.

:::caution[X11 global shortcuts]
On X11, synthetic key events from `enigo`'s `XTestFakeKeyEvent` backend
frequently fail to trigger `tauri-plugin-global-shortcut` handlers. The
upstream `global-hotkey` crate uses `XGrabKey` passive grabs on its Linux/X11
backend; these match the X server's logical modifier state, and `enigo`'s
separate fake-input calls for each modifier and the main keycode can
desynchronize that state, so the grab's exact-modifier-mask match may fail.
DOM listeners and Tauri accelerators continue to receive `isTrusted=true`
events.

**Workaround**: factor the shortcut handler body into a `#[tauri::command]`
and invoke it via `tauri-pilot ipc <command>`, or have the handler emit a
Tauri event the test can re-emit. There is no way to invoke an arbitrary
closure passed to `on_shortcut(...)` directly from outside the process. See
[issue #75](https://github.com/mpiton/tauri-pilot/issues/75).
:::

```bash
tauri-pilot press <key>
```

`<key>` accepts modifier-prefixed combos in the form `Modifier+...+Key`. Only
`+` is a separator — `-` is treated as the literal minus key, so `Shift+-`
means Shift + minus. A trailing `+` is the `+` key itself (e.g. `Control++`
is Control + plus).

- **Modifiers** (case-insensitive): `Control`/`Ctrl`, `Shift`, `Alt`/`Option`, `Meta`/`Cmd`/`Super`/`Win`
- **Common keys**: `Enter`, `Tab`, `Escape`, `ArrowUp`/`ArrowDown`/`ArrowLeft`/`ArrowRight`, `Backspace`, `Delete`, `Home`, `End`, `PageUp`, `PageDown`, `Space`, `F1`–`F12`, or any single character

**Example:**

```bash
tauri-pilot press Enter
tauri-pilot press Tab
tauri-pilot press Control+1
tauri-pilot press Ctrl+Shift+P
```

Before pressing, the plugin requests focus on the target window and checks
that the window actually received it. If the window manager refuses the
request (common on X11 with focus-stealing prevention), `press` returns an
error instead of injecting into whichever application currently has focus.
The press takes effect on whatever element holds focus inside that window —
call `click` first if you need to focus a specific input.

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

Toggle an `<input type="checkbox">` (check if unchecked, uncheck if
checked). For `<input type="radio">`, select it and leave it selected —
an already-selected radio stays selected, matching a real click.
Throws on any other element.

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
| `<direction>` | `up`, `down`, `left`, `right`, `top`, or `bottom` |
| `[amount]` | Scroll distance in pixels (ignored for `top`/`bottom`, default: 300) |

**Options:**

| Option | Description |
|--------|-------------|
| `--target <target>` | Element to scroll: `@ref` (or bare `e3`), CSS selector, or `x,y`. Defaults to the page. `--ref` is an alias. |

`top` jumps to `scrollY = 0`; `bottom` jumps to the maximum scroll position (`scrollHeight - innerHeight` for the page, `scrollHeight - clientHeight` for an element). Unknown directions raise an error instead of silently no-op.

**Example:**

```bash
tauri-pilot scroll down 500
tauri-pilot scroll up --target @e4
tauri-pilot scroll down 50 --target "#log"
tauri-pilot scroll top
tauri-pilot scroll bottom --ref @e4
```

---

### `drag`

Drag an element to another element or by a pixel offset.

Two unrelated kinds of drag implementation exist, and they listen for different
things, so `drag` emits both:

- **HTML5 native drag** (`draggable="true"`) reacts to the drag event sequence:
  `dragstart` → `dragleave` → `dragenter` → `dragover` → `drop` → `dragend`.
- **JS drag libraries** (dnd-kit, sortable.js, interact.js, react-dnd's mouse
  backend) never see those events. They activate on `mousedown`, then track
  *repeated* `mousemove`/`pointermove` events — usually behind a small distance
  threshold — and commit on `mouseup`. So `drag` presses, streams interpolated
  moves from the source to the drop point, and releases. Each pointer event is
  followed by its compatibility mouse event, the order a browser produces.

The press targets the deepest node under the start point (`elementFromPoint`)
rather than the resolved element, because library listeners are commonly attached
to an inner handle or card and events only bubble upward. That node must be the
source or inside it — when an overlay, toast or backdrop covers the start point,
`drag` presses the resolved source instead of the thing on top of it. Moves are
hit-tested the same way at each step, so listeners on elements between the pressed
node and the document see them too.

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

With `--offset`, the drop point is resolved with `elementFromPoint`, which only hits elements inside the visible viewport. If the source element's center — the drag start point — is outside the viewport, the bridge scrolls the element into view (centered) before computing coordinates. The offset itself must still land inside the viewport — an offset larger than the visible area fails with a `Drop point ... is outside the viewport` error.

The gesture's shape is tunable over JSON-RPC and MCP (the CLI uses the defaults):
`steps` (default 12, clamped to 1–60) is how many move events are emitted,
`stepDelayMs` (default 16) is the pause between them, and `settleMs` (default 250)
is how long the bridge waits after the release so an async state update, request,
or re-render can land before you assert. The plugin sizes its eval timeout from
these, so a long gesture is not cut short by the default 10-second budget.

`ok` reports that the gesture was **delivered**, not that the app reacted to it —
nothing observable from outside can prove a library handled a drop. Always assert
the expected effect afterwards. The one signal the bridge can observe is
`html5DropHandled`, which is true when a handler called `preventDefault()` on the
`drop` event. The result also echoes the `from`/`to` points and `steps` used.

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

**Limits:** one request is at most 1 MiB and files are sent base64-encoded, so
the files in one drop can total a little under 768 KiB.

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

Watch for DOM mutations using `MutationObserver`. By default, it resolves once the DOM stays quiet for `--stable` ms and returns a summary of what changed — the change set can be empty on idle pages. Pass `--require-mutation` to reject on timeout when nothing mutated. Useful for waiting on async UI updates without polling snapshots.

```bash
tauri-pilot watch [OPTIONS]
```

**Options:**

| Option | Description |
|--------|-------------|
| `--selector <sel>` | Scope observation to a subtree matching this CSS selector |
| `--timeout <ms>` | Maximum wait time in milliseconds (default: 10000) |
| `--stable <ms>` | Wait until DOM is stable (no new mutations) for N ms (default: 300) |
| `--require-mutation` | Defer the stability timer until at least one mutation occurs. Rejects on timeout if nothing changed. |

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

# Wait for an async re-render after an IPC call (e.g. React state update)
tauri-pilot ipc settings_update --args '{"theme":"dark"}'
tauri-pilot watch --require-mutation --stable 300 --timeout 2000
```

**JSON-RPC example:**

```json
{"jsonrpc":"2.0","id":1,"method":"watch","params":{"selector":"#results","timeout":5000,"stable":300,"requireMutation":true}}
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

For a `<select multiple>`, every selected option is joined with `", "`
(the same display `forms` uses). A single-select is unchanged.

```bash
tauri-pilot value <target>
```

**Example:**

```bash
$ tauri-pilot value "#search"
my-feature-branch

$ tauri-pilot value 'select[name=skills]'
rust, js
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
tauri-pilot eval [script|-]
```

**Example:**

```bash
$ tauri-pilot eval "document.title"
PR Dashboard

$ tauri-pilot eval "window.location.pathname"
/dashboard
```

Use `-` to read JavaScript from stdin. This is useful for complex selectors,
quotes, Panda CSS class names, or multi-line scripts:

```bash
$ tauri-pilot eval - <<'EOF'
document.querySelector('[data-id="main"]').textContent
EOF
# (output depends on your app)
Main content

$ echo 'document.title' | tauri-pilot eval -
# (output depends on your app)
PR Dashboard
```

Prefer `<<'EOF'` with quotes around the heredoc delimiter. It disables shell
variable and command expansion, so `$` and backticks inside the script do not
need escaping.

The plugin reads at most 1 MiB (1,048,576 bytes) per request. The limit counts
the JSON-RPC request the CLI sends, where quotes, backslashes and newlines in
the script are escaped, so a script just under 1 MiB can still be too large. A
larger request fails before it is sent, with
`eval request is N bytes; the plugin accepts at most 1048576 bytes`.
To inject a library, use its minified build rather than the development one.
Otherwise split the script into calls that each parse on their own, and pass
state between them through `window`: `let`, `const` and `class` bindings do
not carry over to the next call.

Statements are supported alongside bare expressions — `const`, `let`, `var`,
function declarations and blocks all work, and the completion value of the last
expression is returned:

```bash
$ tauri-pilot eval "const els = document.querySelectorAll('button'); els.length"
7

$ tauri-pilot eval "function pick(n){return n*2;} pick(21)"
42
```

If the final statement is a `Promise`, it is awaited before the value is
serialized. Scripts that end on a declaration (`const x = 1;`) instead of an
expression return `null` — append the bare identifier (`; x`) to read the value
back.

Expressions that return `undefined` (e.g. `element.click()`, `console.log(...)`,
any void function) print nothing and exit with status `0`, so they compose
cleanly with `&&` and `set -e`:

```bash
$ tauri-pilot eval "document.querySelector('a')?.click()" && tauri-pilot state
# click fires, then state prints — exit 0
```

Top-level `await` is auto-wrapped in an async IIFE, so the natural shape works:

```bash
$ tauri-pilot eval 'await Promise.resolve("hello")'
hello

$ tauri-pilot eval 'await fetch("/api/items").then(r => r.json())'
[…]
```

For multi-statement scripts (e.g. `const` followed by a value to surface), use
an explicit `return` since the wrapper has no completion-value semantics:

```bash
$ tauri-pilot eval 'const r = await fetch("/api"); return r.status'
200
```

Without `return`, the result is `null`. If the auto-wrap can't compile the
script (rare; usually a syntax error), the error message points back here.

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

Capture the full page as a PNG using the injected `html-to-image` bridge.

```bash
tauri-pilot screenshot [path] [OPTIONS]
```

Without `--selector`, the capture covers the whole document height, not just
the visible viewport. The bridge serializes the DOM to render it, so the
current scroll position is not reflected in the image — what you get is the
entire page from the top. Use `--selector` to capture a single element
(works for elements below the fold too). For a pixel-exact capture of the
native window, use `screenshot_native` — macOS only, it returns
`PERMISSION_DENIED` on Linux, Windows, Android and iOS. A Simulator build is an
iOS build even though the host is a Mac, so use `screenshot` there.

The bridge also copies only an allowlist of painted CSS properties onto the
clone it renders (a full computed-style copy wedges WebKit for minutes on
style-variable-heavy pages). A page that paints through a property outside
that list renders without it, with no error, so if the PNG does not match
what you see in the app that is the first thing to check. Standard CSS
scrollbar properties (`scrollbar-color`, `scrollbar-width`,
`scrollbar-gutter`) are on the list. `::-webkit-scrollbar` and other
scrollbar pseudo-element rules are not: html-to-image copies computed
styles on the element, not on pseudo-elements, so a WebKit-styled
scrollbar is still missing from the PNG. On macOS, `screenshot_native`
sidesteps the allowlist entirely.

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

Bridge commands only work on origins allowed to call the plugin back: the app
origin, plus any origin listed in a capability's `remote.urls` that also
grants `pilot:default`. They pin to the origin they checked. The wrapper
compares `location` fields (not `location.origin`) and does not run the
command if the document is elsewhere. If the URL already moved after eval,
the RPC fails immediately; a later timeout is reclassified the same way.
`navigate` is not pinned. It assigns the absolute destination resolved from
the URL at check time, so a relative path cannot resolve against a page that
already left, and it still loads any URL. If the destination origin has never
said hello, it waits up to 3 seconds for one and fails without it. A slow
page allowed by `remote.urls` can miss that window on its first visit; later
commands succeed once the hello arrives. On an origin that cannot call back,
later commands fail at once with an error that names the page and the origins
that work. Run `tauri-pilot navigate` with an app URL to get the session
back.

---

### `url`

Get the current page URL.

```bash
tauri-pilot url
```

The plugin answers this one from the webview itself, not from the injected
bridge, so it still works on a page the bridge cannot drive — a foreign origin
missing from `remote.urls`, where the bridge commands fail. `windows`,
`navigate` and `press` do not go through the bridge either and keep working
there; `navigate` to an app URL is the way back.

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

**Positional `[target]`** is parsed the same way as for `click`, `text`, `value`, etc., except that `wait` only accepts a snapshot ref or a CSS selector — coordinate targets (`x,y`) are not supported here, since waiting for a position has no meaning:

- `@e3` or `e3` — snapshot ref (resolved via `idMap`); the `@` is optional
- anything else that is not `e<digits>` — CSS selector (e.g. `#loading-spinner`, `.toast-success`, `[data-test=foo]`)

`--selector` takes precedence when both are provided.

**Options:**

| Option | Description |
|--------|-------------|
| `--selector <sel>` | CSS selector to wait for (overrides positional `[target]`) |
| `--gone` | Wait for the element to disappear instead of appear |
| `--timeout <ms>` | Maximum wait time in milliseconds (default: 10000) |

**Example:**

```bash
tauri-pilot wait "#trigger-deferred"          # CSS selector via positional
tauri-pilot wait "@e3"                        # snapshot ref
tauri-pilot wait --selector "#spinner" --gone
tauri-pilot wait --selector ".toast-success" --timeout 5000
```

---

### `logs`

Display or stream captured console logs (`console.log`, `console.warn`, `console.error`, `console.info`), plus uncaught exceptions and unhandled promise rejections (prefixed `Unhandled rejection: `) at level `error`. Failed resource loads (`<img>` / `<script>` 404) are not recorded.

The JS bridge monkey-patches the browser console methods, listens for `error` and `unhandledrejection`, and stores entries in a 500-entry ring buffer with timestamp, level, serialized arguments, and source location. The human-readable renderer prints `source` after the arguments when it is set.

Console capture is an accessor on each method, so a page that assigns its own `console.log` is chained behind the bridge instead of replacing it: the page's function still runs, and reading `console.log` back returns the bridge's entry point rather than that function. Two cases drop capture until the next `logs` call restores it, and lines logged in between are lost: redefining the property outright (React's dev build does while it builds a component stack), and assigning to a method that cannot be redefined, where the bridge could only install itself by plain assignment.

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
[14:32:01.123Z] log App initialized
[14:32:01.456Z] warn Deprecated API call
[14:32:02.789Z] ✗ error Failed to fetch: NetworkError

# Filter by level
$ tauri-pilot logs --level error
[14:32:02.789Z] ✗ error Failed to fetch: NetworkError

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

The JS bridge monkey-patches `fetch` and `XMLHttpRequest` and stores entries in a 200-entry ring buffer with timestamp, method, URL, status code, duration, and error details. The plugin's own `__callback` IPC (`ipc://localhost/plugin:pilot|__callback` and the Windows `http(s)://ipc.localhost/...` form) is not recorded.

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
[14:32:01.123Z] GET  https://api.github.com/repos  200  125ms
[14:32:02.456Z] POST https://api.github.com/graphql  200  340ms
[14:32:03.789Z] GET  https://api.github.com/rate_limit  403  12ms

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

**JSON output format:**

```json
[
  {
    "id": 1,
    "timestamp": 1712073600000,
    "method": "GET",
    "url": "https://api.github.com/repos",
    "status": 200,
    "duration_ms": 125,
    "error": null,
    "request_size": 0,
    "response_size": 1024
  }
]
```

`response_size` is `number | null`: `null` means the size is unknown, not
zero. A response carries no size when it has no `Content-Length` (`tauri://`
never sends one), when the body is of a type the bridge cannot measure
without buffering it, or when no response arrived at all (network error,
timeout, abort). An explicit `Content-Length: 0` reports `0`.

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

# A missing key prints "(not found)" on stderr and exits 1.
# Connection and RPC errors exit 1 too, with "Error: ..." on stderr.
$ tauri-pilot storage get "missing" || echo absent
(not found)
absent

# With --json a missing key prints {"found": false} and still exits 1
$ tauri-pilot storage get "missing" --json
{
  "found": false
}

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

Stop recording and save captured interactions to a JSON file. Without a
recording in progress it fails with `No recording in progress`, exits 1, and
writes no file.

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

### `run`

Execute a declarative TOML scenario — steps with actions, assertions and
timeouts. Exits 0 when every step passes, 1 on any failure.

```bash
tauri-pilot run <scenario.toml> [OPTIONS]
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `<scenario>` | Path to the scenario TOML file |

**Options:**

| Option | Description |
|--------|-------------|
| `--junit <FILE>` | Write a JUnit XML report to this path |
| `--no-fail-fast` | Keep running the remaining steps after a failure |
| `--screenshots-dir <DIR>` | Directory for failure screenshots. Default `./tauri-pilot-failures`, resolved against the working directory |
| `--json` | Print the JSON report on stdout. The text summary goes to stderr either way (global flag) |

**Step keys:**

Every `[[step]]` takes `action`, plus optional `name` and `timeout_ms`. The
other keys depend on the action:

| Action | Required | Optional |
|--------|----------|----------|
| `click`, `check` | `target` | |
| `fill`, `select` | `target` | `value` |
| `type` | `target` | `text` |
| `press` | `key` | |
| `scroll` | | `target`, `direction`, `amount` |
| `navigate` | `url` | |
| `wait` | `target` or `selector`, not both | `gone` |
| `watch` | | `selector`, `stable`, `require_mutation` |
| `eval` | `script` | |
| `screenshot` | | `path`, `selector` |
| `assert-exists`, `assert-visible`, `assert-hidden` | `target` | |
| `assert-text`, `assert-value` | `target`, `expected` | |
| `assert-url` | `expected` | |
| `storage-get` | `key` | |

`run` checks every step against this table before it connects. An unknown
action, a missing required key, or a key the action does not read fails the
whole file, and no step runs:

```text
Error: Failed to load scenario: sel.toml

Caused by:
    0: Invalid scenario
    1: step 1: step 'assert-exists' does not accept 'selector'; use 'target'
```

A failed step captures a screenshot and reports where it landed as
`screenshot`, or why it could not be written as `screenshot_error` — in
`run --json`, and as `<system-out>` in the JUnit XML. The reported path is
always absolute, so a CI job can upload it from any directory. A step the
CLI gave up on before the app answered gets no screenshot: `--rpc-timeout` ran
out, or its `timeout_ms` did on any action but `wait` and `watch`, which pass
it to the app instead. The app may still answer that abandoned request on the
connection, so `screenshot_error` says the connection is out of sync. With
`--no-fail-fast`, the next step opens a fresh connection.

**Example:**

```bash
tauri-pilot run docs/examples/login-flow.toml
tauri-pilot run scenario.toml --no-fail-fast --junit results.xml
tauri-pilot run scenario.toml --json --screenshots-dir /tmp/shots
```

The same scenarios run over MCP through `pilot.run`, which returns the JSON
report instead of JUnit XML — see [`mcp`](#mcp).

---

## JSON-RPC Protocol

The CLI communicates with the plugin over a Unix socket using a hand-rolled JSON-RPC 2.0 protocol with newline-delimited framing (`\n`).

Every request is one line of at most 1 MiB (1,048,576 bytes), newline
included. The plugin answers a longer line with a `-32700` error whose `id` is
`null`, then closes the connection. The CLI refuses such a request before
sending it, with
`<method> request is N bytes; the plugin accepts at most 1048576 bytes`.

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
