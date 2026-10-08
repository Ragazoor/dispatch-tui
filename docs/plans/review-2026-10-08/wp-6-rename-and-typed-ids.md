# Rename Database and --db; Typed Ids

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make names match the store that exists and use TaskId/EpicId in signatures.

## Context

This work package addresses findings from the code review in `docs/plans/review-2026-10-08/report.md`.

## Findings

### 💡 `Database` router and `--db` flag (`src/store/mod.rs`, `src/lib.rs::default_db_path`, `docs/reference.md`)

**Issue:** `Database` keeps no data; `--db`/`DISPATCH_DB` names a directory and the file is never opened.

**Fix:** Ask the user before renaming public flags. Minimum: say "data directory" in flag help. Preferred: add `--data-dir`/`DISPATCH_DATA_DIR`, keep `--db` as a deprecated alias, rename `Database` to `Store`. Spec first (`storage.allium`).

### 💡 Bare `i64` ids (116 non-test signatures)

**Issue:** `task_id: i64`, `epic_id: i64` although `TaskId`/`EpicId` exist (e.g. `subscribed_epics -> Vec<i64>`).

**Fix:** Convert at the service boundary first, then outward. Do it in small commits.

## Changes

| File | Change |
|------|--------|
| `src/store/mod.rs`, `src/lib.rs`, `src/cli/` | Rename, alias |
| `docs/reference.md`, `CLAUDE.md`, `docs/specs/storage.allium` | Update |
| service and handler signatures | Typed ids |

## Verification

- [ ] `cargo test` and clippy pass
- [ ] Old `--db` still works (alias test)
