# Fixtures and test-only gating

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Tests build requests, tasks and apps through the existing builders instead of full struct literals, and every function that only tests call is gated or deleted.

## Context

This work package addresses findings from the 2026-10-14 code review
(`docs/plans/review-2026-10-14/report.md`, §3 "Fixture duplication", T3,
§8 M5, M6). The builders exist (`CreateTaskRequest::fixture` at
`src/store/fixtures.rs:14`, `TaskBuilder`, `EpicBuilder`, five `make_app*`
helpers in `src/tui/tests/dispatch.rs`); the 2026-10-08 work package added
them but did not migrate the call sites. No behaviour change.

## Findings

### 💡 Fixture literals (`src/store/tests/*.rs`, `src/mcp/handlers/tests/tasks/wrap_up.rs`, `src/service/tasks/tests/crud/*.rs`, `src/tui/tests/dispatch.rs`)

**Issue:** `CreateTaskRequest {` is written out 180 times, `Task {` 228 times,
`App::new(` 842 times (101 in `src/tui/tests/dispatch.rs` alone).

**Fix:** Migrate the `CreateTaskRequest` literals to
`CreateTaskRequest::fixture(title, repo)` plus `..`/setters; the `Task`
literals in test code to `TaskBuilder`; the `App::new(` setups in
`src/tui/tests/dispatch.rs` to the `make_app*` helpers (add a
`make_app_with_running_task` if the repeated `task.tmux_window = Some(…)`
pattern needs one). A mechanical pass per file; keep each test's assertions
untouched.

### 💡 Test-only functions without a gate

| Location | Function |
|---|---|
| `src/keybindings.rs:357` | `contexts_overlap` |
| `src/agent_tree/state.rs:55`, `:63`, `:222` | `Notice::agent_list`, `Notice::commit_list`, `RenderState::is_diff_open` |
| `src/service/tasks/wrap_up.rs:225` | `TaskService::with_verify_wait` |
| `src/runtime/editor.rs:87` | `EditorSession::occupied_for_test` |
| `src/runtime/mod.rs:806` | `TuiRuntime::bootstrap_with` |
| `src/sync/memory_caller/mod.rs:276` | `MemoryReducerCaller::task_owner` |
| `src/spacetime/snapshot.rs:366`, `:447` | `SharedTable::assembled_columns`, `module_only_columns` |
| `src/spacetime/managed_store.rs:585` | `ManagedStore::with_timeouts` |
| `src/process.rs:634`, `:641`, `:654`, `:670`, `:723`, `:743` | `MockProcessRunner` builders and recorders |
| `src/setup/confirm.rs:113`, `:117`, `:121` | `ConfirmSpy` counters |
| `src/agent_tree/test_repo.rs:78`, `:102`, `:117`, `:124` | `TestRepo` helpers |

**Issue:** 26 functions have no production caller and no
`#[cfg(test)]`/`#[cfg(any(test, feature = "test-support"))]` gate, so they
ship in the release binary and read as API.

**Fix:** Gate each with `#[cfg(any(test, feature = "test-support"))]` (the
`MockProcessRunner`, `ConfirmSpy` and `TestRepo` ones may already sit in a
gated `impl`; if so, confirm and skip). Where a function is a derived rule
nothing reads (`contexts_overlap`, `is_diff_open`), delete it and its tests
unless a spec names it — then say so in the gate comment.

## Changes

| File | Change |
|------|--------|
| `src/store/tests/tasks.rs`, `src/store/tests/tasks_patch.rs`, `src/mcp/handlers/tests/tasks/wrap_up.rs`, `src/service/tasks/tests/crud/sub_status.rs`, `src/service/tasks/tests/crud/epic_in_epic.rs` | `CreateTaskRequest::fixture` |
| `src/tui/tests/dispatch.rs` | `make_app*` helpers; `crate::tui::messages::` paths via `use` |
| Test files with `Task {` literals | `TaskBuilder` |
| The 13 production files above | `#[cfg(any(test, feature = "test-support"))]` or delete |

## Verification

- [ ] `cargo test --no-fail-fast` green; `cargo clippy --all-targets -- -D warnings` green
- [ ] `grep -rn 'CreateTaskRequest {' src tests | wc -l` ≤ 20 (the fixture itself and genuine edge cases)
- [ ] `cargo build --release` compiles without the listed functions (`cargo doc --no-deps` shows none of them)
