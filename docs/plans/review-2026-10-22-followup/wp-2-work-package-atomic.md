# Work-Package Atomic Writes

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make work-package create and update one store transaction each, and make the update reply say when an unlink touched nothing.

## Context

This work package addresses findings from the code review in `docs/plans/2026-10-22-codebase-review-followup.md` (A2, S4).

## Findings

### 🟢 Low: Work-package create and update are not one transaction (`src/service/work_packages.rs`)

**Issue:** `create_work_package` makes one reducer call for the package and then one `link_work_package_item` call per item. `update_work_package` patches, then unlinks, then links, all as separate calls. Scenario: a linked task is deleted between the item check and its link call; the package exists with partial links, the MCP call errors, the agent retries and a duplicate package appears. The service also repeats title/package/item checks the module already makes — the "data rules in service" pattern CLAUDE.md says to stop.

**Fix:** Add module reducers `create_work_package_with_links` and an update twin (patch + unlinks + links) in `spacetime/module/src/packages.rs`, each one transaction. Mirror them in `src/sync/memory_caller/work_packages.rs` and cover the pair in `tests/memory_caller_conformance.rs`. Point the service at them and delete its duplicate validation. Regenerate bindings (`./scripts/regenerate-spacetime-bindings.sh`) and rebuild the module (`./scripts/build-managed-module.sh`). Check whether the rule move warrants a `RULES_VERSION` bump by the constant's own doc (`spacetime/module/src/rules.rs`); move `BOARD_RULES_VERSION` with it if so.

### 🟢 Low: `update_work_package` reports success when an unlink touched nothing (`src/mcp/handlers/work_packages.rs`)

**Issue:** `handle_update_work_package` says "updated" even when a requested unlink named an item that was never linked; `unlink_work_package_item` treats a missing link as success.

**Fix:** Have the new update reducer (or the service) report which requested unlinks were not linked, and name them in the tool reply.

## Changes

| File | Change |
|------|--------|
| `spacetime/module/src/packages.rs` | New create-with-links and update reducers, one transaction each |
| `spacetime/module-tests/` | Reducer tests for the new reducers, including a missing item mid-list |
| `src/sync/memory_caller/work_packages.rs` | Twin of both reducers |
| `tests/memory_caller_conformance.rs` | Conformance cases for both |
| `src/service/work_packages.rs` | Call the new reducers; drop duplicate validation |
| `src/mcp/handlers/work_packages.rs` | Name unlinks that touched nothing |
| `src/spacetime/bindings/`, `module.wasm` | Regenerate / rebuild |

## Verification

- [ ] Run the task's verify command — all pass
- [ ] `cargo test --no-fail-fast` green; module-tests crate green
- [ ] `./scripts/build-managed-module.sh --check` passes
