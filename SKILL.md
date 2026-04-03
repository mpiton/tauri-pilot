# tauri-pilot — Skill for Claude Code

## Overview

`tauri-pilot` lets you inspect, interact with, and test a running Tauri v2 app via CLI. It communicates over a Unix socket using JSON-RPC 2.0.

## Standard Workflow

```
1. ping          — verify connectivity
2. snapshot -i   — get interactive elements with refs
3. read refs     — inspect specific elements (text, value, attrs)
4. act on refs   — click, fill, type, select, check
5. snapshot -i   — verify result
```

## Rules

1. **Always snapshot before interacting.** Refs reset on each snapshot call.
2. **Prefer `snapshot -i`** (interactive only) to minimize output tokens.
3. **Use `wait` after async actions** (navigation, data loading, animations).
4. **Use `@ref` notation** (e.g., `@e3`) to target elements from the snapshot.
5. **One action at a time**, then re-snapshot to verify.
6. **Check `logs --level error`** after actions to catch JS errors.

## Commands Reference

| Command | Description | Example |
|---------|-------------|---------|
| `ping` | Check connectivity | `tauri-pilot ping` |
| `snapshot` | Accessibility tree | `tauri-pilot snapshot -i` |
| `click` | Click element | `tauri-pilot click @e3` |
| `fill` | Set input value | `tauri-pilot fill @e2 "hello"` |
| `type` | Type characters | `tauri-pilot type @e2 "abc"` |
| `press` | Press key | `tauri-pilot press Enter` |
| `select` | Select option | `tauri-pilot select @e5 "opt1"` |
| `check` | Toggle checkbox | `tauri-pilot check @e6` |
| `scroll` | Scroll page/element | `tauri-pilot scroll down 500` |
| `text` | Get text content | `tauri-pilot text @e1` |
| `html` | Get innerHTML | `tauri-pilot html @e1` |
| `value` | Get input value | `tauri-pilot value @e2` |
| `attrs` | Get attributes | `tauri-pilot attrs @e1` |
| `eval` | Run arbitrary JS | `tauri-pilot eval "document.title"` |
| `ipc` | Invoke Tauri IPC command | `tauri-pilot ipc greet '{"name":"World"}'` |
| `wait` | Wait for element | `tauri-pilot wait --selector ".loaded"` |
| `navigate` | Go to URL | `tauri-pilot navigate "https://..."` |
| `url` | Get current URL | `tauri-pilot url` |
| `title` | Get page title | `tauri-pilot title` |
| `state` | Get app state | `tauri-pilot state` |
| `screenshot` | Capture PNG | `tauri-pilot screenshot ./out.png` |
| `logs` | Show console output | `tauri-pilot logs` |
| `logs --level` | Filter by level | `tauri-pilot logs --level error` |
| `logs --last` | Last N entries | `tauri-pilot logs --last 10` |
| `logs --follow` | Stream logs | `tauri-pilot logs -f` |
| `logs --clear` | Flush buffer | `tauri-pilot logs --clear` |

## Example Workflows

### Test a login form

```bash
tauri-pilot ping
tauri-pilot snapshot -i
# Output: textbox "Email" [ref=e1], textbox "Password" [ref=e2], button "Login" [ref=e3]
tauri-pilot fill @e1 "user@example.com"
tauri-pilot fill @e2 "password123"
tauri-pilot click @e3
tauri-pilot wait --selector ".dashboard"
tauri-pilot snapshot -i
```

### Verify page content

```bash
tauri-pilot snapshot
# Full tree — look for specific text
tauri-pilot text @e5
tauri-pilot attrs @e5
tauri-pilot html @e5
```

### Test navigation

```bash
tauri-pilot url
tauri-pilot navigate "/settings"
tauri-pilot wait --selector "#settings-page"
tauri-pilot snapshot -i
```

### Debug with console logs

```bash
tauri-pilot logs --clear
tauri-pilot click @e3           # trigger action
tauri-pilot logs --level error  # check for JS errors
tauri-pilot logs --json         # structured output for parsing
```

## Flags

- `--socket <path>` — Explicit socket path (auto-detected by default)
- `--json` — Output raw JSON instead of compact text
- `-i` / `--interactive` — Snapshot interactive elements only
- `-s` / `--selector` — Scope snapshot to CSS selector
- `-d` / `--depth` — Limit snapshot depth

## Socket Auto-Detection

If `--socket` is not provided, the CLI looks for:
1. `$TAURI_PILOT_SOCKET` env var
2. Most recent `/tmp/tauri-pilot-*.sock` file
