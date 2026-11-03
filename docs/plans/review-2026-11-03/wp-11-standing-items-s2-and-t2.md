# Standing Items: Cached Done Check and Port Races

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Close two items carried for three or four review rounds: the service's last cache-based decision in `update_task`, and the bind-and-drop port pickers in the test harnesses.

## Context

Standing items from `docs/plans/2026-11-03-codebase-review.md` §5. Tasks #98747 and #98751 were closed Done without these; read their diffs before starting so you do not redo what they did. CLAUDE.md "Store Layering": a data rule belongs in the module's reducers, mirrored in `src/sync/memory_caller/` and covered in `tests/memory_caller_conformance.rs`. Tests must not use wall-clock sleeps (`scripts/check-no-test-sleep.sh`).

## Findings

### 🟢 Low — S2 residue: `close_pr_on_done` reads the subscription cache (`src/service/tasks/crud.rs::TaskService::update_task`)

**Issue:** `update_task` reads `prior` from the board's cache (`self.db.get_task`) to decide whether this update moves the task into Done and so whether to close its PR. Under a stale cache (another host already moved it, or the row just arrived), the decision differs from the store's. Raised 2026-10-22, 10-28, 10-31 and 11-03.

**Fix:** let the module's `update_task` reducer report whether it performed the Done transition (it already owns the stamp), and close the PR from that answer. If the reducer's return type cannot carry it, have the module write a request row that this host's reactor serves, like the PR state seam. Test: a stale cache that says "already Done" still closes the PR once.

### 🟢 Low — T2 residue: test harnesses bind a port and drop it (`tests/startup_smoke.rs::free_address`, `tests/managed_store_real.rs::free_address`, `src/runtime/tests/live_store.rs::try_start`)

**Issue:** each binds port 0, reads the port, drops the listener and hands the port to a child; another process can take it in between, giving flaky "address in use" failures. Raised 2026-10-28, 10-31 and 11-03. Also: `live_store.rs::try_start`'s `is_address_in_use` reads only stderr; check where `spacetime start` reports a bind error.

**Fix:** pass port 0 to the child and read the bound port back from its output or log where the child supports it; otherwise retry on address-in-use only (with the diagnostics R8 kept) in all three, sharing one helper.

## Changes

| File | Change |
|------|--------|
| `src/service/tasks/crud.rs` | PR close from the store's answer |
| `spacetime/module/src/tasks_epics.rs`, `src/sync/memory_caller/` | Report the Done transition (if needed) |
| `tests/startup_smoke.rs`, `tests/managed_store_real.rs`, `src/runtime/tests/live_store.rs` | Shared port helper |

## Verification

- [ ] New tests fail before the change and pass after
- [ ] `cargo test --no-fail-fast` green, with `spacetime` and `tmux` on PATH
- [ ] `cargo clippy --all-targets -- -D warnings` clean
