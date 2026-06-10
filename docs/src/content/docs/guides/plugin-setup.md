---
title: Plugin Setup
description: How to add tauri-plugin-pilot to your Tauri application for interactive testing.
---

This guide walks you through integrating `tauri-plugin-pilot` into an existing Tauri v2 application.

## 1. Add the dependency

In your app's `src-tauri/Cargo.toml`, add the plugin under `[dependencies]`:

```toml
# src-tauri/Cargo.toml
[dependencies]
tauri-plugin-pilot = { git = "https://github.com/mpiton/tauri-pilot" }
```

## 2. Register the plugin

Register the plugin in your app entry point using a `#[cfg(debug_assertions)]` guard:

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

## 3. Debug-only compilation

The `#[cfg(debug_assertions)]` guard is intentional and important:

- The plugin is **only compiled and included in debug builds**
- Production builds (`cargo build --release`) will not include the plugin
- There is zero runtime overhead or binary size impact in production
- No need to strip or disable the plugin before shipping

## 4. Socket path

Once your app starts in dev mode, the plugin creates a Unix socket at:

```text
/tmp/tauri-pilot-{identifier}.sock
```

The `{identifier}` value comes from the `identifier` field in your `tauri.conf.json`.

**Example:** an app with identifier `com.myapp.dev` creates the socket at:

```text
/tmp/tauri-pilot-com.myapp.dev.sock
```

The CLI auto-discovers this socket when you run commands.

## 5. Permissions

The plugin requires the `pilot:default` permission for its internal `__callback` IPC command. Add it to your capability file (e.g. `src-tauri/capabilities/default.json`):

```json
{
  "permissions": ["core:default", "pilot:default"]
}
```

Without this permission, eval commands will time out with "eval timed out after 10s".

## 6. Verify the setup

Start your Tauri app in development mode, then test the connection from a second terminal:

```bash
# Terminal 1 — start your app
cargo tauri dev

# Terminal 2 — verify the plugin is reachable
tauri-pilot ping
# Connected. Plugin and CLI both 0.7.1.
```

`ping` reports the plugin version compiled into your app alongside the CLI version. When they match, you're ready to start using the snapshot/action workflow.

## 7. Keep the plugin and CLI in sync

The plugin is a Rust dependency compiled into your app. The CLI is a separate binary. They're versioned independently, so they drift apart if you update one and not the other. `ping` surfaces a drift:

```bash
tauri-pilot ping
# Connected. Plugin 0.7.0, CLI 0.7.1.
# Plugin 0.7.0 and CLI 0.7.1 differ. Rebuild your app against tauri-plugin-pilot 0.7.1 ...
```

If `ping` reports `Plugin <= 0.7.0`, or eval commands fail on macOS with `native WebKit eval callback returned an error`, the plugin baked into your app predates the unified eval path (removed in 0.7.1). Update it and rebuild:

```bash
# git dependency (the setup above): pull the latest commit
cargo update -p tauri-plugin-pilot

# or pin a released version in src-tauri/Cargo.toml:
#   tauri-plugin-pilot = "0.7.1"

# then rebuild
cargo tauri dev
```
