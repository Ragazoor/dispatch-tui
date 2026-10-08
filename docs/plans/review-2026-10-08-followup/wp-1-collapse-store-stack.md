# Collapse the Store Stack

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove the forwarding layer between the store domain traits and the `sync` ports, make the ports non-optional, and give the TUI one card read path.

## Context

This work package addresses findings A1, A2, A6 and the store/sync half of A3 from `docs/plans/review-2026-10-08-followup/report.md`. It is the largest package; consider writing a short plan of the trait layout and getting it reviewed before moving code. Behaviour must not change — this is a structural refactor. Read `docs/invariants.md` ("Mutation boundary", "board read source", "one store with routed ports") and `docs/specs/storage.allium` / `sync.allium` first; update any spec rule that names a removed trait.

## Findings

### ⚠️ Three-layer store stack for one backend (`src/store/mod.rs`, `src/store/queries/tasks.rs`)

**Issue:** `src/store/mod.rs` declares 14 domain traits (`TaskRead`, `TaskCrud`, `EpicRead`, `EpicCrud`, `SettingsStore`, `RepoConfig*`, `HostStore`, `IdentityCredentialStore`, `SubscriptionStore`, `LearningStore`, `LearningRetrievalStore`, `PollOwnershipStore`, `UsageStore`). Each has one production impl, `Store`, which only forwards (e.g. `TaskRead for Store` is `self.shared_reader()?.get_task(id)`). The calls then go through 5 port traits (`SharedReader`, `SharedWriter`, `SharedLearningReader`, `SharedUsageReader`, `SharedRetiredFeedItemReader`), each with one production impl in `src/sync/` over the same `SharedRows` (`runtime::StoreParts::build`). Every store method is written about three times.

**Fix:** Keep the narrow domain traits (the services depend on their narrowness) and implement them directly on the `sync` adapters. Drop the port traits and the forwarding bodies. Keep the in-memory test double working through the same traits.

### ⚠️ `Store` ports are `Option` (`src/store/mod.rs::Store`, `src/runtime/mod.rs::placeholder_database`)

**Issue:** `Store` holds five `Option<Arc<dyn …>>` ports. `with_shared_store` sets all five together, yet `runtime::placeholder_database` builds `Store::unattached()` in production. A missing port only surfaces at runtime as "no shared store attached".

**Fix:** If `Store` survives A1, hold one non-optional ports bundle. Restrict unattached handles to `cfg(test)`/`test-support`, and remove the placeholder path in `runtime` (or replace it with a type that cannot serve reads).

### 💡 Board reads through two traits to one object (`src/sync/board_reads.rs`, `src/runtime/mod.rs::TuiRuntime`)

**Issue:** `TuiRuntime.database` and `TuiRuntime.board_reads` both end at `SubscriptionBoardReads`; `get_task`, `list_epics` and others exist on both `BoardReads` and `SharedReader`. Which one a TUI caller uses matters for redraw tracking, but only convention enforces it.

**Fix:** Make `BoardReads` the TUI's only card read path; remove the duplicate methods from the other trait (or make one extend the other).

### 💡 Stale SQLite comments in store/sync (`src/sync/mod.rs`, `src/store/**`, `src/sync/board_reads.rs`, `src/runtime/mod.rs::bootstrap_with`)

**Issue:** Comments still describe SQLite as live: `src/sync/mod.rs` ("the board still reads and writes its local store … a later change"; lists 3 of 16 submodules), `src/store/queries/tasks.rs` ("local SQLite fallback below", "`INSERT OR IGNORE`"), `src/store/mod.rs::SharedWriter` ("the SQLite branch remains…"), `src/sync/board_reads.rs` ("SQLite's cumulative change counter"), `src/runtime/mod.rs::bootstrap_with` ("tests hand them SQLite… until Phase 12b").

**Fix:** Rewrite these as part of the refactor (the code they describe is moving anyway). WP10 handles stale comments outside `src/store`, `src/sync` and `src/runtime/mod.rs`.

## Changes

| File | Change |
|------|--------|
| `src/store/mod.rs` | Remove port traits and `Option` ports; keep narrow domain traits |
| `src/store/queries/*.rs` | Delete forwarding impls (or move bodies to `sync` adapters) |
| `src/sync/**` | Implement domain traits directly; update module docs |
| `src/runtime/mod.rs` | `StoreParts::build`, `placeholder_database`, `TuiRuntime` read fields |
| `docs/invariants.md`, `docs/module-map.md`, `docs/specs/storage.allium`, `docs/specs/sync.allium` | Update to the new trait names |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass (with `spacetime` on `PATH` so live tests run)
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] All `scripts/check-doc-*.sh` pass
- [ ] `grep -rn 'SQLite' src/store src/sync src/runtime/mod.rs` returns only intentional historical notes
- [ ] `allium:weed` on `storage.allium` and `sync.allium` reports no new drift
