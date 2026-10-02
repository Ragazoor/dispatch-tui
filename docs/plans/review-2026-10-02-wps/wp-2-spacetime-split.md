# SpacetimeDB Twin Split and Typed Boundary

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Split the two god files by domain, share the task row blanks, and type the `ReducerCaller` boundary.

## Context

Findings from `docs/plans/review-2026-10-02/report.md`, sections 1 and 4. Spec first, then tests, then code. Regenerate bindings and rebuild the managed module if module code changes (`./scripts/build-managed-module.sh`).

## Findings

### ⚠️ God files (`spacetime/module/src/lib.rs`, `src/sync/memory_caller.rs`)

**Issue:** 4463 and 4598 lines. memory_caller has a 60-method impl at :1318-2600 and ~2000 inline test lines from :2603. The module has banners at :147, 670, 776, 808, 1099, 1425.

**Fix:** Directory modules split by domain (tasks/epics, learnings, feed, agent state, config); move tests to `tests/`. Split along the same lines in both.

### ⚠️ Duplicated row literal (`memory_caller.rs:988,1033`, `spacetime/module/src/lib.rs:926,976`, `src/sync/tests/decode.rs:490`, `memory_caller.rs:2647`)

**Issue:** ~40-field row restated in 4+ places.

**Fix:** Make module `blank_*` public or add `Default`; call from the others.

### 💡 Primitive obsession on `ReducerCaller` (`src/sync/writes.rs:108-370`)

**Issue:** raw `i64` ids, `String` timestamps and `sub_status`.

**Fix:** `EpicId`/`TaskId`, a timestamp type, status enums; encode only in `encode.rs`. Also `enum PollScope` (`board_reads.rs:52`, `rows.rs:587`, `memory_caller.rs:1270,1306`) and `FromStr` for verdict/clear-mode (`memory_caller.rs:1947,2166`).

### 💡 sdk_connector boilerplate and long fns (`src/sync/sdk_connector.rs`)

**Issue:** ~40 near-identical `awaiting_answer` blocks (:856, 888, 944, 1077, 1192, 1222, 1239, 1274, 1415, 1457, 1578); `abandon` ~151 lines, `wire_rows` ~126, `connect` ~84; ~15 silent `let _ = tx.send`.

**Fix:** Sibling macros, split functions, a logging `fire(tx, v)` helper.

### 💡 Dead scaffolding and duplicated parsing

**Fix:** Delete `is_complete`, `covered_domain_count`, `COVERED_DOMAINS` (`memory_caller.rs:139`). One `normalize_server` helper for `spacetime/managed_store.rs:117` and `src/startup.rs:208`.

## Changes

| File | Change |
|------|--------|
| `spacetime/module/src/lib.rs` | Split by domain |
| `src/sync/memory_caller.rs` | Split by domain, move tests |
| `src/sync/writes.rs`, `encode.rs` | Typed boundary |
| `src/sync/sdk_connector.rs` | Macros, split fns |
| `src/sync/board_reads.rs`, `rows.rs` | `PollScope` |

## Verification

- [ ] `cargo test`, `cargo clippy --all-targets -- -D warnings`
- [ ] `./scripts/check-spacetime-module.sh`; bindings and module wasm stamp up to date
