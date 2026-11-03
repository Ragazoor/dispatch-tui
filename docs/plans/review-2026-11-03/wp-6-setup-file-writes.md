# Setup File Writes

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Setup never leaves a live agent reading a torn hook script or settings file, and never loses Claude Code's own concurrent writes to `~/.claude.json`.

## Context

Findings F4 and F5 from `docs/plans/2026-11-03-codebase-review.md` (fresh area `src/setup/`). Tests use temp directories; do not verify by running the TUI (CLAUDE.md: a throwaway `--data-dir` still rewrites the real Claude configuration).

## Findings

### 🟡 Medium — F4: setup rewrites live hook scripts and settings in place (`src/setup/mod.rs::write_file_if_changed`, `src/setup/plugins.rs::write_shipped_script`, `::write_script_manifest`)

**Issue:** it uses `fs::write` (truncate, then write) for every plugin file including `hooks/scripts/*.sh`, `dispatch-statusline.json` (passed via `--settings` at every spawn) and the feed scripts. For a new hook, `set_permissions(0755)` comes after the write. A board restart runs this while agent windows live; hooks fire on every tool call and bash reads scripts as it goes, so a hook can run a torn file and a spawn can read empty JSON.

**Fix:** route all of these through `crate::atomic_write::write_bytes` (or its equivalent) with the mode set at creation (0755 for scripts, 0644 otherwise). Test that the mode is right on a new file and that the write goes through a temp file and rename.

### 🟡 Medium (plausible) — F5: updates to `~/.claude.json` can be lost (`src/setup/config_update.rs::apply_mcp_setup`, `src/setup/config.rs::remove_mcp_config`)

**Issue:** the read-merge-rename is atomic against torn files but not against Claude Code's own frequent writes (trust, project state); a write that lands between our read and our rename is lost.

**Fix:** just before the rename, re-read the file and compare (mtime and length, or a hash) with what was merged; on a change, re-merge and retry a bounded number of times. Test with a simulated concurrent write between read and rename.

## Changes

| File | Change |
|------|--------|
| `src/setup/mod.rs` | `write_file_if_changed` atomic with mode |
| `src/setup/plugins.rs` | Shipped scripts and manifest atomic |
| `src/setup/config_update.rs`, `src/setup/config.rs` | Compare-and-retry before rename |

## Verification

- [ ] New tests fail before the change and pass after
- [ ] `cargo test --no-fail-fast` green
- [ ] `cargo clippy --all-targets -- -D warnings` clean
