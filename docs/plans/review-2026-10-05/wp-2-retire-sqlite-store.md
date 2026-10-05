# Retire the SQLite Store

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Record what `src/db` still owns, then delete the SQLite paths that production no longer reads.

## Context

This work package addresses findings from the code review in `docs/plans/review-2026-10-05/report.md`.

## Findings

### ⚠️ Two stores, one live (`src/runtime/mod.rs:396`, `src/main.rs:607`)

**Issue:** `Database::open` is still called, but `docs/reference.md` says live data is in SpacetimeDB and `tasks.db` is stale. `src/db` is about 20k lines incl. tests (4.2k in `queries/`). `db/queries/learnings.rs` is 35% covered (144 uncovered lines).

**Fix:** First, find what production still reads from SQLite (import source, settings, tests?) and write it down. This needs a decision; if it is ambiguous, stop and ask the user. Then delete the dead paths, learnings queries first. Follow spec-first order: update the spec, then tests, then code.

### 💡 `storage.allium` describes a backend that no longer holds the board (`docs/specs/storage.allium:1`)

**Issue:** It scopes "the local store" and journal modes.

**Fix:** Retitle or trim it to what SQLite still does. Update `docs/reference.md` and `docs/invariants.md` to match.

## Changes

| File | Change |
|------|--------|
| `docs/specs/storage.allium` | Trim to surviving behaviour |
| `src/db/queries/learnings.rs` and callers | Remove if unused in production |
| `src/db/mod.rs`, `src/runtime/mod.rs`, `src/main.rs` | Remove dead open/attach paths |
| `docs/reference.md` | State what SQLite still stores |

## Verification

- [ ] Run existing tests — all pass (`cargo test > /tmp/t.txt 2>&1; echo $?`)
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] Doc checkers pass
