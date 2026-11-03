# Feed Runner

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the feed runner respect host ownership for teardown and repo paths, settle how feed cycles are serialised across hosts, and make its errors and warnings useful.

## Context

Findings F1, F2, F3, F6, F7 from `docs/plans/2026-11-03-codebase-review.md` (fresh area `src/feed/`). Oracle: `docs/specs/feeds.allium`. Spec first, then tests that fail, then code. Coordinate with the "Feed Command Authority and Poll Claims" package (same epic), which changes who runs a feed; if both touch `src/feed/mod.rs`, ask that agent (ListAgents/SendMessage).

## Findings

### 🟡 Medium — F1: feed teardown ignores which host owns the task (`src/feed/mod.rs::cleanup_removed_feed_tasks`, `src/store/queries/mod.rs::confirm_removed`)

**Issue:** `RemovedFeedTask` carries no `host`. Host B (poll owner, or whoever pressed `r`) runs `teardown_task` for a task dispatched on host A, kills nothing of its own, and A's tmux window (with a live agent) and worktree outlive the deleted row. This bypasses the session seam (only the owning host tears down).

**Fix:** carry `host` in `RemovedFeedTask`; tear down locally only when it is this host; otherwise write a teardown request for the owner's reactor (see `docs/invariants.md`, session seam).

### 🟡 Medium — F2: feeds resolve repo paths against every host's clones (`src/feed/cycle.rs::FeedCycle::sync_emission`, `src/feed/mod.rs::FeedRunner::tick`, `src/dispatch/mod.rs::resolve_repo_path`)

**Issue:** the cycle uses `list_repo_paths()` (all hosts) where `IngestFeedEmission` says local clones and `list_repo_paths_saved_by_this_host` exists. A teammate's `/home/alice/Code/api` gets stamped on the task; dispatch from this host targets a missing path. Short-name matching also maps `orgA/api` and `orgB/api` to one clone.

**Fix:** use this host's list; match on the clone's `owner/repo` from its remote.

### 🟡 Medium — F3: feed cycles are serialised per process, but the spec promises per epic (`src/feed/guard.rs::FeedSyncGuard`; `feeds.allium: SerialisedFeedCycle`, `ManualFeedTrigger`)

**Issue:** manual refresh runs on non-owner hosts, and `ClaimAndProceed` proceeds before its claim lands, so two hosts can reconcile the same epic; an older emission from B can delete what A just inserted. The same spec rule still names `kill_on_drop`; the code kills the process group.

**Fix:** per the decision below; update the `kill_on_drop` guidance either way.

### 🟢 Low — F6: a failing feed with empty stderr reports nothing (`src/feed/exec.rs::exec_feed_command`)

**Issue:** on non-zero exit it returns `Err(stderr)`; `exit 1` or a quiet `jq` shows "Failed: " with no exit code.

**Fix:** include `output.status` in the message; test it.

### 🟢 Low — F7: a below-floor interval logs every 2 seconds (`src/feed/mod.rs::epic_due`)

**Issue:** the warning returns before `last_run` is bumped, so it logs every tick: ~43k lines a day per epic in an unrotated `app.log`. The spec says "EVERY tick".

**Fix:** spec first: warn once per epic per interval value (a set of warned epics); test it.

## Decisions

### How feed cycles are serialised across hosts (F3)

**Question:** should only one host at a time reconcile an epic?

**Options:**
- (a) A manual refresh on a non-owner host writes a request that the poll owner serves; the guard stays per process. Matches the spec's global promise; the refresh is slower and needs the owner online.
- (b) A manual refresh on a non-owner host runs additive-only (no deletes or moves). Simple; the user may see stale tasks until the owner polls.
- (c) Change the spec to say the guard is per host and accept the race. No code change beyond the doc.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/feeds.allium` | F3 per decision; F7 warn-once; `kill_on_drop` guidance |
| `src/feed/mod.rs` | F1 host-aware teardown; F7 warn-once |
| `src/store/queries/mod.rs` | `RemovedFeedTask.host` |
| `src/feed/cycle.rs` | F2 this-host repo paths |
| `src/dispatch/mod.rs` | `resolve_repo_path` matches `owner/repo` |
| `src/feed/exec.rs` | F6 exit status |
| `src/feed/guard.rs` | F3 per decision |

## Verification

- [ ] New tests fail before the change and pass after
- [ ] `cargo test --no-fail-fast` green
- [ ] `cargo clippy --all-targets -- -D warnings` clean
