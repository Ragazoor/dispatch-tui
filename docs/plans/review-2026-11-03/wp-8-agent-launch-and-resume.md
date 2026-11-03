# Agent Launch and Resume

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** One resume per task at a time, build limits that respect the user's configuration, a scope launch that cannot leave an empty window, and waits that end.

## Context

Findings U1, U3, U4, U7, U8 from `docs/plans/2026-11-03-codebase-review.md`. Specs: `docs/specs/dispatch.allium` (`AgentBuildsRunFewerParallelJobs`, the scope's best-effort clause), the agent-tree spec. Spec first where behaviour changes, then tests that fail, then code. Never drive tmux by hand; test through `tests/tmux_harness/mod.rs`.

## Findings

### 🟡 Medium (plausible) — U1: two resumes can start two `claude --continue` in one worktree (`src/tui/update/agent.rs::handle_resume_task`, `src/dispatch/agents.rs::resume_agent_with_message`)

**Issue:** the only guard is `task.tmux_window.is_some()`, set when `Resumed` comes back; `tmux::has_window` runs before `new_window` in separate `spawn_blocking` calls. b306f148 adds Space on a detached row in the agent-tree pane, with no "resuming" state, so a second press is likely; a pane resume plus MCP `resume_task` also overlap.

**Fix:** an in-flight resume set on `App`; refuse (with a status message) while the task id is in it; clear on `Resumed` or error. Optionally ignore Space in the pane while `pending_jump == Some(id)`. Test two resumes back to back.

### 🟡 Medium — U3: `CARGO_BUILD_JOBS=8` overrides a repo's own `build.jobs` (`src/dispatch/mod.rs::agent_launch_env`)

**Issue:** cargo ranks the environment above `.cargo/config.toml`, so a repo or user that set `jobs = 2` to save memory is raised to 8; under 8 cores it oversubscribes.

**Fix:** default to `min(8, available_parallelism)`; do not set the variable when `build.jobs` is configured in the repo's or user's cargo config (or document the precedence in the spec if detection is not worth it). Update `AgentBuildsRunFewerParallelJobs`.

### 🟡 Medium (plausible) — U4: the scope check and the scope launch see different environments (`src/process/agent_scope.rs::prepare_agent_scope`, `AgentScope::wrap`)

**Issue:** the check runs `systemctl --user` from the board; the wrapped command runs in the tmux pane's shell with the tmux server's `XDG_RUNTIME_DIR`/`DBUS_SESSION_BUS_ADDRESS`, and `systemd-run` on `PATH` is never checked. A tmux server started from ssh or a TTY gives "Failed to connect to bus": the window holds no agent and the board does not say so, which the spec's best-effort clause forbids.

**Fix:** probe `systemd-run --user --scope -- true` once per board (cached), and make the wrapped command fall back: `systemd-run … || exec claude …`. Test the fallback string.

### 🟢 Low — U7: a resumed agent's wait never times out (`src/agent_tree/state.rs::adopt_agent_list`)

**Issue:** `Accepted` means only "handed to the board"; a failed launch leaves `pending_jump` set for ever with no notice in the pane.

**Fix:** a deadline on `pending_jump` (use the timing-constant conventions, no wall-clock sleeps in tests), then a pane notice.

### 🟢 Low — U8: scope and sccache start sit outside the dispatch deadline (`src/dispatch/request_host.rs::DeadlineRunner`)

**Issue:** `agent_scope` and `start_sccache_server` delegate to the inner runner, so two 10 s `systemctl` calls and an untimed `sccache --start-server` run outside the request deadline on every launch, and warn on every launch on macOS.

**Fix:** cache the scope decision per board (shared with U4's probe); put a timeout on the sccache start.

## Changes

| File | Change |
|------|--------|
| `src/tui/update/agent.rs`, `src/tui/types.rs` | In-flight resume set |
| `src/agent_tree/state.rs` | `pending_jump` deadline and notice |
| `src/dispatch/mod.rs` | `CARGO_BUILD_JOBS` default |
| `src/process/agent_scope.rs` | Cached probe; fallback |
| `src/dispatch/request_host.rs` | Deadline for scope and sccache |
| `docs/specs/dispatch.allium` | U3, U4 rules |

## Verification

- [ ] New tests fail before the change and pass after
- [ ] `cargo test --no-fail-fast` green, with `tmux` on PATH
- [ ] `./scripts/check-no-test-sleep.sh` clean
- [ ] `cargo clippy --all-targets -- -D warnings` clean
