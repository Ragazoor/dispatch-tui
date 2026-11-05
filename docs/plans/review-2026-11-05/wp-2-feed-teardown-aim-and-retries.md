# Feed teardown aim and request retries

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Nobody can aim another person's host at a worktree by creating a task, a failed request report is retried, and the setter-claims sweep keeps its own rules version.

## Context

Findings A1, A5 and A6 from `docs/plans/2026-11-05-codebase-review.md` §3.2. A1 is the sibling of 2026-11-04's Q2, fixed for patch/update only (7779c3f6, 130a519a). Behaviour changes start in the spec (`docs/specs/store-split.allium`, `feeds.allium`), then tests, then code. Every module change is mirrored in `src/sync/memory_caller/` with a case in `tests/memory_caller_conformance/`; bump `RULES_VERSION` (`spacetime/module/src/rules.rs`) and `BOARD_RULES_VERSION` (`src/sync/rules_gate.rs`) together when a reducer rule changes. Rebuild with `./scripts/build-managed-module.sh`.

## Findings

### 🔴 High — A1: `create_task` can aim another person's host at a live worktree (`spacetime/module/src/tasks_epics.rs::create_task`, `::create_task_signalled`; `feed_request.rs::teardown_request_owed`)

**Issue:** both reducers write `..row` with no `check_task_aim`. Bob sees Alice's feed epic and creates a task with `external_id = "x"`, `host = A1` (Alice's desktop) and `worktree = /home/alice/proj/.worktrees/999-live`. Alice's laptop A2 ingests; `x` is not in the feed, so the task is stale-deleted; the teardown is owed (host differs from the ingesting host; Alice owns A1). A1 runs `git worktree remove --force` and `git branch -D` on the live worktree.

**Fix:** on create, run `check_task_aim` against an empty row (so naming another person's host, a worktree or a window is refused), or blank `host`, `worktree` and `tmux_window` outright; refuse an `external_id` unless the caller acts for the epic's polling host. Same in the twin's create paths. Conformance case: Bob's create naming Alice's host is refused on both stores. Spec: extend `PatchTaskNeverAimsAnotherHostsTeardown` (or its create twin) in `store-split.allium`.

### 🟡 Medium — A5: a failed report leaves a request `running` until the board restarts (`src/feed/requests.rs::serve_teardowns`, `::rows_to_serve`; `src/feed/mod.rs::spawn_refresh_request`; `spacetime/module/src/feed_request.rs::validate_feed_request_claim`)

**Issue:** after a failed done/failed report, `forget` drops the id; the next tick re-claims; the module refuses inside the 10-minute re-claim grace; the board reads `Ok(false)`, puts the id back in `served`, and `retain` keeps it while the row is open. The process never retries, even after the grace. Open rows are not pruned.

**Fix:** keep a pending-report set and retry the report (not the claim) on the next tick; or `forget` on a grace refusal so the claim is retried after the grace. Test with a caller whose first report fails.

### 🟢 Low — A6: `FEED_SETTER_CLAIMS_RULES_VERSION` moved 37 → 38 (`spacetime/module/src/rules.rs`)

**Issue:** 70007f70 added the sweep at 37; 8376c7e9 bumped it with `RULES_VERSION`, apparently by replace-all. Stores on 37 re-run the sweep (idempotent today); the constant's doc ("the version that added the sweep") is false, and the next mechanical bump may move it again.

**Fix:** restore 37; add a test asserting `FEED_SETTER_CLAIMS_RULES_VERSION == 37` and `WORKSPACES_RULES_VERSION` at its value, with a comment that these never move.

## Changes

| File | Change |
|------|--------|
| `docs/specs/store-split.allium` | create may not aim another person's host |
| `spacetime/module/src/tasks_epics.rs` | aim check (or blanking) in both create reducers |
| `src/sync/memory_caller/` (task create) | mirror |
| `tests/memory_caller_conformance/` | create-aim case |
| `src/feed/requests.rs`, `src/feed/mod.rs` | retry a failed report |
| `spacetime/module/src/rules.rs` | restore the constant; version bump for A1 |
| `src/sync/rules_gate.rs` | matching bump |

## Verification

- [ ] `./scripts/verify.sh` passes
- [ ] Conformance case fails before the fix on both stores
- [ ] `spacetime/module-tests` case for A1 (docs/testing.md, "SpacetimeDB module and CI details")
