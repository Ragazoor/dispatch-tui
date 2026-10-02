# Long-Function Decomposition

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Break up the longest functions and the mixed-concern `startup.rs`, without changing behaviour.

## Context

Findings from `docs/plans/review-2026-10-02/report.md`, sections 3 and 4. Existing tests are the safety net; add characterisation tests first where a function has none.

## Findings

### 💡 Long functions

| Function | Lines | Fix |
|---|---|---|
| `src/db/migrations.rs:2917 migrate_v106_archived_status_migration` | 322 | One fn per phase; shared CTE consts |
| `src/runtime/mod.rs:865 bootstrap_inner` (and `run_tui` :486, `bootstrap_*` :829-871) | 303 | Named step helpers; param structs |
| `src/tui/input/normal.rs:32 handle_key_board_normal` | 277 | Per-key handlers or keymap table |
| `src/tui/input.rs:271 handle_key_activate` | 8+ nesting | `try_*` helpers chained with `or_else` |
| `src/runtime/commands.rs:134 dispatch_task` | 208 | One-line arms calling `exec_*` |
| `src/service/epics.rs:352 update_epic`, `src/service/tasks/crud.rs:253 update_task` | 175, 121 | validate / diff / persist / side-effects |
| `src/db/queries/tasks.rs:1436 upsert_feed_tasks_inner`, `:581 patch_task`, `:1081-1135` | 189, 147 | Split passes; named SQL params |
| `src/mcp/handlers/tasks/wrap_up.rs:283`, `src/dispatch/worktree.rs:604`, `src/repo_sync.rs:206`, `src/cli/agent_tree.rs:1067`, `src/setup/mod.rs:726`, `src/dispatch/finish.rs:83`, `src/feed/cycle.rs:87`, `src/feed/mod.rs:339`, `src/dispatch/agents.rs:374`, `src/mcp/handlers/learnings.rs:74` | 100-140 | Extract steps |

### 💡 `startup.rs` mixes concerns (`src/startup.rs`)

**Fix:** Split into `startup/{launch,retire,config,host}.rs`; retire first.

### 💡 Stringly-typed and primitive values

**Fix:** `PrState: FromStr` (`src/dispatch/mod.rs:199`); `resolve_repo(task)` shared by `dispatch/agents.rs:374` and `dispatch/mod.rs:199` (check other call sites first); `PaneId`/`RepoPath` newtypes (`src/tmux.rs:1241`, `expand_tilde` call sites) if cheap.

## Changes

One module per row above; no behaviour change.

## Verification

- [ ] `cargo test`, `cargo clippy --all-targets -- -D warnings`
- [ ] `./scripts/check-doc-symbols.sh` (docs cite some of these names)
