# Board window checks and split pane

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** A board checks and reclassifies only the agents on its own machine, a failed tmux query never reads as "gone", and two boards in one tmux server stop stepping on each other.

## Context

Findings W1, W2, W7 and W8 from `docs/plans/2026-11-05-codebase-review.md` §3.4 (fresh area `src/runtime/`). Behaviour changes start in the spec (`docs/specs/agent-health.allium`, `split-pane.allium`), then tests, then code. Never drive tmux by hand without `-L <socket>`; real-tmux tests go through `tests/tmux_harness/mod.rs`.

## Findings

### 🔴 High — W1: every board crashes other machines' agents (`src/tui/update/agent.rs::tick_window_checks`, `::tick_sub_status`, `::handle_window_gone`, `::handle_agent_crashed`; `src/runtime/tasks.rs::exec_batch_check_windows`)

**Issue:** `tick_window_checks` takes every task with a `tmux_window`; `visible_agents` serves agents of every held task, so a teammate's running task, or your own on a second machine, has one. `exec_batch_check_windows` compares against `tmux list-windows -a` on this machine, never finds it, sends `WindowGone`, and `handle_agent_crashed` writes sub_status Crashed with `releasing_tmux_window()`, clears subagents and notifies "agent crashed". The module does not refuse it. The owning host's `tick_sub_status` skips Crashed tasks, so the card stays Crashed while the agent runs, and "press d to retry" can start a second agent. A non-running foreign task also has its pointer cleared. `tick_sub_status` has the same gap: every board reclassifies every Running task with its own clock and writes `batch_patch_sub_status`.

**Fix:** filter both ticks on `t.host.is_none() || t.host == local_host_id`, as `tick_pr_poll` does (and its test "a foreign-hosted task must not be polled"). As a second line, drop foreign ids in `handle_window_gone`. Tests: a foreign-hosted Running task with a window is neither checked nor reclassified. State it in `agent-health.allium: DetectCrashedAgent`.

### 🟡 Medium — W2: a failed tmux query reads as "split pane closed" (`src/runtime/split.rs::exec_check_split_pane`, `::exec_respawn_split_pane`; `src/tmux.rs::pane_exists`)

**Issue:** `pane_exists` does `list_all_pane_ids(..).unwrap_or(false)`. A timeout sends `SplitMessage::PaneClosed`; `SplitPaneClosedResets` clears `active` and `pinned_task` while the pane is still joined. Next tick `tick_window_checks` no longer skips that task, its standalone window does not exist, and W1's path marks it Crashed. `exec_respawn_split_pane` also sends `PaneClosed` on any error. Contradicts the policy of `has_window_or_assume_present`.

**Fix:** `pane_exists` returns `Result<bool>`; on `Err` do nothing (log), as `exec_check_window` does. Respawn errors report the error without resetting the split. Test with a runner that fails the query.

### 🟢 Low — W7: two boards in one tmux server (`src/runtime/mod.rs::teardown_tmux_for_tui`; `src/runtime/editor.rs::exec_pop_out_editor`)

**Issue:** teardown unbinds Prefix+Space and Prefix+e server-wide, so quitting one board strips the other's keys. The editor's focus restore uses `select_window(TUI_WINDOW_NAME)`, a server-wide name lookup that fails as ambiguous with two boards (H6's class).

**Fix:** restore focus by `self_pane` (`$TMUX_PANE`, read at the entry point). For the unbinds, see Decisions.

### 🟢 Low — W8: a signal skips the tmux teardown; a relaunch can kill a pinned agent (`src/runtime/store_target.rs::clean_up_on_termination`; `src/startup/retire.rs::retire_board_window_with`) — plausible

**Issue:** SIGHUP/SIGTERM calls `process::exit(0)`, skipping `await_split_restores`, the unbinds and leaving the alternate screen. `retire_board_window_with` kills the old board window without breaking a pinned agent pane out, so a relaunch while a task is pinned kills that agent.

**Fix:** run the split restore before exiting on a signal, and break out a joined agent pane before the retire kill. Check `split-pane.allium` first; add the rule if it is silent.

## Decisions

### W7: key unbinds with two boards (`src/runtime/mod.rs::teardown_tmux_for_tui`)

**Question:** should quitting one board leave the key bindings when another board runs in the same tmux server?

**Options:** (a) skip the unbind when another board window exists (one more tmux query at quit); (b) leave bindings as they are and document that the last board to quit removes them; (c) keep today's behaviour (no change, the other board loses its keys until restart).

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/agent-health.allium` | `DetectCrashedAgent` and sub-status reclassification only for this host's tasks |
| `docs/specs/split-pane.allium` | a failed pane query changes nothing; signal exit restores the split; retire breaks out a pinned pane |
| `src/tui/update/agent.rs` | host filter in `tick_window_checks`, `tick_sub_status`; guard in `handle_window_gone` |
| `src/tmux.rs` | `pane_exists` returns `Result` |
| `src/runtime/split.rs` | ignore `Err` in `exec_check_split_pane`; respawn error does not reset |
| `src/runtime/editor.rs` | focus restore by own pane |
| `src/runtime/mod.rs` | unbind per the decision |
| `src/runtime/store_target.rs`, `src/startup/retire.rs` | split restore on signal; break out before kill |
| tests under `src/tui/update/tests/`, `src/runtime/tests/` | foreign-host tick tests; failing pane query |

## Verification

- [ ] `./scripts/verify.sh` passes (run in the foreground; first run can take ~6 minutes)
- [ ] New tests fail before the fix and pass after
- [ ] `allium check docs/specs/agent-health.allium docs/specs/split-pane.allium`
