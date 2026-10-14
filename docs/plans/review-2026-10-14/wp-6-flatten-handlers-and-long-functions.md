# Flatten MCP handlers and long functions

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** No production function nests deeper than 4, none is over 100 lines, and no signature takes more than 7 parameters. No behaviour change.

## Context

This work package addresses findings from the 2026-10-14 code review
(`docs/plans/review-2026-10-14/report.md`, §4, §8 M2–M4). The 2026-10-08
review flattened `handle_get_epic`/`handle_list_epics`; the same
parse → look-up → match → match shape remains in every handler that was not
touched. The two ingest functions in `src/sync/memory_caller/feed.rs` are
out of scope here — work package 2 replaces them.

## Findings

### 💡 Nest-5 MCP handlers (`src/mcp/handlers/learnings.rs:129`, `:259`; `src/mcp/handlers/managed_feeds.rs:73`; `src/mcp/handlers/epics.rs:78`, `:243`; `src/mcp/handlers/tasks/crud.rs:17`, `:343`)

**Issue:** `handle_record_learning`, `handle_rate_learning`,
`handle_managed_feed_config`, `handle_create_epic`, `handle_update_epic`,
`handle_update_task`, `handle_create_task` each reach nesting 5 on the
never-panic MCP surface.

**Fix:** Parse params and resolve ids with early-return helpers (`let … else
{ return … }`), as `handle_get_epic` does now; keep each handler to one
`match` on the service result.

### 💡 Other nest-5 or 100-line functions

| Function | Lines | Nest |
|---|---:|---:|
| `src/service/tasks/wrap_up.rs:241::check_wrap_up_ready` | 86 | 5 |
| `src/runtime/commands.rs:138::dispatch_task` | 103 | 4 |
| `src/sync/sdk_connector/wiring.rs:39::wire_tables` | 105 | 3 |
| `src/spacetime/import.rs:117::import_old_store` | 73 | 5 |
| `src/cli/store_import.rs:114::read_data_dir_with` | 66 | 5 |
| `src/runtime/split.rs:46`, `:77`, `:159` (`exec_enter_split_mode*`, `exec_swap_split_pane`) | 30–39 | 5 |
| `src/runtime/pr.rs:124::report` | 51 | 5 |
| `src/runtime/tasks.rs:686::spawn_shared_store_connection` | 39 | 5 |
| `src/feed/mod.rs:70::cleanup_removed_feed_tasks` | 39 | 5 |

**Fix:** Extract the inner match arms into named functions; `wire_tables`
becomes one `wire_table!` invocation per table or a loop over a table list.

### 💡 Parameter lists over 7 (`src/tui/ui/kanban/cards.rs:709`, `src/runtime/mod.rs:984`, `src/runtime/event_loop.rs:142`, `src/runtime/tasks.rs:1012`, `src/dispatch/agents.rs:373`, `src/dispatch/worktree.rs:385`, `:566`)

**Issue:** `render_epic_item` takes 9, `build_runtime` and `run_loop` 8,
`exec_cleanup`, `launch_agent`, `resolve_start_point`, `add_worktree` 7.

**Fix:** One context struct each (`EpicItemRender`, `RuntimeParts`,
`LoopChannels`, `CleanupRequest`, `WorktreeTarget`), built at the one call
site. Remove any `#[allow(clippy::too_many_arguments)]` that becomes
unnecessary.

## Changes

| File | Change |
|------|--------|
| `src/mcp/handlers/learnings.rs`, `managed_feeds.rs`, `epics.rs`, `tasks/crud.rs` | Early-return parse/look-up helpers; one match per handler |
| `src/service/tasks/wrap_up.rs` | Split `check_wrap_up_ready` into commit check / verify run |
| `src/runtime/commands.rs` | `dispatch_task` into prologue / launch / report |
| `src/sync/sdk_connector/wiring.rs` | Table-driven `wire_tables` |
| `src/spacetime/import.rs`, `src/cli/store_import.rs`, `src/runtime/split.rs`, `src/runtime/pr.rs`, `src/runtime/tasks.rs`, `src/feed/mod.rs` | Extract inner arms |
| `src/tui/ui/kanban/cards.rs`, `src/runtime/mod.rs`, `src/runtime/event_loop.rs`, `src/runtime/tasks.rs`, `src/dispatch/agents.rs`, `src/dispatch/worktree.rs` | Context structs |

## Verification

- [ ] `cargo test --no-fail-fast` green; `cargo clippy --all-targets -- -D warnings` green
- [ ] Re-run the review's brace-depth measure (or `cargo clippy -W clippy::cognitive_complexity` as a proxy): no production function at nest ≥ 5, none over 100 lines
- [ ] `grep -rn 'too_many_arguments' src` returns nothing
