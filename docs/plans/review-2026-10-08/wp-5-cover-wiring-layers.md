# Cover the Wiring Layers

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Raise coverage on the SDK connector wiring and callers, runtime/mod.rs and main.rs using fakes.

## Context

This work package addresses findings from the code review in `docs/plans/review-2026-10-08/report.md`.

## Findings

### 💡 Uncovered wiring (`src/sync/sdk_connector/wiring.rs` 0/74, `src/sync/sdk_connector/callers.rs` 11/91, `src/runtime/mod.rs` 413/603, `src/main.rs` 133/241)

**Issue:** These are only exercised by live-store tests, which skip under tarpaulin.

**Fix:** Add a fake table/caller seam so decision logic runs without a live store; add tests for the `runtime/mod.rs` bootstrap branches and `main.rs` dispatch. If a part is live-only, say so in `docs/testing.md`. Spec first for any behaviour change (`sync.allium`).

## Changes

| File | Change |
|------|--------|
| `src/sync/sdk_connector/{wiring,callers}.rs` | Seam for fakes |
| `src/sync/tests/` | New tests |
| `src/runtime/`, `src/main.rs` | New tests |

## Verification

- [ ] `cargo tarpaulin --engine llvm --out stdout` shows the files above improved
- [ ] `cargo test` passes
