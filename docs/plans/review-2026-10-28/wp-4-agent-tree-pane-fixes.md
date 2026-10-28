# Agent Tree Pane Fixes

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Stop the `v` key from running the pane-close path, and tidy the agent tree's pane lifecycle, refresh timing and docs.

## Context

This work package addresses findings G1–G6 from `docs/plans/2026-10-28-codebase-review.md`. Spec: `docs/specs/agent-tree.allium`. Test real tmux only through `tests/tmux_harness/mod.rs` (private `-L` socket); never drive the operator's tmux by hand.

## Findings

### 🟡 Medium — G1: `v` runs the close path the spec says it must not (`src/agent_tree/run.rs:140-143`)

**Issue:** `open_in_editor` → `publish_open_set` → `diff_pane.rs::reconcile_diff_pane(false)`. `OpenAgentTreeFileInEditor` says emptying the set this way is not an `AgentTreeDiffOpenSetChanged` and does not close the pane. It is harmless only while `diff_pane.rs::mark_content(EDITOR)` succeeds; that function logs and swallows its error (`diff_pane.rs:226`). If the `set-option` fails, the pane reads as "diff", reconcile takes `(false, Some((pane, false)))` and kills the editor just opened.

**Fix:** In `open_in_editor`, write the emptied set with `open_set::write_open_set` and skip the reconcile. Test with a failing content mark.

### 🟢 Low — G2: role and content read in two tmux calls (`src/agent_tree/diff_pane.rs:55-72`)

**Issue:** `existing_lower_pane` reads the role, then the content, separately; a pane closed between them counts as a diff and `kill_pane` hits a gone pane (spurious notice). `src/dispatch/agents.rs::toggle_agent_tree_pane` avoids this (comment near line 246).

**Fix:** One `list-panes` with both options in the format string.

### 🟢 Low — G3: a second `$TMUX_PANE` parser (`src/agent_tree/diff_pane.rs:42`)

**Issue:** `current_pane_from_env` checks only non-empty; `tmux::self_pane_id` checks `is_pane_id`.

**Fix:** Use `tmux::self_pane_id`, mapping `None` to the existing error.

### 🟢 Low — G4: an error exit orphans the diff pane (`src/agent_tree/run.rs:293-303`)

**Issue:** A `?` from `terminal.draw` or `event::poll` in `run_loop` skips `tear_down_diff_pane` (breaks `KillAgentTreeDiffPaneWithItsTree`). `src/agent_tree/pane.rs:378-380` (`with_pane_terminal`) leaves raw mode on if `EnterAlternateScreen` fails.

**Fix:** Drop guards for the teardown and the terminal restore.

### 🟢 Low — G5: key presses starve the refresh (`src/agent_tree/run.rs:303-333`, `src/agent_tree/diff_viewer.rs:731-750`)

**Issue:** Both loops refresh only when the 1-second `event::poll` times out, so held keys stop git refreshes. `RefreshAgentTree` is defined on elapsed time.

**Fix:** Track the last refresh time; refresh when due, key or no key. Use the repo's clock abstraction (no wall-clock sleeps in tests; see `docs/testing.md`).

### 🟢 Low — G6: stale doc comments (`src/agent_tree/open_set.rs`, `src/agent_tree/diff_viewer.rs`)

**Issue:** `open_set.rs::write_open_set` ("the order is part of the message") and `diff_viewer.rs::LastSeen` ("ORDER INCLUDED") predate 18c7d51f; the set is now a `BTreeSet` of at most one path.

**Fix:** Reword both.

## Changes

| File | Change |
|------|--------|
| `src/agent_tree/run.rs` | G1 skip reconcile on `v`; G4 drop guard; G5 due-time refresh |
| `src/agent_tree/diff_pane.rs` | G2 one `list-panes`; G3 `tmux::self_pane_id` |
| `src/agent_tree/pane.rs` | G4 raw-mode restore guard |
| `src/agent_tree/diff_viewer.rs` | G5 due-time refresh; G6 comment |
| `src/agent_tree/open_set.rs` | G6 comment |

## Verification

- [ ] G1 and G5 tests fail first, then pass
- [ ] `cargo test --no-fail-fast` with `tmux` on `PATH`; no `skipping:` lines
- [ ] `./scripts/check-no-test-sleep.sh`, `cargo fmt --check && ./scripts/check-doc-paths.sh`
