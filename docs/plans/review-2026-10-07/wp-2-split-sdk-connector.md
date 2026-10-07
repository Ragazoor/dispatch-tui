# Split sdk_connector

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Split `src/sync/sdk_connector.rs` into focused modules and raise its coverage with the fake connector.

## Context

This work package addresses findings from the 2026-10-07 codebase review (`docs/plans/review-2026-10-07/report.md`, §2 and §4). It is a behaviour-preserving refactor plus tests; no spec change is expected. WP1 (remove SQLite) also touches `src/sync/rows.rs` and `src/sync/decode.rs` — not this file — so the two can run in parallel, but merge `main` before wrapping up.

## Findings

### 💡 God file (`src/sync/sdk_connector.rs`)

**Issue:** 2058 lines, of which ~1830 are production code (tests from ~1831): the connection loop, row decoding, reducer callers and reconnect logic in one file. It is the largest hand-written production file outside `src/db`. `create_managed_role_epic` has 7 parameters here and is restated in `sync/writes.rs`, `sync/memory_caller/mod.rs` and `spacetime/module/src/feed.rs`.

**Fix:** Split into a `sdk_connector/` directory by concern, following the shape `sync/memory_caller/` already has (e.g. `connect`, `subscription`, `callers`, `reconnect`). Move the tests to a sibling `tests` module. Consider a params struct for `create_managed_role_epic` shared by the caller trait and its implementations.

### 💡 Least-covered file on the live data path (`src/sync/sdk_connector.rs`)

**Issue:** 42% covered (147/344 lines, 197 uncovered) — the worst file in the crate. Most paths need a live server.

**Fix:** After the split, pull pure decision logic (decode, reconnect/backoff decisions, error mapping) into functions that take plain inputs, and test them directly or with the fake connector used by `d70f82c8` ("offline coverage for the SDK connector and caller").

## Changes

| File | Change |
|------|--------|
| `src/sync/sdk_connector.rs` | Becomes `src/sync/sdk_connector/mod.rs` plus per-concern submodules. Public API unchanged. |
| `src/sync/sdk_connector/tests.rs` (new) | Moved tests plus new tests for extracted decision logic. |
| `src/sync/writes.rs`, `src/sync/memory_caller/mod.rs` | Only if a `create_managed_role_epic` params struct is introduced. |
| `docs/module-map.md`, `docs/architecture.md` | Update any reference to `sdk_connector.rs`. Use `path::symbol` citations. |

## Verification

- [ ] `cargo clippy --all-targets -- -D warnings` clean
- [ ] `cargo test` — all pass
- [ ] `./scripts/check-doc-paths.sh` and `./scripts/check-doc-symbols.sh` pass
- [ ] Tarpaulin (`--engine llvm`, bindings excluded, `spacetime` off `PATH`): `sync/sdk_connector*` coverage above 42%; report the new figure
