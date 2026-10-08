# Layering Fixes and Service Wiring

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove upward/sideways module dependencies, disambiguate the four "store" names, and build services once per process.

## Context

This work package addresses findings A4, A5 and the dependency-injection notes in `docs/plans/review-2026-10-08-followup/report.md`. It touches `src/runtime/mod.rs` and `src/store` naming, which WP1 also restructures — **start after WP1 lands**, and re-measure the edges first since WP1 may already remove some.

## Findings

### 💡 Lower layers depend upward or sideways

**Issue:**
- `feed→runtime`: `src/feed/mod.rs` uses `runtime::poll_ownership`.
- `feed→mcp`: `McpEvent`.
- `mcp→cli`: `src/mcp/handlers/hooks.rs` calls `crate::cli::pane_key_event`.
- `host_file→startup`, `spacetime→startup`: `src/host_file/mod.rs`, `src/spacetime/managed_store.rs` return `startup::StartupAbort`.
- `sync→service`: `src/sync/writes.rs` and `memory_caller` use `service::Clock`.
- `dispatch→service` and `service→dispatch` form a cycle via `src/dispatch/prompts.rs` → `service::embeddings`.

**Fix:** Move `poll_ownership`, `Clock`, `pane_key_event` and `StartupAbort` to leaf modules (`models` or a new small module); move `embeddings` out of `service` (e.g. `src/embeddings.rs`). Move `McpEvent` to where both `feed` and `mcp` can depend on it downward.

### 💡 "Store" names four unrelated things

**Issue:** `store::Store` (router), `spacetime::store::SharedStore` (backup/restore trait; impls `SpacetimeCliStore`, `MemoryStore`), `sync::SyncStore` (session trait), `spacetime/managed_store.rs` (process lifecycle). Store wiring (`StoreParts`, `CliStore`, `open_cli_store`) lives in `src/runtime/mod.rs`, so `src/cli/store_import.rs` imports `runtime`.

**Fix:** Rename `spacetime::store::SharedStore` → `SnapshotTarget` (and its file). Move the store wiring out of `runtime/mod.rs` into its own module that both `runtime` and `cli` use. Coordinate with WP9, which splits `runtime/mod.rs` too — do the wiring move here and leave the event-loop split to WP9.

### 💡 Services built twice; inconsistent handles (`src/mcp/mod.rs:181`, `src/runtime/mod.rs:1418`, `src/service/api.rs:629`)

**Issue:** `EpicService::new(db, learnings)` takes two handles that are the same object at every call site. `TaskService` resolves its host id lazily (`db.ensure_host_identity()`) while `ReducerWriter` receives `host_id` up front. `McpState::new` and the TUI runtime each build their own `TaskService`/`EpicService`/`LearningService`.

**Fix:** `EpicService::new` takes one handle. Build the services once in the composition root and share them with MCP and the TUI. Pass `host_id` explicitly if that removes the lazy path without changing behaviour; check `docs/specs/` for the host-identity rules first.

## Changes

| File | Change |
|------|--------|
| `src/feed/mod.rs`, `src/runtime/poll_ownership*` | Move `poll_ownership` down |
| `src/mcp/handlers/hooks.rs`, `src/cli/*` | Move `pane_key_event` down |
| `src/startup/*`, `src/host_file/mod.rs`, `src/spacetime/managed_store.rs` | Move `StartupAbort` down |
| `src/service/*`, `src/sync/writes.rs`, `src/dispatch/prompts.rs` | Move `Clock`, `embeddings` |
| `src/spacetime/store.rs` | Rename to `SnapshotTarget` |
| `src/runtime/mod.rs`, `src/cli/store_import.rs` | Move store wiring to its own module |
| `src/service/epics.rs`, `src/mcp/mod.rs`, `src/runtime/mod.rs` | One handle; services built once |
| `docs/module-map.md`, `docs/invariants.md` | Update names |

## Verification

- [ ] Re-run the cross-module edge count: no `feed→runtime`, `feed→mcp`, `mcp→cli`, `*→startup` from lower layers, no `service↔dispatch` cycle
- [ ] `cargo test --no-fail-fast` — all pass
- [ ] `cargo clippy --all-targets -- -D warnings`; doc checkers pass
