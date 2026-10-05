# Complexity Cleanup

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Shrink the longest functions and widest parameter lists, and move inline tests out of large files.

## Context

This work package addresses findings from the code review in `docs/plans/review-2026-10-05/report.md`.

## Findings

### 💡 Long functions

**Issue:** `src/tui/input/table.rs:297 run_row` 211 lines; `src/runtime/mod.rs:990 bootstrap_inner` 191; `src/cli/agent_tree.rs:1061 dispatch_key` 150; `src/runtime/mod.rs:630 run_tui` 129; `src/tui/ui/kanban/cards.rs:548 build_task_list_item` 123.

**Fix:** Extract named steps, as the 2026-10-02 splits did. Keep behaviour identical.

### 💡 8-parameter functions (`src/runtime/mod.rs:1416 run_loop`, `:1185 build_runtime`, `src/db/queries/tasks.rs:98 upsert_feed_item`)

**Fix:** Introduce a context or params struct.

### 💡 Scattered `env::var` reads (17 in `src/`)

**Fix:** Read once at the entry point into a config struct and pass it down.

### 💡 Inline tests in a large file (`src/setup/plugins.rs`)

**Fix:** Move the ~2,200 test lines to a sibling `tests` module. Keep the `skill_body` helper contract that CLAUDE.md names.

## Changes

| File | Change |
|------|--------|
| `src/tui/input/table.rs`, `src/runtime/mod.rs`, `src/cli/agent_tree.rs`, `src/tui/ui/kanban/cards.rs` | Split long functions |
| `src/runtime/mod.rs`, `src/db/queries/tasks.rs` | Params structs |
| `src/setup/plugins.rs` | Move tests out |
| `src/` (env reads) | Central config struct |

## Verification

- [ ] Run existing tests — all pass (no behaviour change; snapshots unchanged)
- [ ] `cargo clippy --all-targets -- -D warnings`; `cargo fmt`
