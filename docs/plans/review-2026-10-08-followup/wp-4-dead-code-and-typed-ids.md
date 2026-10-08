# Dead Code and Typed Ids at the Edges

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Delete uncalled functions, gate test-only ones, and use `TaskId`/`TaskStatus` where raw primitives remain at the hook and CLI boundary.

## Context

This work package addresses findings S3 and S4 from `docs/plans/review-2026-10-08-followup/report.md`. Today's earlier review (epic "Code Review: 2026-10-08", WP6) typed the bulk of the service signatures; these are the remaining edge sites. Avoid `src/cli/agent_tree.rs` beyond the two `i64` sites — WP3 and WP8 restructure it.

## Findings

### 💡 Uncalled public functions (`src/tmux.rs:576`, `src/tui/mod.rs:456`)

**Issue:** `tmux::current_session_name` and `App::input_buffer` have no callers anywhere, including tests. They escape the `dead_code` lint because they are `pub` in a lib crate.

**Fix:** Delete them.

### 💡 Test-only functions not gated

**Issue:** Called only from tests, with no `cfg` gate: `TaskTag::short_label`, `RepoSyncState::is_diverged`, `RepoSyncState::is_measured` (an Allium `derived` clause), `tmux::current_window_name`, `ConnectionStatus::is_healthy`, `App::tasks_by_status`, `App::status_message`, `FieldUpdate::from_optional_string`, `TmuxWindow::into_string`, `spacetime/store.rs::next_generated_id`, `snapshot.rs::canonical_rows`.

**Fix:** Gate each with `#[cfg(test)]` or the `test-support` feature (the latter if `tests/` uses it). For `is_measured`/`is_diverged`, check `docs/specs/repo-sync.allium`: if the spec's `derived` value is meant to drive behaviour, that's a spec-code gap to raise with the user rather than gate silently.

### 💡 Raw `i64` task ids at the edge (`src/hooks/wire.rs`, `src/cli/mod.rs`, `src/main.rs`, `src/agent_tree_diff_pane.rs`, `src/hooks/mod.rs`, `src/cli/agent_tree.rs`)

**Issue:** About 31 `task_id: i64` sites remain (wire.rs 7, cli/mod.rs 9, main.rs 4, cli/agent_tree.rs 2, agent_tree_diff_pane.rs 2, hooks/mod.rs). `TaskId` derives serde as a newtype, so it serialises as a bare number and the wire format does not change.

**Fix:** Use `TaskId`. Add a serde round-trip test on one hook wire struct proving the JSON is unchanged.

### 💡 Status compared as a string (`src/sync/sdk_connector/outcome.rs::is_review`)

**Issue:** `is_review` compares `status == "review"` instead of using `TaskStatus`.

**Fix:** Parse to `TaskStatus` (or compare against `TaskStatus::Review.as_str()`), matching the rest of the module.

## Changes

| File | Change |
|------|--------|
| `src/tmux.rs`, `src/tui/mod.rs` | Delete two dead fns; gate test-only ones |
| Files owning the test-only fns listed above | `#[cfg(test)]` / `test-support` |
| `src/hooks/wire.rs`, `src/hooks/mod.rs`, `src/cli/mod.rs`, `src/main.rs`, `src/agent_tree_diff_pane.rs`, `src/cli/agent_tree.rs` | `i64` → `TaskId` |
| `src/sync/sdk_connector/outcome.rs` | `is_review` uses `TaskStatus` |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass
- [ ] `cargo clippy --all-targets -- -D warnings` (also catches newly-dead code once gated)
- [ ] Hook wire JSON round-trip test passes unchanged
