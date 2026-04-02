---
title: Getting Started
description: Install tauri-pilot and start testing your Tauri v2 app interactively from the command line.
---

## Requirements

- Linux (WebKitGTK) — macOS/Windows planned
- Tauri v2 (v1 not supported)
- Rust 1.94.1+ (LTS, edition 2024)

## Installation

### 1. Add the plugin to your Tauri app

Add the plugin to `src-tauri/Cargo.toml`:

```toml
[dependencies]
tauri-plugin-pilot = { git = "https://github.com/mpiton/tauri-pilot" }
```

### 2. Register the plugin

Register the plugin in `src-tauri/src/main.rs`. The plugin is gated to debug builds only — it has no effect in production releases:

```rust
// src-tauri/src/main.rs
fn main() {
    let mut builder = tauri::Builder::default();

    #[cfg(debug_assertions)]
    {
        builder = builder.plugin(tauri_plugin_pilot::init());
    }

    builder.run(tauri::generate_context!()).expect("error running app");
}
```

### 3. Install the CLI

```bash
cargo install tauri-pilot
```

## Quick Start

```bash
# Check connection
tauri-pilot ping

# Inspect the UI
tauri-pilot snapshot -i          # interactive elements only
tauri-pilot snapshot -s "#sidebar"  # scoped to a CSS selector

# Interact
tauri-pilot click @e3
tauri-pilot fill @e2 "hello"
tauri-pilot press Enter

# Verify
tauri-pilot text @e1
tauri-pilot wait --selector ".success-message"
tauri-pilot screenshot ./capture.png
```

## Basic Usage Flow

tauri-pilot follows a **ping → snapshot → interact → verify** workflow:

1. **Ping** — verify the plugin is running and the socket is reachable.
2. **Snapshot** — capture the current state of the UI. This assigns stable refs (`@e1`, `@e2`, …) to every element. Refs are reset on each snapshot, so always snapshot before interacting.
3. **Interact** — use refs to click, fill, press, or scroll elements.
4. **Verify** — read text, wait for selectors, or take a screenshot to confirm the expected state.

### Example snapshot output

```
$ tauri-pilot snapshot -i
- heading "PR Dashboard" [ref=e1]
- textbox "Search PRs" [ref=e2] value=""
- button "Refresh" [ref=e3]
- list "PR List" [ref=e4]
  - listitem "fix: resolve memory leak #142" [ref=e5]
  - listitem "feat: add workspace support #138" [ref=e6]
- button "Load More" [ref=e7]
```

After this snapshot, `@e3` refers to the "Refresh" button. You can then run:

```bash
tauri-pilot click @e3
```

If the UI changes (navigation, re-render), take a new snapshot before using refs again.
