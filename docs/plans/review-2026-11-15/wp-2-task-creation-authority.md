# Task creation authority: phoenix respawn and the move-site guard

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** no public reducer inserts a client-built task without the aim check and identity stamp that `create_task` runs, and a test makes that true for every future reducer.

## Context

Findings V1 and G10 from `docs/plans/2026-11-15-codebase-review.md` §3.2 and §3.9. V1 is the sibling A1's fix (637732a1) missed. Spec first (`docs/specs/tasks.allium: PhoenixRespawn`, `core.allium` identity stamping), then tests, then code. Mirror module changes in `src/sync/memory_caller/` with a conformance case; bump `RULES_VERSION` and `BOARD_RULES_VERSION` together; rebuild the module and regenerate bindings.

## Findings

### 🔴 High — V1: `respawn_phoenix_successor` inserts a client-built task with no aim check and no identity stamp (`spacetime/module/src/tasks_epics.rs::respawn_phoenix_successor`; twin `src/sync/memory_caller/agent_state.rs::apply_respawn_phoenix_successor`; client `src/store/queries/tasks.rs`)

**Issue:** the reducer checks only that the predecessor is visible and phoenix and that the epic may be entered, then writes the caller's row (`..successor`): no `check_new_task_aim`, no `stamp_new_task`. Bob sees phoenix task P in Alice's feed epic and calls it with `host = alice-h1`, `worktree = /home/alice/proj/.worktrees/999-live`, `external_id = "x"`, `created_by = "alice"`. Alice's laptop ingests; `x` is absent; a teardown is owed on h1 and removes the live worktree. `created_by` is forged.

**Fix:** if no production caller needs the public reducer any more (the module respawns through `respawn_phoenix_successor_of` on close), retire it and its client path (`src/store/queries/tasks.rs`, `src/sync/sdk_connector/callers.rs`); otherwise run `check_new_task_aim` and `stamp_new_task` on both sides. Conformance case: a successor naming another person's host is refused (or the reducer no longer exists). Graduate: a module test that enumerates `#[spacetimedb::reducer]` functions writing a task with `id: 0` from a client row and asserts each calls `check_new_task_aim` and `stamp_new_task` (a source scan is acceptable; mutation-test it by removing the call from `create_task`).

### 🟢 Low — G10: the `epic_id` guard checks listed sites by name only (`tests/memory_caller_conformance/move_paths.rs::TWIN_SITES` and the module list)

**Issue:** a second `epic_id` write added inside a listed function (`apply_set_task_epic`, `apply_create_task`, the blanket `convert.rs::from`) passes; a whole-row replacement (`*task = other;`) is missed; the "mirrors X" text is unchecked. (Mutation run on 2026-11-15: new writes in *unlisted* functions are caught on both sides.)

**Fix:** record a write count per listed site, as `scripts/single-write-service.allow` does, and fail when it changes; treat `*<task> = ` as a write. Add a self-test case per miss.

## Changes

| File | Change |
|------|--------|
| `docs/specs/tasks.allium` | PhoenixRespawn: the successor is stamped and aim-checked like any create (or: no client entry point) |
| `spacetime/module/src/tasks_epics.rs` | retire or guard `respawn_phoenix_successor` |
| `src/sync/memory_caller/agent_state.rs` | mirror |
| `src/store/queries/tasks.rs`, `src/sync/sdk_connector/callers.rs` | drop or keep the client path |
| `spacetime/module/src/tests/` | reducer-insert authority test |
| `tests/memory_caller_conformance/move_paths.rs` | per-site write counts |
| `spacetime/module/src/rules.rs`, `src/sync/rules_gate.rs` | bump together |

## Verification

- [ ] `./scripts/verify.sh` passes
- [ ] The conformance case fails before the fix
- [ ] The authority test fails when `check_new_task_aim` is removed from `create_task`
- [ ] The move-site guard fails when a second `epic_id` write is added to `apply_set_task_epic`
