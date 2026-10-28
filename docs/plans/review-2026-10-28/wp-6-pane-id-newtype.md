# PaneId Newtype

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace raw tmux pane-id strings with a validated `PaneId` type, so a pane id is parsed once and cannot be confused with a window name or target.

## Context

Open since the 2026-10-22 review; carried in `docs/plans/2026-10-28-codebase-review.md` (magic wand item 3). G3 in that report (a second, weaker `$TMUX_PANE` parser in `src/agent_tree/diff_pane.rs`) is the drift this type prevents. This is a refactor: no behaviour change. If WP "Agent Tree Pane Fixes" has not landed, coordinate with its agent on `src/agent_tree/diff_pane.rs`.

## Findings

### 🟢 Low — pane ids travel as `&str`/`String` (`src/tmux.rs`, `src/models/tmux_window.rs`)

**Issue:** Pane ids pass as strings through `src/tmux.rs`, `src/models/tmux_window.rs`, `src/dispatch/split_panes.rs`, `src/dispatch/agents.rs`, `src/agent_tree/diff_pane.rs`, `src/runtime/split.rs`, `src/tui/messages/split.rs` and others (`grep -rn 'pane_id' src`). Validation (`tmux::is_pane_id`) is optional at each entry point.

**Fix:** Add `PaneId` (e.g. in `src/tmux.rs` or `src/models/tmux_window.rs`) with a fallible constructor using `is_pane_id`, `Display`, and `as_str` for argv. Make `tmux::self_pane_id` return `Option<PaneId>`. Convert call sites module by module; parse at the boundary (env, tmux output, store rows), keep `PaneId` inside. Read `docs/conventions.md` on visibility and newtypes first.

## Changes

| File | Change |
|------|--------|
| `src/tmux.rs` | `PaneId` type; `self_pane_id` returns it; tmux helpers take it |
| `src/models/tmux_window.rs` | Fields typed `PaneId` where they hold pane ids |
| `src/dispatch/split_panes.rs`, `src/dispatch/agents.rs` | Use `PaneId` |
| `src/agent_tree/diff_pane.rs`, `src/agent_tree/` | Use `PaneId` |
| `src/runtime/split.rs`, `src/tui/messages/split.rs`, `src/tui/commands/split.rs` | Use `PaneId` |
| `docs/module-map.md` | Mention the type where `tmux.rs` is described |

## Verification

- [ ] Unit tests for `PaneId` parsing (valid `%N`, empty, window names)
- [ ] `cargo test --no-fail-fast` with `tmux` on `PATH`; no `skipping:` lines
- [ ] `cargo clippy --all-targets -- -D warnings`; `cargo fmt --check && ./scripts/check-doc-paths.sh`
