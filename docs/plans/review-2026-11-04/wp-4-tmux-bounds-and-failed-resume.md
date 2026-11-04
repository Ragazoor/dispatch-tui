# tmux bounds and failed resume

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Bound every tmux call on provisioning and UI paths, clean up after a failed resume, and target the board's own pane and window.

## Context

Findings H2, H3, H4, H5, H6, H8 and H9 from `docs/plans/2026-11-04-codebase-review.md` §3.7 (fresh area). **Never run `tmux kill-server` and never drive tmux by hand without a private `-L` socket**; real-tmux tests go through `tests/tmux_harness/mod.rs`. `tmux display-message -p` without `-t` answers about the session's active window — pass `tmux::self_pane_id()` from the entry point.

## Findings

### 🟡 Medium — H2: a resume that fails after creating the window leaves an empty window the next resume adopts (`src/dispatch/agents.rs::resume_agent_with_message`, `src/tui/update/agent.rs::handle_resume_failed`)

**Issue:** if `set_window_dispatch_dir`, `ensure_split_hook`, the `.claude-prompt` write or `send_keys` fails after `tmux::new_window`, the `task-N` window stays with a bare shell. The next resume finds it via `has_window`, "reattaches", marks the task Running/Active with no claude, and drops the message. The fresh-dispatch path rolls back with `rollback_failed_provisioning`.

**Fix:** on any error after `new_window`, kill the window (`kill_window_if_present`) before returning. Add a mock-runner test that fails `send_keys` and asserts the kill.

### 🟡 Medium — H3: the #4202 subprocess bound is incomplete (`src/tmux.rs::new_window`, `::set_window_option`, `::run_checked_timeout`)

**Issue:** `new_window` first runs `refuse_duplicate_window_name` → `has_window` → `list_all`, an unbounded `list-windows -a`; every `set_window_option` resolves through `window_target`, an unbounded `list-panes -a`. `send_keys`, `new_window_running`, `select_window`, `kill_window`, `join_pane`, `respawn_pane*` and `split_*` are unbounded throughout. A wedged tmux server parks provisioning and resume threads; the old provisioning watchdog (`PROVISION_MAX_SUBPROCESS_CALLS`, d0c2fe26) is gone.

**Fix:** route `list_all` and `window_target` through `run_with_timeout`, or make `run_checked` bounded by default (one timing constant, per `docs/invariants.md` timing constants). Fix `run_checked_timeout`'s doc to match.

### 🟡 Medium — H4: jumping to a task window runs tmux on the command-drain thread (`src/runtime/split.rs::exec_jump_to_tmux`)

**Issue:** it calls `tmux::select_window` inline from `src/runtime/commands.rs` (`JumpToTmux`). `src/runtime/editor.rs` states the rule: a wedged server "would otherwise freeze the command-drain thread, and with it input and rendering".

**Fix:** `spawn_blocking`, report failure through `msg_tx`, as the sibling `exec_*` helpers do. Update `exec_jump_to_tmux_calls_select_window`.

### 🟢 Low — H5: entering split mode targets the session's active pane (`src/runtime/split.rs::enter_split_outcome`, `::enter_split_with_task_outcome` → `tmux::current_pane_id`)

**Fix:** use `tmux::self_pane_id()` read at the entry point and passed down (as `setup_tmux_for_tui` does), falling back to `current_pane_id` only when unset.

### 🟢 Low — H6: teardown restores the board window's name by a server-wide lookup (`src/tmux.rs::rename_window_in_session`, `src/runtime/mod.rs::teardown_tmux_for_tui`)

**Issue:** with two boards in two sessions the "TUI" lookup fails (or renames the other session's window). The call-site comment claims the opposite.

**Fix:** target `self_pane_id()` (or resolve with `pane_id_of_window_in_session`); correct the comment.

### 🟢 Low — H8: `~/.tmux.conf` is rewritten in place, and a commented-out line counts as set (`src/tmux.rs::write_focus_events_to_tmux_conf_at`, `::contains_focus_events`)

**Fix:** write with `crate::atomic_write`; match an uncommented `set(-option)? -g focus-events on` line. Test both.

### 🟢 Low — H9: doc drift in `src/tmux.rs`

**Fix:** `split_window_below_running` (hook uses `respawn-pane -k -c`, not a typed `cd`); `refuse_duplicate_window_name_in_session` (it errors, and its `Err` arm is mostly unreachable); `run_checked_timeout`'s user list (add `set_window_awaits_pane`, `rejoin_pane`).

## Changes

| File | Change |
|------|--------|
| `src/dispatch/agents.rs` | H2 kill on error |
| `src/tmux.rs` | H3 bounds, H6, H8, H9 |
| `src/runtime/split.rs`, `src/runtime/commands.rs` | H4, H5 |
| `src/runtime/mod.rs` | H6 caller |
| tests under `src/tmux/tests/`, `src/dispatch/`, `src/runtime/tests/` | one per finding |

## Verification

- [ ] `cargo test --no-fail-fast` green (needs `tmux` on `PATH`; check no `skipping:` lines); `cargo clippy --all-targets -- -D warnings` clean
- [ ] `./scripts/check-no-test-sleep.sh` green
