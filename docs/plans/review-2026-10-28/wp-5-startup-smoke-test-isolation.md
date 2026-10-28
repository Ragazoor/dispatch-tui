# Startup Smoke Test Isolation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Stop a dropped runtime's background tasks from outliving it, make test port picking safe, and settle whether CI enforces the previous-store gate.

## Context

This work package addresses findings T1, T2 and T3 from `docs/plans/2026-10-28-codebase-review.md`. Read `docs/testing.md` ("startup smoke tests", the no-wall-clock-sleep rule).

## Findings

### 🟡 Medium — T1: the first board keeps running into the second launch (`src/runtime/tests/startup_smoke.rs:143-202`)

**Issue:** `a_fresh_install_starts_against_a_real_store` calls `drop(first)` (line 176) and launches again on the same address and data directory. `src/runtime/mod.rs::start_background_tasks` (around line 932) drops the handles of the row pump and the shared-store reconnect loop, detaching them; nothing implements `Drop` for `Bootstrap` or `TuiRuntime`. The first board's reconnect loop can attach to the second store.

**Fix:** Keep `AbortHandle`s for the detached tasks on the runtime and abort them on drop (production exit then also stops them cleanly). Add a test that a dropped runtime's tasks are finished.

### 🟢 Low — T2: port picking races (`src/runtime/tests/startup_smoke.rs:33-38`, `live_store.rs:45-48`)

**Issue:** `free_address` and `LiveStore::start` bind port 0, drop it, then hand out the port. `LiveStore::start` probes before checking its own child, so it can accept another test's store, and `managed_store.rs:862` `stop(scan_port=true)` SIGTERMs every PID on the port.

**Fix:** Check `child.try_wait()` before trusting the probe; retry with a new port on port-in-use.

### 🟢 Low — T3: the previous-store gate never runs in CI (`scripts/check-previous-store.sh:28-37`)

**Issue:** It exits 0 with no local `main` or in a shallow clone, which is what CI checks out. Only a developer's pre-push hook runs it.

## Decisions

### T3: enforce in CI or document (`scripts/check-previous-store.sh`)

**Question:** Should CI enforce the previous-store gate?

**Options:**
- (a) Fetch `main` (with enough depth) in the CI job that runs the script. Costs a slower checkout and a CI change.
- (b) Keep the skip and say plainly in `docs/testing.md` that CI does not enforce it. Costs nothing; the gate stays local-only.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `src/runtime/mod.rs` | Keep abort handles; abort on drop |
| `src/runtime/tests/startup_smoke.rs` | Rely on the drop abort; safer `free_address` |
| `src/runtime/tests/live_store.rs` (or wherever `LiveStore` lives) | `try_wait` before probe; retry |
| `.github/workflows/ci.yml` or `docs/testing.md` | Per T3 decision |

## Verification

- [ ] T1 test fails first, then passes
- [ ] `cargo test --no-fail-fast` with `tmux` and `spacetime` on `PATH`
- [ ] `./scripts/check-no-test-sleep.sh`, `cargo fmt --check && ./scripts/check-doc-paths.sh`
