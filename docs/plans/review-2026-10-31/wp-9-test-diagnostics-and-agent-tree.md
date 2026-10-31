# Test Diagnostics and Agent Tree

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Keep the editor pane alive on tree exit and make the live-store test helpers report real causes and avoid port races.

## Context

Findings R4, R7, R8 and standing item T2 from `docs/plans/2026-10-31-codebase-review.md`. Test real tmux only through `tests/tmux_harness/mod.rs` (private socket). No wall-clock sleeps in tests (`scripts/check-no-test-sleep.sh`).

## Findings

### 🟢 Low — R4: the close path still runs on every tree exit (`src/agent_tree/run.rs::DiffPaneTeardown`, `::tear_down_diff_pane`; `src/agent_tree/diff_pane.rs::mark_content`)

**Issue:** a failed editor mark is logged and swallowed; on `q` the teardown reads the lower pane as a diff and kills the editor with its unsaved edits. Breaks `EditorPaneOutlivesItsTree`. Residue of G1 (a29c0c51).

**Fix:** have `record_editor_took_over` set a flag that teardown honours, or treat a failed mark as "editor".

### 🟢 Low — R7: the N2 test copies its oracle (`spacetime/module/src/user_profile.rs`, test `every_format_character_and_line_separator_is_refused`)

**Issue:** its `REFUSED` table is a copy of `FORMAT_RANGES`, so a range missing from both passes.

**Fix:** an independent oracle, e.g. a Unicode general-category crate as a host-side dev-dependency (run `kognic-code-quality:dependency-review` first), checking every Cf code point is refused.

### 🟢 Low — R8: live-store start hides the real failure (`src/runtime/tests/live_store.rs::LiveStore::start`)

**Issue:** any early exit is retried five times and reported as "no store stayed up on a free port"; the child's stderr is never shown.

**Fix:** retry only when stderr shows address-in-use; otherwise fail with exit status and stderr.

### 🟢 Low — T2 residue (standing): port pick still races (`src/runtime/tests/startup_smoke.rs::free_address`; `src/spacetime/managed_store.rs` `stop(scan_port=true)`)

**Issue:** `free_address` binds and drops a port before the store uses it; a parallel test can take it. The scan-port stop can hit a sibling's store.

**Fix:** let the store pick port 0 and read it back, or retry on address-in-use as `LiveStore::try_start` does; scope the stop to the child's own pid.

## Changes

| File | Change |
|------|--------|
| `src/agent_tree/run.rs`, `diff_pane.rs` | Honour the editor take-over on teardown |
| `spacetime/module/src/user_profile.rs` (or a host-side test) | Independent oracle |
| `src/runtime/tests/live_store.rs` | Retry only on address-in-use; show stderr |
| `src/runtime/tests/startup_smoke.rs`, `src/spacetime/managed_store.rs` | Race-free port pick; narrow stop |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass
- [ ] New test: tree exit after a failed mark does not kill the editor pane
- [ ] `cargo clippy --all-targets -- -D warnings` clean
