# Sync Reconnect and Subscriptions

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** A board never sits "Connected" with a missing or stale shared view.

## Context

This work package addresses findings 13, 14 and 50 of the 2026-10-17 code
review (`docs/plans/2026-10-17-codebase-review.md`). Spec:
`docs/specs/sync.allium`. Finding 14 is plausible only — prove it with a
live-store test before changing code.

## Findings

### 🟡 "Connected" with an empty view after a post-accept error (`src/sync/session.rs:422`)

**Issue:** `SyncSession::attempt` applies `ConnectionEvent::Accepted`, then
`?` on `adopt_user_identity_with_credential`, `subscribed_epics` and
`build_subscription_request`. On error the status is `Connected` but
`self.identity` is empty and nothing is subscribed; the next tick goes to
`reassert_on_shrink`, which never subscribes. The board stays empty until
restart.

**Fix:** On any `Err` after `Accepted`, apply `Dropped`, call
`connector.disconnect()`, and return `Failed` so backoff retries. Keep the
`StopOnAUserIdentityConflict` ordering the comment describes.

### 🟡 Unfollowing one epic can drop depth-2+ rows of another (`src/sync/session.rs` `reassert_on_shrink`, `src/sync/sdk_connector/mod.rs` `reset_subtree`, `src/sync/sdk_connector/connect.rs` `subscribe`)

**Issue:** `subscribe` resets every widening handle, then re-requests deeper
levels only from `on_insert` callbacks. Rows already in the SDK cache fire no
insert, so a still-followed epic's sub-epic tasks can vanish until reconnect.
*Plausible.*

**Fix:** First write a live-store test (`tests/spacetime_module.rs` style):
follow A (with sub-epic S holding tasks) and B, unfollow B, assert S's tasks
remain. If it fails, re-seed `follow_queries`/`widen_queries` from the cached
epics after a replacing subscribe.

### 🟢 Low finding

- `src/sync/sdk_connector/connect.rs` (`disconnect`) with `install` — late
  callbacks from the dead connection can repopulate `SharedRows` after
  `rows.clear()`, and `install` does not clear again. Always clear in
  `install`, or generation-tag callbacks.

## Changes

| File | Change |
|------|--------|
| `src/sync/session.rs` | Post-accept error path |
| `src/sync/sdk_connector/mod.rs`, `src/sync/sdk_connector/connect.rs` | Re-seed widenings (if proven); clear on install |
| `tests/spacetime_module.rs` or `src/sync/tests/` | Live-store and session tests |

## Verification

- [ ] Test first: a fake store whose adopt fails after accept → status returns to a retrying state and a later attempt subscribes
- [ ] Live-store test for finding 14 (fails before, passes after — or documents it does not reproduce)
- [ ] `cargo test --no-fail-fast` and the task's verify command pass
