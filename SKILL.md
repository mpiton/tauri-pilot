---
name: tauri-pilot
description: Inspect, interact with, and test a running Tauri v2 app via CLI. Communicates over Unix socket using JSON-RPC 2.0. Use when testing UI, automating interactions, or debugging a Tauri app.
---

# tauri-pilot

## Workflow

```text
1. ping          — verify connectivity
2. snapshot -i   — get interactive elements with refs
3. read refs     — inspect elements (text, value, attrs)
4. act on refs   — click, fill, type, select, check
5. assert        — verify result in one step (exit 0 = pass, exit 1 = fail)
```

## Rules

1. **Always snapshot before interacting.** Refs reset on each snapshot.
2. **Prefer `snapshot -i`** to minimize output.
3. **Use `wait` after async actions** (navigation, data loading).
4. **One action at a time**, then re-snapshot to verify.
5. **Check `logs --level error`** after actions to catch JS errors.

## Targeting

Three target formats, auto-detected:

| Format | Example | Usage |
|--------|---------|-------|
| `@ref` | `@e3` | Element ref from last snapshot |
| CSS selector | `#login-btn`, `.card` | Direct DOM query |
| Coordinates | `100,200` | Click at x,y position |

## Commands

### Connectivity & Windows

| Command | Description |
|---------|-------------|
| `ping` | Check connectivity |
| `windows` | List all open windows (label, URL, title) |
| `state` | Get app state (URL, title, viewport, scroll) |
| `url` | Get current URL |
| `title` | Get page title |

### Snapshot & Inspection

| Command | Description |
|---------|-------------|
| `snapshot` | Full accessibility tree |
| `snapshot -i` | Interactive elements only |
| `snapshot -s ".panel"` | Scope to CSS selector |
| `snapshot -d 3` | Limit tree depth |
| `snapshot --save file.snap` | Save snapshot to file |
| `diff` | Show changes since last snapshot |
| `diff --ref file.snap` | Diff against a saved snapshot |
| `text <target>` | Get text content |
| `html [target]` | Get innerHTML (page if no target) |
| `value <target>` | Get input value |
| `attrs <target>` | Get all attributes |

### Interaction

| Command | Example |
|---------|---------|
| `click <target>` | `click @e3` |
| `fill <target> <value>` | `fill @e2 "hello"` |
| `type <target> <text>` | `type @e2 "abc"` |
| `press <key>` | `press Enter` |
| `select <target> <value>` | `select @e5 "opt1"` |
| `check <target>` | `check @e6` |
| `scroll <dir> [amount] [--ref <target>]` | `scroll down 500` |
| `drag <source> [target] [--offset X,Y]` | `drag @e5 @e8` |
| `drop <target> --file <path>` | `drop @e3 --file ./img.png` |

### Assertions

| Command | Example |
|---------|---------|
| `assert text <target> <expected>` | `assert text @e1 "Dashboard"` |
| `assert visible <target>` | `assert visible @e3` |
| `assert hidden <target>` | `assert hidden @e3` |
| `assert value <target> <expected>` | `assert value @e2 "workspace"` |
| `assert count <selector> <n>` | `assert count ".item" 5` |
| `assert checked <target>` | `assert checked @e8` |
| `assert contains <target> <substr>` | `assert contains @e1 "error"` |
| `assert url <substr>` | `assert url "/dashboard"` |

Exit code 0 + `ok` on success. Exit code 1 + `FAIL: ...` on failure. Prefer `assert` over manual `text` + compare — saves one round-trip and parsing.

### Navigation & Waiting

| Command | Description |
|---------|-------------|
| `navigate <url>` | Go to URL |
| `wait [target]` | Wait for element to appear |
| `wait --selector ".loaded"` | Wait for CSS selector |
| `wait --gone @e3` | Wait for element to disappear |
| `wait --timeout 5000` | Custom timeout (default: 10000ms) |
| `watch [--selector ".el"]` | Watch for DOM mutations (MutationObserver) |
| `watch --timeout 3000 --stable 500` | Custom timeout and stability window |

### Storage & Forms

| Command | Description |
|---------|-------------|
| `storage get <key>` | Read from localStorage |
| `storage set <key> <value>` | Write to localStorage |
| `storage list` | Dump all key-value pairs |
| `storage clear` | Clear all storage |
| `storage --session <command>` | Use sessionStorage instead (applies to all storage commands) |
| `forms` | Dump all form fields on the page |
| `forms --selector "#login"` | Target a specific form |

### Debugging

| Command | Description |
|---------|-------------|
| `eval <script>` | Run arbitrary JS |
| `ipc <command> [--args <json>]` | Invoke Tauri IPC command |
| `screenshot [path] [--selector ".el"]` | Capture PNG |
| `logs` | Show console output |
| `logs --level error` | Filter by level (log/info/warn/error) |
| `logs --last 10` | Last N entries |
| `logs -f` | Stream logs (follow) |
| `logs --clear` | Flush buffer |
| `network` | Show captured network requests |
| `network --last 10` | Last N requests |
| `network --filter "api/"` | Filter by URL pattern |
| `network --failed` | Only 4xx/5xx and network errors |
| `network -f` | Stream requests (follow) |
| `network --clear` | Flush request buffer |

### Record & Replay

| Command | Description |
|---------|-------------|
| `record start` | Start recording interactions |
| `record stop --output <file>` | Save recorded interactions to JSON |
| `record status` | Check if recording is active |
| `replay <file>` | Replay recorded session with original timing |
| `replay <file> --export sh` | Export recording as executable shell script |

## Global Flags

| Flag | Description |
|------|-------------|
| `--socket <path>` | Explicit socket path (auto-detected by default) |
| `--window <label>` | Target a specific window (env: `TAURI_PILOT_WINDOW`). Default: `main` or first available |
| `--json` | Raw JSON output |

## Socket Auto-Detection

1. `$TAURI_PILOT_SOCKET` env var
2. Most recent `/tmp/tauri-pilot-*.sock` file

## Examples

```bash
# Login form test
tauri-pilot ping
tauri-pilot snapshot -i
tauri-pilot fill @e1 "user@example.com"
tauri-pilot fill @e2 "password123"
tauri-pilot click @e3
tauri-pilot wait --selector ".dashboard"
tauri-pilot snapshot -i
tauri-pilot assert text @e1 "Welcome"
tauri-pilot assert url "/dashboard"

# Verify element state
tauri-pilot snapshot -i
tauri-pilot assert checked @e8
tauri-pilot assert visible @e3
tauri-pilot assert count ".list-item" 5

# Debug after action
tauri-pilot logs --clear
tauri-pilot click @e3
tauri-pilot logs --level error

# IPC call
tauri-pilot ipc greet --args '{"name":"World"}'

# Multi-window app
tauri-pilot windows
tauri-pilot --window settings snapshot -i
tauri-pilot --window settings fill @e2 "dark"
tauri-pilot --window settings click @e3
```
