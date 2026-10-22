# Bootstrap Inputs Struct

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the three 7-parameter composition-root functions with one inputs struct.

## Context

This work package addresses findings from `docs/plans/2026-10-22-codebase-review.md`. It is a pure refactor with no behaviour change. It touches `src/runtime/mod.rs`, so run it after the split-pane package or rebase carefully.

## Findings

### Medium: Composition root threads the same values through three functions (`src/runtime/mod.rs:799-830`, `:978`)

**Issue:** `bootstrap_with` forwards six values to `bootstrap_inner`, which builds `StoreParts` and passes five of them plus new ones to `build_runtime`. Adding one startup input touches three signatures and every test caller.

**Fix:** Add `BootstrapInputs { data_dir, port, paths, target, build_store, accept_store_switch }`. `build_runtime` takes it by reference.

### Low: `list_epic_subtree` takes ten parameters (`src/tui/update/board_filter.rs:328`)

**Issue:** Six (`children`, `epics`, `hidden`, `folded`, `qualifies`, `out`) stay the same across the recursion.

**Fix:** A `SubtreeWalk<'a>` struct holds them, with a `walk(node, depth, parent_hidden)` method.

## Changes

| File | Change |
|------|--------|
| `src/runtime/mod.rs` | Add `BootstrapInputs`; update `bootstrap_with`, `bootstrap_inner`, `build_runtime` |
| `src/runtime/tests/` | Update test callers of the bootstrap functions |
| `src/tui/update/board_filter.rs` | Add `SubtreeWalk`; update `list_epic_subtree` |

## Verification

- [ ] Run existing tests: `cargo test --no-fail-fast > scratch.txt 2>&1; echo $?`
- [ ] `cargo fmt` and clippy clean; no behaviour change
