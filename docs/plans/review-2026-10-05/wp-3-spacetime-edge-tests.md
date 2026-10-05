# Tests for the SpacetimeDB Edge

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Cover the least-tested glue around the shared store with fake connectors and spawners.

## Context

This work package addresses findings from the code review in `docs/plans/review-2026-10-05/report.md`.

## Findings

### 💡 `sync/sdk_connector.rs` is 26% covered (`src/sync/sdk_connector.rs`)

**Issue:** 256 of 344 lines uncovered; most paths need a live server.

**Fix:** Cover decode and reconnect decisions with the fake connector.

### 💡 `cli/store_import.rs` is 13% covered (`src/cli/store_import.rs:135`)

**Issue:** The probe, spawn, wait loop (`thread::sleep`) and printing are untested. Confirm the sleep does not run on the async runtime.

**Fix:** Add a fake-spawner test (`StoreSpawner` trait).

### 💡 `cli/agent_diff.rs` 67%, `cli/mod.rs` 60%, `main.rs` 53%

**Fix:** Add tests for the uncovered branches that are reachable without a live server.

## Changes

| File | Change |
|------|--------|
| `src/sync/tests/` | New fake-connector tests |
| `src/cli/` tests | Fake spawner test for `store_import`; branch tests for `agent_diff` |

## Verification

- [ ] Run existing tests — all pass
- [ ] Tarpaulin (with `spacetime` off `PATH`) shows these files up from 26% / 13% / 67%
