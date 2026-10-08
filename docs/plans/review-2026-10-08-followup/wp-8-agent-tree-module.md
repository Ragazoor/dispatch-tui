# Agent Tree in One Module

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Gather the agent-tree feature, now spread over seven modules in two places, into one `src/agent_tree/` module with `cli` as a thin entry point.

## Context

This work package addresses the "Module placement" finding and the `src/cli/agent_tree.rs` split in `docs/plans/review-2026-10-08-followup/report.md`. **Depends on WP3** (shared git helper) — start after it lands. Also check epic #20482 "Move agent-tree to Claude Code mods": if the Rust pane is about to be retired, ask the user whether this package is still worth doing before starting.

## Findings

### 🔵 Agent tree spread across 7 modules (`src/agent_tree.rs`, `src/agent_tree_open_set.rs`, `src/agent_tree_diff_pane.rs`, `src/cli/agent_tree.rs`, `src/cli/agent_diff.rs`, `src/cli/agent_tree_agents.rs`, `src/cli/agent_tree_commits.rs`)

**Issue:** Top level holds the pure tree, open set and tmux diff-pane effect; `src/cli/` holds the 1,801-line run loop/render/keys plus three siblings. `dispatch` reaches into `agent_tree_open_set`; `cli` imports `tui::ui::palette`.

**Fix:** `src/agent_tree/{model,open_set,diff_pane,keys,run,render/*}`. `src/cli/` keeps thin subcommand entry points. Move `palette` to a shared UI module both `tui` and `agent_tree` import.

### 🔵 `src/cli/agent_tree.rs` is 1,073 code lines

**Issue:** Git plumbing (L342–600, moved by WP3), key handling (L1141–1410), pollers and the run loop share one file.

**Fix:** Split along those seams inside the new module.

## Changes

| File | Change |
|------|--------|
| `src/agent_tree*.rs`, `src/cli/agent_*.rs` | Move into `src/agent_tree/` |
| `src/cli/mod.rs` | Thin entry points |
| `src/tui/ui/palette.rs` | Move to a shared location |
| `src/dispatch/*` | Update imports |
| `docs/module-map.md`, `CLAUDE.md` (if it cites paths) | Update paths |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass (tests move with their modules)
- [ ] `cargo clippy --all-targets -- -D warnings`; `scripts/check-doc-paths.sh` and `check-doc-symbols.sh` pass
- [ ] `dispatch agent-tree` still runs in a scratch tmux socket
