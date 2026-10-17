# tmux Errors and Runtime Reactors

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** tmux query failures are reported as failures, tmux calls cannot freeze the TUI, and the session reactor never loses a tear-down.

## Context

This work package addresses findings 10–12 and 36 of the 2026-10-17 code
review (`docs/plans/2026-10-17-codebase-review.md`). Never drive tmux by hand
without `-L <unique-socket>`; test through `tests/tmux_harness/mod.rs`.

## Findings

### 🟡 tmux non-zero exit read as "no windows" (`src/tmux.rs:375`, `src/runtime/editor.rs:41`)

**Issue:** `list_all` maps any non-zero exit to an empty list, so
`has_window` returns `Err` only on spawn failure. A transient `list-windows`
failure makes a live window look absent: the editor watcher ends, liveness
reports a crash, teardown skips the kill. This contradicts the docs on
`has_window_or_assume_present` and `MAX_CONSECUTIVE_QUERY_FAILURES`.

**Fix:** Treat only "no server running" / "error connecting" stderr as an
empty list; return `Err` otherwise, so the documented retry and
assume-present paths run. Audit `list_all` callers for the new `Err`.

### 🟡 Unbounded tmux calls on the command-drain thread (`src/runtime/editor.rs:235`)

**Issue:** `exec_pop_out_editor` runs `new_window_running` and
`select_window` synchronously via unbounded `runner.run`; a wedged tmux
server freezes input and rendering.

**Fix:** Move them into `spawn_blocking`, or use `run_with_timeout`. Check
`setup_tmux_for_tui` / `teardown_tmux_for_tui` (`src/runtime/mod.rs:130`)
for the same shape.

### 🟡 Session reactor drops tear-down rows before identity settles (`src/runtime/session_host.rs:68`)

**Issue:** In `act`, when `own_identity()` is `None`, only `work.serve` rows
are released back; `work.tear_down` ids were already removed from
`open_exits` by `observe` and are never handed out again, leaving an orphaned
window.

**Fix:** Re-open the tear-down ids (add a `SessionRequestObserver::reopen`),
or defer the removal until the kill is attempted.

### 🟢 Low finding

- `src/tmux.rs:703` — `ensure_split_hook`'s doc block sits above
  `SPLIT_NEEDS_CORRECTION`, so the const carries it and the function has none.
  Move it.

## Changes

| File | Change |
|------|--------|
| `src/tmux.rs` | `list_all` error classification; doc placement |
| `src/runtime/editor.rs` | Off-thread / bounded tmux calls |
| `src/runtime/session_host.rs` | Re-open tear-down rows |

## Verification

- [ ] Tests first: mock runner returning non-zero with other stderr → `has_window` is `Err`; "no server running" → empty
- [ ] Test: tear-down row survives an `act` with no identity and is served once identity arrives
- [ ] `cargo test --no-fail-fast` (with `tmux` on PATH) and the task's verify command pass
