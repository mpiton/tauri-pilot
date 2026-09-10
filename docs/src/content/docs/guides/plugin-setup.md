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
  "remote": {
    "urls": ["https://allowed.example/*"]
  },
  "permissions": ["core:default", "pilot:default"]
}
```

Without this permission, eval commands will time out with "eval timed out after 10s". List a foreign origin in `remote.urls` on the same capability if you need to drive that origin; otherwise commands there fail fast.

## 6. Verify the setup

Start your Tauri app in development mode, then test the connection from a second terminal:

```bash
# Terminal 1 — start your app
cargo tauri dev

# Terminal 2 — verify the plugin is reachable
tauri-pilot ping
# Connected. Plugin and CLI both 0.7.3.
```

`ping` reports the plugin version compiled into your app alongside the CLI version. When they match, you're ready to start using the snapshot/action workflow.

## 7. Keep the plugin and CLI in sync

The plugin is a Rust dependency compiled into your app. The CLI is a separate binary. They're versioned independently, so they drift apart if you update one and not the other. `ping` surfaces a drift:

```bash
tauri-pilot ping
# Connected. Plugin 0.7.2, CLI 0.7.3.
# Plugin 0.7.2 and CLI 0.7.3 differ. Rebuild your app against tauri-plugin-pilot 0.7.3 ...
```

If `ping` reports `Plugin <= 0.7.0`, or eval commands fail on macOS with `native WebKit eval callback returned an error`, the plugin baked into your app predates the unified eval path (removed in 0.7.1). Update it and rebuild:

```bash
# git dependency (the setup above): pull the latest commit
cargo update -p tauri-plugin-pilot

# or pin a released version in src-tauri/Cargo.toml:
#   tauri-plugin-pilot = "0.7.3"

# then rebuild
cargo tauri dev
```

## Android via ADB

The computer running the CLI must use Linux or macOS. The Windows CLI uses named pipes and cannot use this Unix socket forwarding setup.

Before connecting:

- Complete [Tauri's Android prerequisites](https://v2.tauri.app/start/prerequisites/#android), including the SDK, NDK, `ANDROID_HOME` and `NDK_HOME`, and put `adb` on `PATH`.
- Run `cargo tauri android init` once for the app. Use an emulator or a device with USB debugging enabled and the computer authorized.
- Register the plugin in the mobile `run()` entry point in `src-tauri/src/lib.rs`, with the debug guard and `pilot:default` permission shown above.
- Configure the app's logging to capture `tauri_plugin_pilot` tracing events at info level. The listening event contains the socket name needed for forwarding.

Disabling the desktop `press` backend on Android is recommended. Replace the shared plugin dependency with target-specific entries so desktop builds keep it enabled:

```toml
[target.'cfg(target_os = "android")'.dependencies]
tauri-plugin-pilot = { git = "https://github.com/mpiton/tauri-pilot", default-features = false }

[target.'cfg(not(target_os = "android"))'.dependencies]
tauri-plugin-pilot = { git = "https://github.com/mpiton/tauri-pilot" }
```

Start a debug build with `cargo tauri android dev`. Select the device and read the `tauri-pilot socket listening` event from the intended app's startup logs. For apps logging to Logcat, use the Android application ID to select its process:

```sh
adb devices -l
pilot_device=YOUR_DEVICE_SERIAL
pilot_package=YOUR_ANDROID_APPLICATION_ID
pilot_pid=$(adb -s "$pilot_device" shell pidof "$pilot_package")
adb -s "$pilot_device" logcat -d --pid="$pilot_pid" | grep 'tauri-pilot socket listening'
```

Copy the complete `tauri-pilot-{identifier}-{random}.sock` name into `pilot_name` below. The identifier is limited to 16 characters, followed by 16 random hexadecimal digits. The name changes each time the app starts, so update the forwarding after a restart. Once the page has loaded:

```sh
pilot_name=NAME_FROM_STARTUP_LOG
pilot_dir=$(mktemp -d /tmp/tauri-pilot.XXXXXX)
export TAURI_PILOT_SOCKET="$pilot_dir/pilot.sock"
adb -s "$pilot_device" forward "localfilesystem:$TAURI_PILOT_SOCKET" \
  "localabstract:$pilot_name"
tauri-pilot ping
tauri-pilot snapshot
```

Use the same `-s "$pilot_device"` for all ADB commands when multiple devices are connected. `TAURI_PILOT_SOCKET` also applies to subsequent CLI commands and MCP processes launched from this shell.

`press` is unavailable on Android and `screenshot_native` is macOS-only. Use `fill` or `type` for text input. Window operations are limited to the mobile app's single window.

If forwarding succeeds but the CLI cannot connect, check `adb -s "$pilot_device" shell cat /proc/net/unix | grep tauri-pilot` and compare the name with the current startup log. The plugin only listens in debug builds.

Remove the forwarding created above when finished:

```sh
adb -s "$pilot_device" forward --remove "localfilesystem:$TAURI_PILOT_SOCKET"
rm -f "$TAURI_PILOT_SOCKET"
rmdir "$pilot_dir"
unset TAURI_PILOT_SOCKET
```

On Linux, forwarding to a socket named `tauri-pilot-{identifier}.sock` inside your private `$XDG_RUNTIME_DIR` also enables CLI auto-discovery without `TAURI_PILOT_SOCKET`. For this alternative, remove only the forward and socket when finished, keeping the runtime directory:

```sh
adb -s "$pilot_device" forward --remove "localfilesystem:$XDG_RUNTIME_DIR/tauri-pilot-{identifier}.sock"
rm -f "$XDG_RUNTIME_DIR/tauri-pilot-{identifier}.sock"
```

Use a private directory rather than a bare `/tmp` socket, since ADB creates the host socket with its own permissions.
