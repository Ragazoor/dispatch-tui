# Loss of sight: claims and marks together

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Every path that takes a task out of someone's sight releases their poll claims and settles their marks in one helper, and a detach no longer strands the agent that runs the task.

## Context

Findings A2, B1 and A8 from `docs/plans/2026-11-05-codebase-review.md` §3.2 and §3.3, plus the "graduate to a check" note in §5: the sibling-path class appeared a fourth run running, so the deliverable is one module helper (`on_sight_lost(ctx, tasks)` or similar) that releases claims *and* drops marks, called from every loss-of-sight path, with the conformance test listing every path. Behaviour changes start in the spec (`docs/specs/workspaces.allium: ClaimsFollowSight`, `core.allium: Bookmark`, `sync.allium`), then tests, then code. Mirror in `src/sync/memory_caller/`; bump both rules versions together.

## Findings

### 🟡 Medium — A2: fix for M11 (detach) → M10 and M12 come back (`spacetime/module/src/tasks_epics.rs::move_task_row`; `visibility.rs::task_visible_to`; `views.rs::hosted_tasks_of`)

**Issue:** Bob dispatched task T; his host holds T's PR-poll claim. An admin detaches T onto the admin's board (allowed since 92c4e19b). An epic-less task is visible to its owner only, so Bob loses sight: `hosted_tasks_of` filters by sight, his reactor's `get_task` answers None and the wrap-up rebase fails as "task deleted" (M12). `move_task_row` calls no `release_task_poll_claims`, so Bob's claim survives and nobody else may claim T: PR status stops (M10). A move into a private epic in the same workspace has the same claim gap (older).

**Fix:** release task-scoped claims after every task move (through the shared helper). For the held task, see Decisions. Add a detach arm and a task-move arm to the loss-of-sight conformance test.

### 🟡 Medium — B1: bookmarks and unseen marks survive most loss-of-sight paths (`spacetime/module/src/bookmark.rs::drop_marks_where`; `config.rs::release_claims_without_sight`)

**Issue:** `drop_marks_where` is called only from `workspace.rs::drop_follows_in_workspace` and `visibility.rs::move_epic_row`. `release_claims_without_sight` is also called from `visibility.rs` (`set_epic_visibility`), `tasks_epics.rs` (epic re-parent) and `feed.rs` (managed root made private), which drop no marks; task moves drop neither. Bob bookmarks T in epic E; Alice makes E private; Bob's row stays as an orphan he cannot remove (`set_bookmark(false)` answers "task does not exist"); when E is public again it reappears. `core.allium: Bookmark` says a row lives until "its person loses sight" and also that it "survives the task moving to another epic" — a move into an epic the person cannot see satisfies both.

**Fix:** settle the spec (Decisions), then one helper beside every `release_claims_without_sight` call and on task relink. Extend `every_loss_of_sight_path_releases_the_claims_it_strands` to assert marks too.

### 🟢 Low — A8: a detach leaves package links and task learnings in the old workspace (`tasks_epics.rs::move_task_row`) — plausible

**Issue:** the workspace-change arm does not treat epic → none as leaving the workspace, against `LinkedItemsShareThePackagesWorkspace`. Reachable by creators and admins since 92c4e19b.

**Fix:** treat a detach as leaving the workspace (unlink packages, move task learnings), as a cross-workspace move does.

## Decisions

### A2: the agent host of a detached task (`views.rs::hosted_tasks_of`)

**Question:** when a task with a live agent on another person's host is detached, what happens?

**Options:** (a) serve a task with a live agent or open request on the caller's host regardless of sight — the agent finishes normally; a small sight exception in the views; (b) refuse the detach while another person's host has a current agent or open request — simplest, but an admin cannot clean up a stuck task; (c) both: refuse unless the agent is gone, and serve hosted tasks regardless.

### B1: bookmark on lost sight (`docs/specs/core.allium: Bookmark`)

**Question:** when a person loses sight of a bookmarked task (epic made private, task moved out of view), does the bookmark end, or hide and come back when sight returns?

**Options:** (a) end — rows dropped on every loss-of-sight path; simple, matches claims; a bookmark is lost if visibility flips briefly; (b) hide and return — keep the row, let `set_bookmark(false)` clear an unseen row; no drop code, but orphan rows accumulate and the view must keep filtering.

Ask the user with `AskUserQuestion` before implementing these findings. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/workspaces.allium`, `core.allium`, `sync.allium` | task moves and detach in `ClaimsFollowSight`; bookmark lifetime; held-task rule |
| `spacetime/module/src/{config,bookmark,visibility,tasks_epics,feed,workspace,views}.rs` | one loss-of-sight helper; call it from every path; detach leaves the workspace |
| `src/sync/memory_caller/` | mirror |
| `tests/memory_caller_conformance/` | loss-of-sight test asserts claims and marks; detach and task-move arms |
| `spacetime/module/src/rules.rs`, `src/sync/rules_gate.rs` | bump together |

## Verification

- [ ] `./scripts/verify.sh` passes
- [ ] The loss-of-sight conformance test lists every caller of the helper; a new path without it fails
- [ ] `spacetime/module-tests` cases for detach and visibility flip
