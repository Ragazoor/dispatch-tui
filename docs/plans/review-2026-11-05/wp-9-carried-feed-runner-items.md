# Carried feed-runner items

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Close three items carried across two or three reviews: the feed runner's wait for stdout EOF, blocking git calls in an async feed function, and the twin's discarded apply outcomes.

## Context

Standing items from `docs/plans/2026-11-05-codebase-review.md` §5 (raised 2026-11-03, 2026-11-04 and 2026-11-05). The user chose to give long-carried items their own package. Behaviour that changes a spec starts in `docs/specs/feeds.allium`.

## Findings

### 🟢 Low — feed runner waits for stdout EOF (`src/feed/exec.rs::run_feed_command_bounded`) — plausible

**Issue:** it waits for both pipes to close, then joins. A background child that inherits stdout (e.g. a helper the feed script starts with `&`) holds the pipe, so a valid emission becomes a timeout. Same class as keep-awake's T1, fixed in a7c40537 only there.

**Fix:** race `child.wait()` against the reads; after the child exits, read what is buffered with a short grace and kill the process group (the child runs under `setsid`). Test with a script that backgrounds `sleep 60` holding stdout and prints a valid emission.

### 🟢 Low — blocking git in `sync_emission` (`src/feed/cycle.rs::sync_emission`)

**Issue:** `resolve_feed_item_repo_paths` and `resolve_base_branches` run `git remote get-url` (bounded at 10 s each) inline inside an async function, parking a runtime worker per call.

**Fix:** run them in `spawn_blocking` (one task for the batch), or resolve once per path per cycle with a cache.

### 🟢 Low — `apply_update_epic` discards outcomes (`src/sync/memory_caller/tasks_epics.rs::apply_update_epic`)

**Issue:** the apply phase uses `?` and discards each part's outcome; safe only while the dry run matches the module's order exactly. A future divergence would apply half an update silently.

**Fix:** assert each apply outcome equals its dry-run outcome (debug assert or an error that names the part); a conformance case that would catch a reordering.

## Changes

| File | Change |
|------|--------|
| `src/feed/exec.rs` | wait on the child, not the pipes |
| `src/feed/cycle.rs` | git calls off the async worker |
| `src/sync/memory_caller/tasks_epics.rs` | check apply outcomes |
| `tests/` | background-child feed script; conformance case |

## Verification

- [ ] `./scripts/verify.sh` passes
- [ ] The background-child test fails before the fix
