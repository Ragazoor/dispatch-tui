# Cleanups

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix stale docs, bound trajectory files and prune unowned repo paths.

## Context

Findings R6 (non-admin part), C7, C8 and standing item W2 from `docs/plans/2026-10-31-codebase-review.md`. Run the `scripts/check-*.sh` doc checkers before committing.

## Findings

### 🟢 Low — R6: stale doc comment (`src/service/tasks/crud.rs::TaskService::update_task`)

**Issue:** still says "`patch_task` and `set_task_epic` recalculate the epics they touch"; it is one `update_task` reducer since 5a5209b7.

**Fix:** update the comment.

### 🟢 Low — C7: trajectory files grow without bound (`src/mcp/trajectory.rs::append_entry`)

**Issue:** full tool results go to `trajectories/<task_id>.jsonl`; nothing prunes or reads them. Concurrent appends over 2 MiB can interleave.

**Fix:** cap each `result`; delete the file when the task is deleted (or by age).

### 🟢 Low — C8: docs and help drift (`docs/reference.md` "CLI Usage"; `src/main.rs` clap help for `prune-repo-paths`; `docs/invariants.md` and `docs/conventions.md` `cmd_plan` citation)

**Issue:** "CLI Usage" claims every subcommand but omits `spacetime dump-server|restore` and `store import`; the prune help omits "only rows this host saved"; the mutation-boundary text cites `cmd_plan` → `TaskService::attach_plan`, but the path is `src/cli/commands.rs::attach_plan` (called from `src/main.rs::cmd_plan`).

**Fix:** correct each text.

### 🟢 Low — W2 (standing): unowned repo paths are never pruned (`src/sync/rows.rs::repo_paths_saved_by`; `src/cli/commands.rs::prune_repo_paths`)

**Issue:** rows with `host == ""` (saved before hosts were stamped) are filtered out of every host's prune, so they live forever.

**Fix:** let prune adopt an unowned row whose path exists on this host and delete one whose path is missing here; record the rule in `docs/specs/cli.allium` (`PruneRepoPathsViaCli`) first.

## Changes

| File | Change |
|------|--------|
| `src/service/tasks/crud.rs` | Comment |
| `src/mcp/trajectory.rs` | Cap and retention |
| `docs/reference.md`, `src/main.rs`, `docs/invariants.md`, `docs/conventions.md` | Text fixes |
| `docs/specs/cli.allium`, `src/sync/rows.rs`, `src/cli/commands.rs` | Unowned-row prune |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass
- [ ] `./scripts/check-doc-paths.sh` and the other `check-*.sh` pass
- [ ] `cargo clippy --all-targets -- -D warnings` clean
