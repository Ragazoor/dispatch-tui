# free_address port races (standing item T2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove the last bind-and-drop port picks from the test harnesses, so a test store can never adopt another test's store.

## Context

Standing item T2 residue, raised 2026-10-28, 10-31, 11-03 and 11-04. Task #98860 (2026-11-03) closed Done after 6220f900 fixed `live_store.rs` only. The user chose (2026-11-04) to file it as its own package. **This task is not done until `grep -rn free_address src tests` shows no bind-and-drop picker left** (or each remaining use is shown to be immune and says why in a comment).

## Findings

### 🟡 Medium — T2: `free_address` binds and drops a port (`src/spacetime/tests/managed_store_real.rs::free_address`, `::on_free_address`; `src/runtime/tests/startup_smoke.rs`)

**Issue:** `free_address` binds `127.0.0.1:0`, reads the port and drops the listener; the store is started on that port later. Between the drop and the bind another test can take the port. The retry loops handle "spacetime lost the bind", but not another test's store winning the port and answering: the managed path's readiness probe then adopts the wrong store, and the test talks to someone else's data. `managed_store_real.rs::free_address` is used by `on_free_address` (retry on "in use") and directly by six fake-spawner tests; `startup_smoke.rs` uses it with a retry loop.

**Fix:** use the pattern 6220f900 used in `src/runtime/tests/live_store.rs::LiveStore::start`: start the store on port 0 (or let it pick) and read the bound port from its output. Where the managed path must be told an address up front, verify the store that answers is the one this test spawned (e.g. compare a per-test identity or the spawned pid) and fail loudly otherwise. The six fake-spawner tests that never bind a real port may keep a placeholder address; say so in a comment.

## Changes

| File | Change |
|------|--------|
| `src/spacetime/tests/managed_store_real.rs` | replace `free_address` with spawn-on-0 + read port, or identity check |
| `src/runtime/tests/startup_smoke.rs` | same |
| `docs/testing.md` | note the rule: never pick a port by bind-and-drop |

## Verification

- [ ] `grep -rn 'free_address' src tests` shows no bind-and-drop left (or each use is justified in a comment)
- [ ] `cargo test --no-fail-fast` green with `spacetime` on `PATH` and no `skipping:` lines; `cargo clippy --all-targets -- -D warnings` clean
- [ ] `./scripts/check-no-test-sleep.sh` green
