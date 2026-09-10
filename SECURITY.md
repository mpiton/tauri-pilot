# Security Policy

## Scope

tauri-pilot is a **debug-only** tool. The plugin runs exclusively under `#[cfg(debug_assertions)]` and should never be included in production builds. Desktop Unix sockets are local files with owner-only permissions.

Android uses the abstract Unix namespace, which has no filesystem permissions. Each instance gets an OS-random socket name, reported in the app's info-level logs, to prevent predictable name pre-binding. Connections are authorized using kernel peer credentials (`SO_PEERCRED`): matching UID/GID pairs for the app, root (0) or ADB shell (2000). Access also depends on the device's SELinux policy. ADB and access to the app's startup logs are trusted; the protocol does not perform a separate server-authentication handshake.

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
