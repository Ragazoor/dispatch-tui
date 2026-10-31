# Private Moves and Repo Groups

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** One rule for moving work under or out of a private epic, so no member can hide, take or expose another person's work by moving it.

## Context

Findings V2, V3, R2, V10, V11, V12 and the "Spec disagreements" list from `docs/plans/2026-10-31-codebase-review.md`. Spec first (`docs/specs/workspaces.allium`, `core.allium`, `epics.allium`). Every module change is mirrored in `src/sync/memory_caller/` (`visibility.rs`, `grouping.rs`, `tasks_epics.rs`) with a conformance case in `tests/memory_caller_conformance/`. Bump `RULES_VERSION` and `BOARD_RULES_VERSION` together when a rule changes.

## Findings

### 🔴 High — V2: re-parenting under your own private epic hides others' work (`spacetime/module/src/visibility.rs::check_epic_move`; `tasks_epics.rs::move_task_row` via `check_task_may_enter_epic`)

**Issue:** only "destination visible, same workspace" is checked. Alice patches Bob's public epic B to `parent_epic_id = Q` (her private epic): B's subtree vanishes for Bob and his hosts, B stays `public` so `unseen_items` never lists it, and the super-admin cannot recover it. Same for one task. Gets around `SetEpicVisibility`'s creator-or-admin rule.

**Fix:** refuse a move that changes an item's effective private holder unless the caller may set its visibility (creator, workspace admin, super-admin).

### 🟡 Medium — V3: repo-group routing and flatten ignore visibility (`spacetime/module/src/grouping.rs::find_or_create_repo_group`, `::route_to_repo_group`, `::flatten_epic`, `::regroup_epic`; `visibility.rs::set_epic_visibility`; `tasks_epics.rs::patch_epic` `origin`)

**Issue:** a repo-group sub-epic is found by title and origin only, and its creator may make it private. Carol's `create_task` on the root lands in Alice's private sub-epic and vanishes from Carol; `flatten_epic` moves Alice's private tasks onto the public root and deletes her sub-epic; `regroup_epic` moves everyone's tasks into it. `patch_epic` can change `origin`, getting around the repo-group re-parent refusal.

**Fix:** refuse `set_epic_visibility` and `origin` changes on `origin == "repo-group"` epics (or skip unseen sub-epics in routing and refuse flatten over them — see Decisions).

### 🟢 Low — R2: repo-group relinks keep the unseen mark (`spacetime/module/src/grouping.rs::relink_task`; twin `src/sync/memory_caller/grouping.rs::relink_task`)

**Issue:** `relink_task` changes `epic_id` without `clear_unseen_for_task`; it backs repo-change reroutes, `regroup_epic` and `flatten_epic`. Breaks `UnseenMarkEndsWithTheTaskLeavingItsEpicOrFinishing` (sibling of N1's feed fix, 2e523c8d).

**Fix:** clear inside `relink_task` in module and twin; add a conformance case.

### 🟢 Low — V10: cross-workspace task moves and learning rescope keep stale workspace data (`tasks_epics.rs::move_task_row`; `learning.rs::rescope_epic_learnings`, `::rescope_learnings`)

**Issue:** a task moved between epics in two workspaces keeps its package links and task-scoped learnings' `workspace_id`; rescope moves `scope_ref` only. Breaks `LinkedItemsShareThePackagesWorkspace`.

**Fix:** reuse `move_epic_to_workspace`'s link and learning cleanup, or refuse cross-workspace task moves.

### 🟢 Low — V11: unlinking checks no sight on the item (`spacetime/module/src/packages.rs::unlink_work_package_item`, unlink list of `update_work_package_with_links`)

**Issue:** a viewer of a public package can unlink an item they cannot see.

**Fix:** treat an unseen item as not linked.

### 🟢 Low — V12: the super-admin hatch deletes any private item (`SuperAdminDeletesUnseenItem`, module `visibility.rs`)

**Issue:** requires only `item_is_private`, so a super-admin can delete an active member's private epic.

**Fix:** also require that the creator is no longer a member (spec first).

### Spec disagreements (resolve in the spec, no code by themselves)

- D5 "a private epic hides its whole subtree" vs `core.allium` `private_holder` (nearest private ancestor).
- `SetEpicVisibility` guidance ("hands it to its creator alone") vs `WritesRequireSight` ("teammates may still act on each other's work").
- `workspaces.allium` open question (~line 988) says hook reducers and `ingest_feed` have no sight check; the code has them. Close it.

## Decisions

### The move rule (V2)

**Question:** what authority does a move under a private destination need?

**Options:** (a) the authority `set_epic_visibility` needs, on the moved item — simple, consistent; (b) (a) plus "the subtree holds nothing of anyone else's" — stricter, blocks moving shared work entirely; (c) refuse any move into a private epic of an item the caller did not create.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

### Repo groups (V3)

**Question:** forbid private repo-group sub-epics, or let routing/flatten skip unseen ones?

**Options:** forbidding is one check and removes the class; skipping keeps the feature but adds a check in three paths and can create duplicate sub-epics per viewer.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

### Private subtree semantics (spec disagreement, affects V2 and V12)

**Question:** nearest-holder rule or "a private epic hides its whole subtree"?

**Options:** nearest-holder keeps today's code but lets a sub-epic be visible under an invisible parent; whole-subtree is simpler to reason about but changes view bodies and `unseen_items_of`.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/workspaces.allium`, `core.allium`, `epics.allium` | Move rule, repo-group rule, subtree semantics, hatch rule, close stale open question |
| `spacetime/module/src/visibility.rs` | `check_epic_move`, `check_task_may_enter_epic`, `set_epic_visibility`, hatch |
| `spacetime/module/src/grouping.rs` | Routing/flatten/regroup rule; clear marks in `relink_task` |
| `spacetime/module/src/tasks_epics.rs` | `origin` patch refusal; cross-workspace task move cleanup |
| `spacetime/module/src/learning.rs`, `packages.rs` | Rescope `workspace_id`; unlink sight |
| `src/sync/memory_caller/` | Mirror each |
| `tests/memory_caller_conformance/` | One case per rule |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass
- [ ] `spacetime/module-tests` pass
- [ ] A conformance case per finding, each failing before the fix
- [ ] `cargo clippy --all-targets -- -D warnings` clean
