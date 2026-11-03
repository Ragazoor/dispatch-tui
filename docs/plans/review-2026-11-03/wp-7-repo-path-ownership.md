# Repo Path Ownership

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** One host never deletes or takes over another host's repo-path row, including legacy rows with no host.

## Context

Findings K1 and K2 from `docs/plans/2026-11-03-codebase-review.md`. K1 is "fix for W2 (714b59ab) → regression"; K2 is "fix for C3 (d6e86328) is client-side only". Spec first: `docs/specs/cli.allium` (`PruneRepoPathsViaCli`) and the repo-path rules, then tests that fail, then code. Mirror module changes in `src/sync/memory_caller/` with a conformance case; a new reducer or signature change bumps `RULES_VERSION` and `BOARD_RULES_VERSION` together; rebuild `module.wasm` and bindings.

## Findings

### 🟡 Medium — K1: prune deletes another host's legacy row (`src/cli/commands.rs::prune_repo_paths`, the unowned loop)

**Issue:** an unowned row (`host == ""`) whose path is missing on *this* disk is deleted. Host B prunes first and deletes Alice's legacy `/home/alice/proj` row, verify command included, before host A can adopt it. The spec's own guidance says a path "says nothing about this disk".

**Fix:** adopt an unowned row when its path is present here; never delete an unowned row. Update `PruneRepoPathsViaCli`. Test both branches.

### 🟡 Medium — K2: the module still re-stamps the host on save, and delete has no host guard (`spacetime/module/src/config.rs::save_repo_path`, `::delete_repo_path`; `src/runtime/tasks.rs::exec_save_repo_path`)

**Issue:** every TUI save of an existing path makes the caller's host its owner, and K1's adoption relies on that re-stamp, so the two fixes pull in opposite directions. `delete_repo_path` deletes by path, so a prune working from a stale cache deletes a row another host has just adopted.

**Fix:** add an `adopt_repo_path` reducer that stamps only when `host == ""`; `save_repo_path` never changes a non-empty host; `delete_repo_path` takes the expected host and deletes only on a match. Use `adopt_repo_path` from prune.

## Changes

| File | Change |
|------|--------|
| `docs/specs/cli.allium` (and the repo-path spec) | Adopt, never delete unowned; save keeps host; delete by host |
| `spacetime/module/src/config.rs` | `adopt_repo_path`; `save_repo_path` keeps host; `delete_repo_path` host guard |
| `src/cli/commands.rs` | Prune uses adopt and host-guarded delete |
| `src/store/`, `src/sync/` | New store calls |
| `src/sync/memory_caller/config.rs` | Mirror |
| `tests/memory_caller_conformance/`, `tests/cli.rs` | Cases for each rule |

## Verification

- [ ] New tests fail before the change and pass after
- [ ] `cargo test --no-fail-fast` green
- [ ] Module checks per `docs/testing.md`; `module.wasm` and bindings rebuilt
- [ ] `cargo clippy --all-targets -- -D warnings` clean
