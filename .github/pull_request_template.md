## Summary

<!-- Brief description of the changes (1-3 bullet points) -->

-

## Motivation

<!-- Why is this change needed? Link related issues with "Closes #123" -->

## Changes

<!-- What was changed and how? -->

## Test Plan

- [ ] `cargo test --workspace` passes
- [ ] `cargo clippy --workspace -- -D warnings` passes
- [ ] Tested manually with a Tauri app (if applicable)

## Checklist

- [ ] No `.unwrap()` outside of tests
- [ ] All new files are under 150 lines
- [ ] Error handling uses `thiserror` (plugin) or `anyhow` (CLI)
- [ ] Commit messages follow conventional commits (`feat:`, `fix:`, `refactor:`, etc.)
