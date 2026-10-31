# View Retire

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Keep a row on the board while any view still serves it, before two views feed one map.

## Context

This work package addresses finding Y1 from `docs/plans/2026-10-31-codebase-review.md`. It must land before task #98691 wires `own_*` and `covered_*` into the same maps; check that task's state with `get_task` first and message its agent if it is running.

## Findings

### 🟡 Medium (plausible, latent) — Y1: a row served by two views is erased when it leaves one (`src/sync/rows.rs::SharedRows::retire_task`, `::retire_epic`; `src/sync/sdk_connector/wiring.rs`)

**Issue:** Retire removes the row when the held row equals the retracted one, without knowing which view sent the delete (`wire_view!`). Once `own_epics` and `covered_epics` (or the task pair) feed one map, unfollowing an epic sends a delete from `covered_epics` only, and the board drops an epic the caller created inside it that `own_epics` still serves.

**Fix:** Track which views hold each row (a per-view id set, or a reference count keyed by view) and remove the row only when none does. Update ordering stays as it is (already correct for an update pair in either order). Spec first: add "a row that leaves one view and stays in another stays on the board" to `docs/specs/sync.allium` near `AViewUpdateNeverErasesTheRow`, then a test in `src/sync/` that fails today when two views are wired into one map.

## Changes

| File | Change |
|------|--------|
| `docs/specs/sync.allium` | Two-view retire rule |
| `src/sync/rows.rs` | Per-view membership on retire |
| `src/sync/sdk_connector/wiring.rs` | Pass the view identity to retire |
| `src/sync/` tests | Leave-one-view-stay-in-another case |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass, no `skipping:` lines
- [ ] The new test fails before the change and passes after
