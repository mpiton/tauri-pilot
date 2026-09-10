# tauri-pilot

A CLI that drives a running Tauri app through a debug-only plugin, so an agent can inspect and act on its webviews.

## Language

**Target window**:
The webview window a command acts on: the `--window` label, else `main`, else the first window by label. An unknown label is an error, never a fallback.
_Avoid_: current window, active window
