# Epic Service Writes

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Epic creation is one write, and the epic Done stamp is decided by the module, as it already is for tasks.

## Context

This work package addresses findings 15, 16 and 52 of the 2026-10-17 code
review (`docs/plans/2026-10-17-codebase-review.md`). Data rules belong in
`spacetime/module/src/`; mirror any module change in `src/sync/memory_caller/`
and cover it in `tests/memory_caller_conformance.rs`. Read the
`FieldUpdate`/`TaskPatch` paragraph in `docs/invariants.md` first.

## Findings

### 🟡 Epic Done stamp computed client-side (`src/service/epics.rs:394`)

**Issue:** `EpicService::update_epic` computes `completed_at = now` from the
cached `existing.status` and the board clock. A stale cache sends an explicit
stamp, which the module honours over its own, and it also overrides an
explicit `params.completed_at` (manual Done reorder). The task path already
moved this into the module (`TaskService::update_task`, `store-split.allium` X2).

**Fix:** Mirror the task path: leave `completed_at` out when entering Done and
let `patch_epic` stamp it (confirm the module does); forward
`params.completed_at` only when the caller named it; read the stamp back.

### 🟡 `create_epic` is create-then-patch (`src/service/epics.rs:205`)

**Issue:** The epic is created, then `sort_order`, `feed_command`,
`feed_interval_secs` are patched in a separate reducer call. A failure in
between leaves a half-configured epic and a retry duplicates it — the doc
comment claims this cannot happen.

**Fix:** Carry the fields in the create row: extend `encode::create_epic_row`
(which hard-codes `feed_command: ""`, `feed_interval_secs: 0`,
`sort_order: None`) and `EpicCrud::create_epic`. Check the module's
`create_epic` validates the interval. Fix the doc comment.

### 🟢 Low finding

- `src/store/queries/mod.rs` (`ingest_feed_inner`) — `created_by` uses
  `identity.user().await.ok().flatten().unwrap_or_default()`, silently
  stamping `""`. Use `require_identity` like every other create path, or
  document why empty is intended.

## Changes

| File | Change |
|------|--------|
| `src/service/epics.rs` | Done-stamp handling; single-write create; doc |
| `src/store/mod.rs`, `src/store/queries/` | `create_epic` signature; ingest identity |
| `src/sync/encode*` / `src/sync/writes.rs` | `create_epic_row` fields |
| `src/sync/memory_caller/` | Mirror if module behaviour changes |

## Verification

- [ ] Tests first: create with feed fields lands in one write (memory caller counts calls); stale-cache Done does not override the module stamp
- [ ] `cargo test --no-fail-fast` and the task's verify command pass
