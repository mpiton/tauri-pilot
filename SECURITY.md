# Security Policy

## Scope

tauri-pilot is a **debug-only** tool. The plugin runs exclusively under `#[cfg(debug_assertions)]` and should never be included in production builds. The Unix socket is local-only (`/tmp/`) with user-level permissions.

## Reporting a Vulnerability

If you discover a security vulnerability, please report it responsibly:

1. **Do NOT open a public issue**
2. Email: contact@mathieu-piton.com (or use GitHub's private vulnerability reporting)
3. Include steps to reproduce and potential impact
4. We will respond within 72 hours

## Supported Versions

| Version | Supported |
|---------|-----------|
| Latest  | Yes       |
| < 1.0   | Best effort |
