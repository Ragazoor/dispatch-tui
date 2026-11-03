# Store Publisher and Restore Authority

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make restore work on stores published before the seed gate, and stop any board other than the publisher from taking the publisher role or calling restore-only reducers.

## Context

Findings M1 and M7 from `docs/plans/2026-11-03-codebase-review.md` (M1 is "fix for V1 → regression"; V1 was fixed in fb7b2c05). Spec first: `docs/specs/spacetime-seed.allium` (Operator = the store publisher) changes before code, then tests that fail, then code. Every module change is mirrored in `src/sync/memory_caller/` and covered in `tests/memory_caller_conformance/`. A reducer signature change bumps `RULES_VERSION` (`spacetime/module/src/rules.rs`) and `BOARD_RULES_VERSION` (`src/sync/rules_gate.rs`) together; rebuild `module.wasm` and bindings per CLAUDE.md.

## Findings

### 🔴 High — M1: restore is broken on existing stores, and the publisher role goes to whoever claims it first (`spacetime/module/src/lib.rs::claim_store_owner`, `::init`; `src/spacetime/managed_store.rs::SpacetimeCliPublisher::publish`; `src/cli/store_import.rs::import_store`)

**Issue:** `init` records the publisher in `store_owner` only on a brand-new database, so every store published before fb7b2c05 has no `store_owner` row. Nothing in `src/` or `scripts/` calls `claim_store_owner`; only its doc comment says to run `spacetime call <db> claim_store_owner` by hand. (a) `dispatch store import` into the existing managed store fails with "only the store's publisher may restore (seed) data", with no hint at the fix. (b) `claim_store_owner` checks no caller, so on a shared store the first board that calls it becomes the publisher, then calls `seed_user_profiles` to make itself super-admin and `seed_memberships` to admin every workspace.

**Fix:** have the publish/bring-up path call `claim_store_owner` as the same CLI login straight after `spacetime publish`, treating "already has a recorded publisher" as success. Check whether SpacetimeDB exposes the database owner identity to a reducer and, if so, gate `claim_store_owner` on it (note `ctx.identity()` is the module's own identity, not the publisher's). If it does not, document the window in the spec. Point the import's refusal message at the fix. Add a test that imports into a store published by a module without a `store_owner` row, and a refusal test for a second claimant.

### 🟢 Low — M7: sibling restore reducers are ungated (`spacetime/module/src/sequence.rs::burn_id_sequence`, `::probe_generated_task_id`; `spacetime/module/src/lib.rs::set_schema_version`)

**Issue:** any board can burn an id sequence (a large `ceiling` stalls the transaction) or rewrite `schema_version`.

**Fix:** gate each on `require_restore_authority`, with one refusal test per reducer; mirror in the twin.

## Changes

| File | Change |
|------|--------|
| `docs/specs/spacetime-seed.allium` | How the publisher is recorded on an old store; who may claim; restore-only reducers listed |
| `spacetime/module/src/lib.rs` | Gate `claim_store_owner`; gate `set_schema_version` |
| `spacetime/module/src/sequence.rs` | Gate `burn_id_sequence`, `probe_generated_task_id` |
| `src/spacetime/managed_store.rs` | Call `claim_store_owner` after publish |
| `src/cli/store_import.rs` | Refusal message names the fix |
| `src/sync/memory_caller/` | Mirror the gates |
| `tests/memory_caller_conformance/`, `tests/spacetime_module/` | Refusal cases; import into an old store |

## Verification

- [ ] New tests fail before the change and pass after
- [ ] `cargo test --no-fail-fast` green (with `spacetime` on PATH so live-store tests run)
- [ ] Module checks per `docs/testing.md` ("SpacetimeDB module and CI details"); `module.wasm` and bindings rebuilt
- [ ] `cargo clippy --all-targets -- -D warnings` clean
