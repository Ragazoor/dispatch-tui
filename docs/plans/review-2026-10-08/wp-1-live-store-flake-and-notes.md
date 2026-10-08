# Live-Store Test Flake and Test Notes

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the live-store tests stop failing on a one-off connection reset, document the triage tips, and tighten the coverage floor.

## Context

This work package addresses findings from the code review in `docs/plans/review-2026-10-08/report.md`.

## Findings

### 💡 Flaky live-store test (`tests/spacetime_module.rs`)

**Issue:** `drop_closed_retired_feed_items_drops_what_the_keep_set_omits` failed once with "Connection reset by peer" from `spacetime publish`, then passed. Plain `cargo test` stops at the first failing target, so later targets did not run.

**Fix:** Retry `spacetime publish` once on a transport error, or serialise publishes. Keep a real failure (non-transport) loud.

### 💡 CLAUDE.md test notes (`CLAUDE.md`)

**Issue:** Nothing tells agents to use `--no-fail-fast`, that this test can flake, that raw coverage includes generated bindings, or that `--db` names a directory.

**Fix:** Add short notes for each. Keep CLAUDE.md slim.

### 💡 Loose coverage floor (`.github/workflows/ci.yml`)

**Issue:** Hand-written coverage is 90.49%; floor is 84.

**Fix:** Re-measure with `cargo tarpaulin --engine llvm` and raise the floor deliberately (about 88), updating the comment in ci.yml and `docs/testing.md`.

## Changes

| File | Change |
|------|--------|
| `tests/spacetime_module.rs` | Retry publish once on transport error |
| `CLAUDE.md` | Add the four notes |
| `.github/workflows/ci.yml`, `docs/testing.md` | Raise floor, update comment |

## Verification

- [ ] `cargo test --no-fail-fast > out.txt 2>&1; echo $?` passes
- [ ] Run `tests/spacetime_module.rs` three times in a row
- [ ] Doc checkers pass (pre-push hook)
