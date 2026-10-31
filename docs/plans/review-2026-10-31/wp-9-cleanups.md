# Cleanups

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix the small residues of the 2026-10-28 fixes: the editor-pane kill on tree exit, test-store leaks, and stale docs.

## Context

This work package addresses findings R4, R7 and R8 from `docs/plans/2026-10-31-codebase-review.md`, plus the long-optional W2.

## Findings

### 🟢 Low — R4: G1's hazard moved to tree exit, and its guard test was deleted (`src/agent_tree/run.rs::DiffPaneTeardown::drop`)

**Issue:** `DiffPaneTeardown::drop` → `tear_down_diff_pane` → `diff_pane.rs::reconcile_diff_pane(false)`. If `diff_pane.rs::mark_content` fails after `v` (it logs and swallows the error), the lower pane still reads as "diff", and quitting the tree kills the editor with unsaved edits, breaking `EditorPaneOutlivesItsTree`. a29c0c51 deleted `a_failed_content_lookup_is_an_error_rather_than_a_kill`.

**Fix:** Return the `set-option` error from `show_in_editor` so a pane is never editor-but-unmarked (close or report instead). Re-add a "failed lookup does not kill" test.

### 🟢 Low — R7: test-store residue (`src/sync/sdk_connector/connect.rs`, `src/runtime/tests/live_store.rs::try_start`)

**Issue:** A dropped `TuiRuntime` aborts its tasks but never disconnects the SDK connection; the subscription and `run_threaded` thread live until process exit. `try_start` can still accept a sibling test's store when the probe answers while its own child is starting.

**Fix:** `impl Drop` (on the connector or `TuiRuntime`) that calls `disconnect()`. In `try_start`, confirm the answering store is ours (its database identity, or the child's listening socket).

### 🟢 Low — R8: stale docs (`src/git.rs:238`, `src/agent_tree/diff_viewer.rs:239`)

**Issue:** `src/git.rs` lists "measuring drift" as a caller of `ahead_behind`; the drift indicator was removed (a2f11560) and the only caller is `dispatch/worktree.rs`. `diff_viewer.rs` (~239-248, ~322) says the tree writes its rows' order into the open set; the set holds at most one path.

**Fix:** Reword both.

## Decisions

### W2: unowned repo paths are never pruned (`src/sync/rows.rs::repo_paths_saved_by`)

**Question:** Open since 2026-10-22 as optional. Should a prune adopt repo paths no host owns?

**Options:**
- (a) Adopt unowned paths on prune (this host claims them, then prunes as usual). Small change; a path another host meant to keep could be pruned.
- (b) Leave as is and close the finding. Costs nothing; the list may grow slowly.

Ask the user with `AskUserQuestion` before implementing this decision. Do not pick an option yourself. If the user does not answer, leave it unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `src/agent_tree/diff_pane.rs`, `src/agent_tree/run.rs` | Surface `mark_content` failure; re-add test |
| `src/sync/sdk_connector/connect.rs` or `src/runtime/mod.rs` | Disconnect on drop |
| `src/runtime/tests/live_store.rs` | Confirm store identity |
| `src/git.rs`, `src/agent_tree/diff_viewer.rs` | Doc fixes |
| `src/sync/rows.rs` | W2, if chosen |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass, no `skipping:` lines
- [ ] `./scripts/check-rustdoc-links.sh` and the other `check-*.sh` pass
