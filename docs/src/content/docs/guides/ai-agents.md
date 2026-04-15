---
title: AI Agent Integration
description: How to use tauri-pilot to let AI agents interact with Tauri app UIs programmatically via an accessibility tree and ref system.
---

## Why tauri-pilot for AI agents

No existing tool lets AI agents interact with Tauri app UIs. The gap exists because:

- **Playwright doesn't work** — Tauri uses WebKitGTK, not Chromium. Playwright has no driver for WebKitGTK.
- **tauri-pilot speaks a protocol optimized for LLM consumption** — the accessibility tree output is text-based, compact, and structured to be read directly by a language model.
- **Refs map to UI elements unambiguously** — `@e3` is a stable handle within a snapshot, removing the need for CSS selectors or XPath expressions.

## The snapshot → interact → verify workflow

The core loop for any AI agent using tauri-pilot is: take a snapshot to discover elements, interact using refs, then verify the result.

```bash
# Step 1: Get the accessibility tree
tauri-pilot snapshot -i

# Output:
# - heading "PR Dashboard" [ref=e1]
# - textbox "Search PRs" [ref=e2] value=""
# - button "Refresh" [ref=e3]
# - list "PR List" [ref=e4]
#   - listitem "fix: resolve memory leak #142" [ref=e5]

# Step 2: Interact using refs
tauri-pilot click @e3

# Step 3: See what changed (instead of re-reading the full tree)
tauri-pilot diff
```

The `diff` command compares the current page with the last snapshot and returns only added, removed, and changed elements. This saves significant tokens — a typical diff after a click is 2-5 lines vs 50-100 for a full re-snapshot.

The `-i` flag filters to interactive elements only, reducing noise in the output.

## Structured output with --json

Use the `--json` flag to get machine-parseable output when the agent needs to process responses programmatically.

```bash
tauri-pilot snapshot --json
tauri-pilot text @e1 --json
tauri-pilot url --json
```

JSON output is useful when the agent needs to extract specific values, compare states, or pass data between steps without text parsing.

## Example: Automated UI testing workflow

```bash
# 1. Health check
tauri-pilot ping

# 2. Navigate to a page
tauri-pilot navigate "http://localhost:1420/settings"

# 3. Wait for page load
tauri-pilot wait --selector ".settings-form"

# 4. Snapshot to discover elements
tauri-pilot snapshot -i

# 5. Fill a form
tauri-pilot fill @e2 "new-value"

# 6. Submit
tauri-pilot click @e5

# 7. Verify success
tauri-pilot wait --selector ".success-toast"
tauri-pilot snapshot -i
tauri-pilot assert text @e1 "Settings saved"
tauri-pilot assert url "/settings"
```

## Best practices for snapshot parsing

- **Always take a fresh snapshot before interacting** — refs reset on each snapshot. `@e1` in one snapshot may refer to a different element in the next.
- **Use `diff` instead of re-snapshotting** — after an interaction, `tauri-pilot diff` returns only what changed. This is much cheaper than re-reading the full tree.
- **Use `-i` to filter interactive elements** — this reduces output size and makes the tree easier to parse.
- **Use `-s` to scope to a section** — `tauri-pilot snapshot -s "#sidebar"` limits the tree to a subtree, further reducing noise.
- **Use `wait` before snapshot** — after navigation or interaction, wait for the page to settle before taking a snapshot to avoid acting on stale state.
- **Use `assert` for verification** — `tauri-pilot assert text @e1 "Dashboard"` returns exit 0 on match, exit 1 on mismatch. This replaces the three-step `text @e1` → parse → compare pattern, saving a round-trip and token parsing.
- **Save snapshots for multi-step workflows** — `tauri-pilot snapshot --save before.snap` then `tauri-pilot diff --ref before.snap` lets you compare against any point in time.
- **Record agent workflows for regression testing** — wrap your interaction sequence in `record start` / `record stop --output test.json` to capture actions as a replayable script. Use `replay test.json` to re-run the same sequence, or `replay test.json --export sh` to generate a standalone shell script.

```bash
# Assert examples — one-step verification
tauri-pilot assert text @e1 "Dashboard"           # exact text match
tauri-pilot assert visible @e3                     # element is visible
tauri-pilot assert value @e2 "workspace"           # input value
tauri-pilot assert count ".list-item" 5            # element count
tauri-pilot assert contains @e1 "error"            # partial text match
tauri-pilot assert url "/dashboard"                # URL substring

# Scoped snapshot example
tauri-pilot snapshot -i -s "#main-content"
```

## Multi-window apps

For Tauri apps with multiple windows, use `tauri-pilot windows` to discover available windows and `--window <label>` to target a specific one:

```bash
# List all windows
tauri-pilot windows
# main      http://localhost:1420/        My App
# settings  http://localhost:1420/settings Settings

# Target a specific window
tauri-pilot --window settings snapshot -i
tauri-pilot --window settings fill @e2 "new-value"

# Or set via environment variable
export TAURI_PILOT_WINDOW=settings
tauri-pilot snapshot -i
```

Without `--window`, all commands target the `main` window (or the first available). The window flag works with every command including `--follow` mode and `ipc`.

## Integration with Claude Code

tauri-pilot is designed to be used as a tool by Claude Code directly:

- Claude Code can read the snapshot output directly — it is an accessibility tree, the same representation Claude Code uses internally for UI reasoning.
- The ref system (`@e1`) maps directly to how Claude Code thinks about UI elements as discrete, addressable targets.
- Combine with `--json` for structured data extraction when the agent needs to compare values or branch on state.

A typical Claude Code session using tauri-pilot looks like this:

```bash
# Claude Code calls these as shell commands during a task
tauri-pilot ping                          # verify the app is running
tauri-pilot snapshot -i                   # discover what's on screen
tauri-pilot fill @e2 "search query"       # interact
tauri-pilot click @e3                     # submit
tauri-pilot wait --selector ".results"    # wait for response
tauri-pilot diff                          # see what changed (token-efficient)
tauri-pilot snapshot -i                   # refresh refs
tauri-pilot assert text @e1 "Results"     # verify in one step
```

No custom integration code is needed — tauri-pilot is a CLI that Claude Code can invoke directly.

## MCP Server

Agents with native Model Context Protocol support can use tauri-pilot as a stdio
MCP server:

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

The MCP server exposes the same app-facing commands as structured tools:
`snapshot`, `click`, `fill`, `logs`, `network`, `eval`, `ipc`, `assert_*`, and
more. Tool calls return structured JSON content, so agents do not need to parse
terminal output.

Use global flags before `mcp` when an agent should target a specific app socket or
window:

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
