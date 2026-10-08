# Split tui/mod.rs and setup/mod.rs

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Split the two largest hand-written files by concern.

## Context

This work package addresses findings from the code review in `docs/plans/review-2026-10-08/report.md`.

## Findings

### 💡 Large mixed-concern files (`src/tui/mod.rs`, `src/setup/mod.rs`)

**Issue:** `tui/mod.rs` (~2300 lines) mixes the `App` state with view-building; `setup/mod.rs` (~1970) mixes several setup steps.

**Fix:** Split into submodules by concern, keeping public paths via re-exports so callers do not change. Read `docs/invariants.md` (layout-cache coherence) before touching `tui/mod.rs`. Update `docs/module-map.md`.

## Changes

| File | Change |
|------|--------|
| `src/tui/mod.rs` | Extract view-building and helpers into submodules |
| `src/setup/mod.rs` | Extract setup steps into submodules |
| `docs/module-map.md` | New rows |

## Verification

- [ ] `cargo test` passes; snapshots unchanged
- [ ] Clippy and doc checkers pass
