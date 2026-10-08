# Move Inline Tests and Split run_tree_action

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Move large inline test modules to sibling files and split the one production function over 120 lines.

## Context

This work package addresses findings from the code review in `docs/plans/review-2026-10-08/report.md`.

## Findings

### 💡 Inline test modules (`src/cli/agent_diff.rs:778`, `src/repo_sync.rs:486`, `src/editor.rs:478`, `src/agent_tree.rs:573`, `src/runtime/mod.rs:1001`, `src/process.rs:808`)

**Issue:** Each file holds 450-1200 lines of tests after the production code, hiding the size of the real code.

**Fix:** Move each `mod tests` to a sibling file (`#[cfg(test)] mod tests;`), as done for `setup/plugins.rs`. Move only; no content changes. Do not commit unrelated `cargo fmt` churn.

### 💡 Long function (`src/cli/agent_tree.rs::run_tree_action`, 126 lines)

**Issue:** Over the 120-line limit.

**Fix:** Extract per-action helpers; keep behaviour identical.

## Changes

| File | Change |
|------|--------|
| the six files above | Tests to sibling files |
| `src/cli/agent_tree.rs` | Split `run_tree_action` |
| `docs/module-map.md`, docs citing moved test paths | Update citations (`check-doc-paths.sh`, `check-doc-symbols.sh`) |

## Verification

- [ ] Test count unchanged (compare `cargo test` totals before and after)
- [ ] Clippy clean; doc checkers pass
