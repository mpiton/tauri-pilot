## Summary

<!-- Brief description of the changes (1-3 bullet points) -->

-

## Motivation

<!-- Why is this change needed? Link related issues with "Closes #123" -->

## Changes

<!-- What was changed and how? -->

## Test Plan

- [ ] `cargo test --workspace` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo clippy --workspace --all-targets --config 'profile.dev.package.tauri-plugin-pilot.debug-assertions=false' -- -D warnings` passes
- [ ] Tested manually with a Tauri app (if applicable)

## Checklist

- [ ] No `.unwrap()` outside of tests
- [ ] All new files are under 150 lines
- [ ] Error handling uses `thiserror` (plugin) or `anyhow` (CLI)
- [ ] Commit messages follow conventional commits (`feat:`, `fix:`, `refactor:`, etc.)
