# Live-Store CI Gate

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the live SpacetimeDB tests impossible to skip silently in CI, and get the SDK glue they exercise into the coverage figure.

## Context

This work package addresses findings T1 and T2 from `docs/plans/review-2026-10-08-followup/report.md`. Coverage fell from 91.56% (2026-08-28) to 86.16% (2026-10-08, floor 85). Earlier work (task from epic "Code Review: 2026-10-08", commit `8c820c83` "Cover the wiring layers without a live store") added fakes for some wiring; check what it covered before adding more.

## Findings

### ⚠️ Live-store skips are silent in CI (`tests/common/spacetime_instance.rs::spacetime_available_or_skip`, `src/spacetime/tests/managed_store_real.rs:361`)

**Issue:** `tests/tmux_harness/mod.rs::tmux_available_or_skip` hard-fails when `CI` is set. The spacetime helpers only `eprintln!` and return. If the "Install spacetime CLI" CI step breaks, `tests/spacetime_module.rs`, `tests/memory_caller_conformance.rs` (the `ConformanceIsCiGated` gate) and `managed_store_real.rs` all pass with nothing run.

**Fix:** Copy the tmux helper's CI hard-fail into both spacetime helpers. Keep the tarpaulin skip (the coverage job deliberately runs without `spacetime`). Check `docs/specs/` for the rule that describes the tmux behaviour and add the matching spacetime rule.

### ⚠️ SDK glue is never measured (`src/sync/sdk_connector/callers.rs`, `connect.rs::open_connection`, `wiring.rs`)

**Issue:** `spacetime_available_or_skip` returns false under `cfg!(tarpaulin)`, so the live tests never run in the coverage job. `callers.rs` (873 lines — every reducer write, including `claim_poll_owner`, `override_poll_owner`, `register_host`), `connect.rs::open_connection` and `wiring.rs::{wire_rows, wire_tables, wire_subtree_walk}` are reached only there. The logic is covered in memory; the glue is not.

**Fix:** Pick one and document it beside the floor in `.github/workflows/ci.yml`:
1. A separate coverage step that runs the live test files under `cargo llvm-cov` (not tarpaulin) with `spacetime` on `PATH`, merged into the uploaded report; or
2. Explicitly `--exclude-files` these three files from tarpaulin and re-calibrate the floor, so the figure stops pretending to measure them.

Prefer (1) if the merge is straightforward; otherwise (2). Ask the user if neither is clean.

## Changes

| File | Change |
|------|--------|
| `tests/common/spacetime_instance.rs` | Hard-fail under `CI` when `spacetime` is missing |
| `src/spacetime/tests/managed_store_real.rs` | Same |
| `.github/workflows/ci.yml` | Coverage for the live files, or explicit exclusion + floor note |
| `docs/testing.md` | Describe the new behaviour |
| `docs/specs/*.allium` | Rule for the CI hard-fail, beside the tmux one |

## Verification

- [ ] `CI=1 PATH=<without spacetime> cargo test --test spacetime_module` fails loudly
- [ ] Same without `CI` skips with the message
- [ ] `cargo test --no-fail-fast` — all pass
- [ ] CI coverage job green on the branch (or the exclusion is documented)
