# Keep-awake hold

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the "awake" badge mean a hold really exists, and give a stuck request a way out.

## Context

Findings T1, T2 and T3 from `docs/plans/2026-11-04-codebase-review.md` §3.6, on bec9e4b2/14bdfd9e/2ac81f3e. Spec: `docs/specs/keep-awake.allium` (`BadgeMeansHeld`, `KeepAwakeLost`, `IgnoreToggleWhileRequesting`, `HoldDiesWithTheBoard`). No wall-clock sleeps in tests (`scripts/check-no-test-sleep.sh`).

## Findings

### 🟡 Medium — T1: the board can miss the end of a GNOME hold (`src/keep_awake.rs::start`) — plausible

**Issue:** the reader waits for EOF on both pipes (`tokio::join!`) before `child.wait()`. The inner `sh -c 'echo ready; exec cat'` inherits the pipes. `systemd-inhibit` kills its child when it dies; `gnome-session-inhibit` likely does not. If it dies alone, `cat` keeps the pipes open, `HoldEvent::Ended` never fires, and the badge stays with no hold.

**Fix:** race `child.wait()` against the pipe reads (`tokio::select!`) and send Ended/Refused when the helper exits; or spawn the helper in its own process group and signal the group on exit and drop. Test with a fake helper that exits while a grandchild holds stdout.

### 🟢 Low — T2: GNOME detection reads the tmux server's environment (`src/keep_awake.rs::hold_command_for_this_machine`) — plausible

**Issue:** a tmux server started from ssh, a TTY or a user unit has no `XDG_CURRENT_DESKTOP`; on GNOME the board picks `systemd-inhibit --what=idle:sleep`, which GNOME ignores for lock and blanking, while the badge says "awake". The `OnceLock` keeps the choice.

**Fix:** also check `XDG_SESSION_DESKTOP`/`DESKTOP_SESSION`, or prefer `gnome-session-inhibit` whenever it is on `PATH` and fall back on refusal. Say which in the spec.

### 🟢 Low — T3: no escape from `Requesting` (`src/tui/update/keep_awake.rs::handle_toggle_keep_awake`)

**Issue:** a helper that neither prints `ready` nor exits (a polkit prompt, a hung D-Bus call) leaves the state `Requesting`; every `W` is ignored with no message.

**Fix:** see Decisions.

## Decisions

### T3: how a stuck request ends

**Question:** what ends a request that never settles?

**Options:**
- `W` while Requesting cancels: drop the hold, bump the generation, go to Off. Changes `IgnoreToggleWhileRequesting`; user-driven.
- A request timeout (a named constant) that turns into Refused with a message. Automatic; one more timing constant.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/keep-awake.allium` | T1 end detection, T2 detection rule, T3 decision |
| `src/keep_awake.rs` | T1, T2 |
| `src/tui/update/keep_awake.rs` | T3 |
| tests next to each | one per finding |

## Verification

- [ ] `cargo test --no-fail-fast` green; `cargo clippy --all-targets -- -D warnings` clean
- [ ] `./scripts/check-no-test-sleep.sh` green
