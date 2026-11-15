# Managed store lifecycle: honest restarts, bounded commands, atomic publish

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** the board never reports a restart that did not happen, no store command can hang startup, overlapping launches cannot corrupt the shared module, and a stop reaches the whole server.

## Context

Findings M1–M4 from `docs/plans/2026-11-15-codebase-review.md` §3.7. M2 is the third timeout finding (H3, P1/P4, now M2), so it graduates into a lint. Spec first where behaviour changes (`docs/specs/startup.allium`). Test through the existing fakes in `src/spacetime/`; never run a bare `cargo run -- tui` (it adopts the real store).

## Findings

### 🟡 Medium — M1: a health-check restart can report success while the old, broken store keeps answering (`src/spacetime/managed_store.rs::ManagedStore::check_health` → `SpacetimeManagedStore::start_managed_store`) — decision item

**Issue:** a stop that times out still goes on to start (`startup.allium` allows it); the start loop probes the port before `try_wait`, so the old store answers while the new child has not yet failed to bind, and the start returns `Ok`. `StoreSpawner::listener_is_ours` exists but only `store_import` calls it. Also: `check_health` reads `stopped` once and calls stop and start directly, not through `stop_once` (an exit between them leaves a store nobody holds); and the health check reads `<board data dir>/managed-store.log`, so a store adopted from another `--data-dir` is never seen failing.

**Fix:** require `listener_is_ours(address, data_dir)` before `Ok`; re-check `stopped` under the lock before the start; see Decisions.

### 🟡 Medium — M2: store commands still run without a timeout (`RealProcessRunner::run` under `SpacetimeCliPublisher::publish`, `SpacetimeManagedStore::cli_identity`, `listening_pids`, `signal`; raw `Command::output()` in `listener_has_arg`)

**Issue:** a store that accepts connections but stalls hangs bring-up at "Starting the local store…" — the case 44d72032's comment names. `listener_has_arg` also fails closed: with `ss` missing every `store import --from` is "foreign".

**Fix:** `run_with_timeout` everywhere (generous for publish, mapped to `PublishFailed`); "cannot tell" distinct from "foreign". Graduate: `disallowed-methods` in `clippy.toml` for `std::process::Command::output` and `::status` (allowed only in `src/process.rs` via `#[allow]`), so the gate catches the next one. Mutation-test it.

### 🟢 Low-Medium — M3: the shared module file and hash record are not written atomically (`SpacetimeManagedStore::publish_embedded_module`; `FileModuleHashRecord::record_module_hash`) — plausible

**Issue:** `fs::write` of `dispatch_module.wasm` at a fixed shared path, then publish from it; `fs::write` of the hash. Two overlapping launches can publish truncated bytes (and the abort then stops the shared store under the other launch) or record a hash that misdescribes the store.

**Fix:** a per-process temp file for the wasm; `atomic_write` for the hash; an `flock` across publish-and-record.

### 🟢 Low — M4: the stop signals single pids with SIGTERM only (`SpacetimeManagedStore::stop`, `stop_spawned_store`) — plausible

**Issue:** the spawn sets `process_group(0)`, but the stop never signals the group or escalates. If `spacetime start` forks, the server is orphaned on the port.

**Fix:** `kill -TERM -<pgid>`, then SIGKILL at the deadline.

## Decisions

### M1: what does a health restart do when the stop did not complete?

**Question:** today a stop that times out still proceeds to start, which `startup.allium` allows. Should it?

**Options:** (a) report `RestartFailed` and leave the old store alone (the user restarts by hand) — honest, no auto-recovery; (b) escalate the stop (SIGKILL the group, M4) and then start — more often recovers, but kills a store that may be mid-write; (c) keep proceeding, but only report `Restarted` when `listener_is_ours` confirms the new child answers — smallest change, the old store may still hold the port.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/startup.allium` | the M1 decision; bounded commands |
| `src/spacetime/managed_store.rs` | M1–M4 |
| `src/process.rs` | allow-listed spawn site |
| `clippy.toml` | `disallowed-methods` |
| `src/spacetime/` tests | stale listener answers; timeouts |

## Verification

- [ ] `./scripts/verify.sh` passes
- [ ] Clippy fails on a re-introduced `Command::output()` outside `src/process.rs`
