# Task Moves and Patch Rules

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Settle whether a member may detach a teammate's task, close the client-chosen `owner` patch, make the twin's `update_epic` all-or-nothing, and fill the conformance gaps left by last round's fixes.

## Context

Findings M3, M5, M6, M8 from `docs/plans/2026-11-03-codebase-review.md`. Spec first (`docs/specs/workspaces.allium`, `docs/specs/tasks.allium`), then tests that fail, then code. Mirror every module change in `src/sync/memory_caller/` and cover it in `tests/memory_caller_conformance/`. Rule-behaviour changes follow the `RULES_VERSION` doc for whether a bump is needed.

## Findings

### 🟡 Medium — M3: b5c9818a lets any member hide a teammate's task (`spacetime/module/src/visibility.rs::task_move_permitted`, `tasks_epics.rs::move_task_row`)

**Issue:** `task_move_permitted` allows `epic_id == 0 && before.is_none()`, and d2073cbe stamps `owner = caller` on a detach. The commit says moving a task off a public epic onto your own board "hides nothing new". Scenario: Alice detaches Bob's running task from a shared public epic; it lands on Alice's board, `task_visible_to` now shows it to Alice alone, and Bob's board and hooks get "does not exist".

**Fix:** per the decision below.

### 🟢 Low — M5: `TaskPatch.owner` is still taken from the client (`spacetime/module/src/rules.rs::check_task_patch`, `tasks_epics.rs::apply_task_patch`)

**Issue:** the board never sends `owner` (`src/sync/encode.rs` sends `None`), but the module applies it. A task's owner can patch `owner: "x"` and orphan the task for good (`AdoptOwnerless` adopts only empty owners).

**Fix:** refuse `owner` in `check_task_patch`; mirror in the twin; conformance case.

### 🟢 Low — M6: the twin's `update_epic` can write part of an update (`src/sync/memory_caller/tasks_epics.rs::apply_update_epic`)

**Issue:** it checks each part against the row before the patch, then applies the parts and ignores their `Refused` outcomes. `{parent_epic_id: P, workspace_id: W2}` on a root epic: the module patches, `move_epic_row` refuses "is a sub-epic" and the call rolls back; the twin writes the parent and answers `Applied`. `{parent: 0, workspace: W2}` diverges the other way.

**Fix:** propagate apply-phase refusals (or apply in the module's order on a copy and commit only on success); add both cases to the conformance suite.

### 🟢 Low — M8: conformance gaps in last round's fixes (`tests/memory_caller_conformance/`; `spacetime/module/src/visibility.rs` ab512934 guard)

**Issue:** no conformance case for the `subscribe_to_epic`/`mark_epic_seen` refusals or the epic and package `created_by` stamps (fb7b2c05); `private_moves.rs` does not cover `ReParentingKeepsTheHolder`; ab512934's `r.epic(task.epic_id).is_some()` guard cannot change an outcome and has no test.

**Fix:** add the cases; drop the guard or add a test that shows the case it decides.

## Decisions

### May a member detach a teammate's task from a public epic? (M3)

**Question:** what should happen when Alice moves Bob's task off a shared public epic to the main board?

**Options:**
- (a) Keep b5c9818a's rule; correct the spec rationale to say the task leaves its previous viewers' sight. Cheapest; teammates can still take each other's tasks.
- (b) Allow the detach only for the task's creator, the workspace admin or the super-admin. Restores the V2 guarantee; other members get a refusal.
- (c) Allow it, but keep the task visible to its previous viewers (for example by leaving `owner` empty and keeping workspace visibility). Needs a spec for "a detached task's audience".

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/workspaces.allium` | Detach rule (M3); `owner` not patchable (M5) |
| `spacetime/module/src/visibility.rs` | M3 per decision; ab512934 guard |
| `spacetime/module/src/rules.rs` | Refuse `owner` in `check_task_patch` |
| `src/sync/memory_caller/tasks_epics.rs` | `apply_update_epic` all-or-nothing; mirror M3/M5 |
| `tests/memory_caller_conformance/` | M3, M5, M6, M8 cases |

## Verification

- [ ] New tests fail before the change and pass after
- [ ] `cargo test --no-fail-fast` green
- [ ] Module checks per `docs/testing.md`; `module.wasm` and bindings rebuilt if the module changed
- [ ] `cargo clippy --all-targets -- -D warnings` clean
