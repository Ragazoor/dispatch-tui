# Flatten Nesting and Small Duplicates

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Flatten the deepest-nested production functions and remove small copy-paste blocks, with no behaviour change.

## Context

This work package addresses the "Deepest nesting" list, S2 and S5 from `docs/plans/review-2026-10-08-followup/report.md`. Nesting was measured as brace depth inside the function body. Avoid `src/cli/agent_tree*.rs` beyond the list-cursor item (WP8 restructures it).

## Findings

### 💡 Nest 5 on the never-panic MCP surface (`src/mcp/handlers/epics.rs::handle_get_epic`, `::handle_list_epics`)

**Issue:** Depth 5 in MCP handlers, which must never panic.

**Fix:** `let … else` and early returns; keep error responses byte-identical (existing handler tests pin them).

### 💡 Nest 6 (`src/setup/plugins.rs::install_shipped_feed_scripts`, `src/spacetime/import.rs::prepare`)

**Issue:** `match` in `match` in `for`.

**Fix:** Extract the inner match into a helper returning a small enum/`Result`.

### 💡 Other nest-5 functions

`src/runtime/mod.rs::apply_loop_event`, `src/feed/ingest/routing.rs::route_and_group_entries`, `src/runtime/tasks.rs::exec_cleanup`, `src/tui/input/normal.rs::handle_key_delete_item`, `src/tui/update/epics.rs::handle_move_epic_status`, `src/tui/selection.rs::sync_board_selection`. Flatten where it is a clear win; skip where the nesting is a genuine `match` structure. Leave `apply_loop_event` if WP9 is mid-split of `runtime/mod.rs`.

### 💡 `UpdateEpicParams` has no constructor (`src/runtime/epics.rs:87`, `:168`, `:194`, `:296`; `src/runtime/editor.rs:407`; `src/mcp/handlers/epics.rs:212`)

**Issue:** Written field by field (mostly `None`) at 6 production sites. `UpdateTaskParams` has `for_task(id)` (`src/service/tasks/params.rs:117`).

**Fix:** `UpdateEpicParams::for_epic(id)` + `..` struct update.

### 💡 Small duplicates

- Identical list-cursor code (`up`, `down`, `top`, `bottom`, `half_page`) in `src/cli/agent_tree_agents.rs:81–100` and `src/cli/agent_tree_commits.rs:101–120` → shared `ListCursor`.
- Shared guard-and-find block in `src/tui/update/lifecycle.rs::handle_dispatch_task` and `::handle_trust_and_dispatch` → one helper.
- `src/tui/columns.rs:293` `unreachable!` can return `stats` directly.

## Changes

| File | Change |
|------|--------|
| `src/mcp/handlers/epics.rs` | Flatten two handlers; use `for_epic` |
| `src/setup/plugins.rs`, `src/spacetime/import.rs` | Extract inner match |
| nest-5 files above | Flatten where clear |
| `src/service/epics.rs` (or params module) | `UpdateEpicParams::for_epic` |
| `src/runtime/epics.rs`, `src/runtime/editor.rs` | Use `for_epic` |
| `src/cli/agent_tree_agents.rs`, `src/cli/agent_tree_commits.rs` | `ListCursor` |
| `src/tui/update/lifecycle.rs`, `src/tui/columns.rs` | Dedupe guard; drop `unreachable!` |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass, no snapshot changes
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] Re-run a nesting measurement on the touched functions: none above 4
