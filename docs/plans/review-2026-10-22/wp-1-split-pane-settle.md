# Split-Pane Swap Settle

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make a split-pane swap settle on what tmux actually did, so a failure after `swap_pane` succeeds no longer leaves the board and tmux disagreeing.

## Context

This work package addresses findings from `docs/plans/2026-10-22-codebase-review.md`. This changes behaviour, so follow the repo's order: spec (`docs/specs/split-pane.allium`), then tests, then code.

## Findings

### Medium: Post-swap failure leaves board and tmux out of step (`src/dispatch/split_panes.rs:203`)

**Issue:** `swap_pane` succeeds, then `rejoin_pane` (`:211`) or `rename_window` (`:215`) fails and returns `Err`. `src/runtime/split.rs:242-249` maps any `Err` to `SplitMessage::SwapFailed`. `src/tui/update/split_pane.rs:244-248` then settles with `Settled::Swap(None)`, assuming the pane still holds the previous occupant (`:144-146`). That is false once `swap_pane` ran. The board keeps task A pinned while tmux shows task B. The rename arm had this before; commit 1f96c697 added the rejoin arm.

**Fix:** On a failure after `swap_pane`, try the reverse `swap_pane(right_pane, new_pane_id)`. Report `SwapFailed` only if the reverse succeeds. Otherwise send `SwapOpened { pane_id: new_pane_id, task_id }` plus a separate error for the leftover. Update `SwapSplitPane` in `docs/specs/split-pane.allium` to say what "settled" means when the exchange half succeeded.

### Medium: No test for a failure after `swap_pane` succeeds (`src/runtime/tests/split_mode.rs:365`, `:408`)

**Issue:** Only success paths are tested. This is why the defect shipped twice.

**Fix:** Add `MockProcessRunner` cases where `rename-window` fails, and where `join-pane` fails, after `swap-pane` succeeded. Assert what the board is told.

### Low: Wrong error context on the pre-swap check (`src/dispatch/split_panes.rs:179`)

**Issue:** A `has_window` failure is labelled "rename window failed", but no rename ran yet.

**Fix:** Use `.context("checking the outgoing window failed")`.

## Changes

| File | Change |
|------|--------|
| `docs/specs/split-pane.allium` | Define "settled" for a half-succeeded swap |
| `src/dispatch/split_panes.rs` | Reverse the swap or report the new occupant; fix `:179` context |
| `src/runtime/split.rs` | Map the new outcome to messages |
| `src/tui/update/split_pane.rs` | Handle the new outcome; fix the `:144-146` assumption |
| `src/runtime/tests/split_mode.rs` | Add the two post-swap failure tests |

## Verification

- [ ] Run existing tests: `cargo test --no-fail-fast > scratch.txt 2>&1; echo $?` (redirect, do not pipe)
- [ ] New tests fail before the code change and pass after
- [ ] Doc checkers pass (`check-doc-paths.sh`, `check-doc-symbols.sh`, `check-doc-headings.sh`)
