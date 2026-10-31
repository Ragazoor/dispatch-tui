# Private Moves

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Close the paths that move other people's work under a private epic, or out of sight, without visibility authority.

## Context

This work package addresses findings V2, V3, V4, V8 and V9 from `docs/plans/2026-10-31-codebase-review.md`. Spec first: `docs/specs/workspaces.allium` (`EpicStaysInItsWorkspace`, `TaskEntersAnEpicTheCallerCanSee`, `WritesRequireSight`). Every module change needs its twin in `src/sync/memory_caller/` and a case in `tests/memory_caller_conformance/` run `with_workspace_rules()`.

## Findings

### 🟡 Medium — V2: re-parenting under your own private epic hides others' work (`spacetime/module/src/visibility.rs:190`)

**Issue:** `check_epic_move` (called from `tasks_epics.rs::patch_epic`; twin `src/sync/memory_caller/visibility.rs::Tables::check_epic_move`) checks only that the destination is visible and in the same workspace. Member M makes private epic P, then patches Y's public epic E (with running tasks) to `parent_epic_id = P`. E's creator and the hosts running its tasks lose sight; their hooks get "does not exist". E stays `visibility = "public"`, so `unseen_items_of` never lists it and no super-admin can recover it.

**Fix:** A move under a private destination (one whose `private_holder` is set) requires the authority `set_epic_visibility` requires on E, and that E's subtree holds nothing of anyone else's (`check_subtree_all_seen` for the would-be holder). Spec first.

### 🟡 Medium — V3: tasks pulled into a private epic, and a client-chosen owner (`spacetime/module/src/tasks_epics.rs:531`, `:587`)

**Issue:** (a) `move_task_row` / `update_task` let the holder of a private epic pull any task from a public epic into it. (b) A move to the main board (`epic_id = 0`) takes `owner` from the caller's argument: `set_task_epic(T, 0, "<anyone>")` takes a teammate's task, plants it on someone else, or strands it with `""`. The spec says it lands on the caller's board. `respawn_phoenix_successor` and `create_task` with epic 0 also trust the client's owner. Twins: `apply_set_task_epic`, `apply_update_task`.

**Fix:** Stamp `owner = caller_identity(ctx)` (operator exempt) on every path that sets it; apply V2's rule to a task moving into a private epic it is not already under.

### 🟡 Medium — V4: repo-group routing and flatten ignore visibility (`spacetime/module/src/grouping.rs:23`, `:44`, `:200`)

**Issue:** `find_or_create_repo_group` finds a repo-group sub-epic by title and origin only; its creator may make it private. Later `create_task` calls aimed at the public root then land in that private sub-epic and leave the creator's own view, so the caller never sees the row it waits for. `flatten_epic` moves a private sub-epic's tasks into the public root and deletes it without `check_subtree_all_seen`. Twin: `src/sync/memory_caller/grouping.rs`.

**Fix:** Refuse `set_epic_visibility` to private on `origin == repo-group` epics (simplest); also refuse `flatten_epic` over a subtree the caller cannot fully see.

### 🟢 Low — V8: cross-workspace learning rescope keeps the old workspace (`spacetime/module/src/learning.rs::rescope_epic_learnings`, `::rescope_learnings`)

**Issue:** Only `scope_ref` moves; the learning becomes visible only to people in both workspaces.

**Fix:** Set `workspace_id` from the target epic, as `move_epic_to_workspace` does.

### 🟢 Low — V9: unlinking checks no sight (`spacetime/module/src/packages.rs::update_work_package_with_links`)

**Issue:** A member who sees a public package can remove its link to an item they cannot see; `WritesRequireSight` names links.

**Fix:** Treat an unseen item in the unlink list as not linked (skip it silently, matching "unseen is missing").

## Decisions

### V2/V3: the rule for moving work under a private epic (`docs/specs/workspaces.allium`)

**Question:** What may a member move under a private epic they hold?

**Options:**
- (a) Only work whose whole subtree they created or hold, plus anything when they are the epic's creator-with-authority (admin or super-admin). Closes the hole; costs a subtree walk per move.
- (b) Nothing that is not already theirs: a private destination accepts only the caller's own epics and tasks. Simplest rule; an admin must first change visibility to move others' work.
- (c) Refuse every move into a private epic except at creation. Strictest; removes a workflow (tidying your own work under a private epic).

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/workspaces.allium` | Private-destination rule; caller-stamped owner |
| `spacetime/module/src/visibility.rs` | `check_epic_move` private-destination check |
| `spacetime/module/src/tasks_epics.rs` | Owner from caller; private-destination check on task moves |
| `spacetime/module/src/grouping.rs` | Refuse private repo groups; flatten sight check |
| `spacetime/module/src/learning.rs` | Rescope sets `workspace_id` |
| `spacetime/module/src/packages.rs` | Skip unseen items when unlinking |
| `src/sync/memory_caller/{visibility,tasks_epics,grouping}.rs` and learning/package twins | Mirror |
| `tests/memory_caller_conformance/` | One case per finding, with workspace rules on |
| `rules.rs` / `rules_gate.rs`, `module.wasm`, bindings | Bump together; rebuild; regenerate |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass, no `skipping:` lines
- [ ] Module tests pass
- [ ] New conformance cases fail before the change and pass after
- [ ] After merge to main: `./scripts/rotate-previous-store.sh`
