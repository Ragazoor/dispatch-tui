# MCP context reads and small dedupes

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Route MCP context resource reads through the services, page them without loading everything, and remove three small duplications.

## Context

This work package addresses findings from the 2026-10-07 codebase review (`docs/plans/review-2026-10-07/report.md`, §1 and §4). The MCP context resources are specced in `docs/specs/mcp-task-tools.allium` (`McpContextResources`, `read_context`). Check the spec before changing behaviour; the listing order and cursor semantics must not change.

## Findings

### 🔵 Context resources read through two paths (`src/mcp/handlers/context.rs`)

**Issue:** `list_entries` lists the caller's own task via `state.db.get_task`, but `task_text` reads it via `state.task_svc.get_task`. Learnings are read via `state.db.get_learning` in both `read` (the `ContextUri::Learning` arm) and `list_entries`, while every learnings handler (`src/mcp/handlers/learnings.rs`) uses `state.learning_svc`. A task could be listed and then fail to read, or the other way round, if the two paths ever diverge.

**Fix:** Use `state.task_svc` and `state.learning_svc` everywhere in `context.rs`. Add a test that a listed entry is always readable.

### 🔵 Listing loads everything per page (`src/mcp/handlers/context.rs::list_entries`)

**Issue:** Each page loads every approved learning and every skill, then filters by cursor. O(N) work per page; grows with the knowledge base.

**Fix:** Push the cursor into the learnings query (id > cursor, limit page size + 1) where the service supports it; skills are a static list and can stay in memory. If the service cannot filter, record why and leave it.

### 🔵 Primitive ids in `ContextUri` (`src/mcp/handlers/context.rs`)

**Issue:** `ContextUri::Learning(i64)` / `Task(i64)` wrap to `LearningId`/`TaskId` only at use.

**Fix:** Hold the newtypes in the enum.

### 🔵 Hex encoding written three times (`src/mcp/handlers/context.rs`, `src/setup/plugins.rs`, `src/spacetime/managed_store.rs`)

**Issue:** Three hand-rolled hex encode/decode helpers.

**Fix:** One small shared helper (e.g. in `src/models/` or a `util` module already in use), with a round-trip test; or the `hex` crate if it is already in the dependency tree (`cargo tree -i hex`).

### 🔵 Status literals bypass constants (`spacetime/module/src/blanks.rs`, `spacetime/module/src/tasks_epics.rs`)

**Issue:** `spacetime/module/src/support.rs` defines `BACKLOG`/`RUNNING`/`REVIEW`/`DONE`, but `blanks.rs` and `tasks_epics.rs` use string literals.

**Fix:** Use the constants. This is the separate wasm crate: rebuild with `./scripts/build-managed-module.sh` and commit `module.wasm` if it changes; see `docs/testing.md` ("SpacetimeDB module and CI details").

## Changes

| File | Change |
|------|--------|
| `src/mcp/handlers/context.rs` | Service reads, cursor-aware listing, newtype ids, shared hex helper. |
| `src/setup/plugins.rs`, `src/spacetime/managed_store.rs` | Use the shared hex helper. |
| shared helper location (new or existing) | Hex encode/decode + test. |
| `spacetime/module/src/blanks.rs`, `spacetime/module/src/tasks_epics.rs` | Use status constants. |
| `spacetime/module/module.wasm` (if changed) | Rebuilt artifact. |

## Verification

- [ ] `cargo clippy --all-targets -- -D warnings` clean
- [ ] `cargo test` — all pass, including new list-then-read test
- [ ] `./scripts/check-spacetime-module.sh` passes
- [ ] `allium:weed` on `mcp-task-tools.allium` context-resource rules reports no new divergence
