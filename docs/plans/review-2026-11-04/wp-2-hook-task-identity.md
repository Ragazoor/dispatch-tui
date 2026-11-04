# Hook task identity

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make a hook act only on the task its agent was launched for, and make the hook scripts portable and robust.

## Context

Findings H1, H7 and H10 from `docs/plans/2026-11-04-codebase-review.md` §3.7. Spec first: `docs/specs/agent-health.allium` (`HookDelivery`, `HookStop`, `HookPeerMessageSent`). Never drive tmux by hand; test hook scripts with temp git repos.

## Findings

### 🔴 High — H1: a hook takes its task id from the branch name, and the board trusts it (`plugin/hooks/scripts/task-status-hook`, `plugin/hooks/scripts/pr-learnings-hook`, `src/hooks/mod.rs::run_event`, `src/mcp/handlers/hooks.rs::apply`)

**Issue:** both scripts take the id from `git -C "$CWD" branch --show-current | grep -oP '^\d+'`, falling back to a `<digits>-` path segment. The `/hook` request carries no caller identity. The agent on task 42 runs `git checkout -b 7-hotfix` (or `cd`s into another checkout); its next Stop runs `try_record_stop(7)` and flips task 7 to Review while task 7's agent works; SessionStart clears task 7's subagents; `gh pr create` spends task 7's PR-gate reminder. Ids are global, so task 7 can be on another host. `HookDelivery` says the hook resolves from the session's worktree.

**Fix:** add `DISPATCH_TASK_ID` to `src/dispatch/mod.rs::agent_launch_env` (it reaches the pane through `new_window -e`, for fresh dispatch and resume). Both scripts prefer it, then fall back to the branch/path. Optionally have the board check the payload's `session_id` against the task's current agent and refuse a mismatch. Update `HookDelivery` to say where the id comes from. Add a hook-script test: cwd on another task's branch with `DISPATCH_TASK_ID` set resolves to the launch task.

### 🟢 Low — H7: hook scripts need GNU grep and abort early on a slow board (both plugin hook scripts)

**Issue:** `grep -oP` fails silently where grep has no `-P` (macOS), so every branch lookup is empty. In `task-status-hook`, `set -e` aborts when `dispatch hook … pre_tool_use` fails, so the AskUserQuestion notification and the SendMessage observer never run.

**Fix:** `grep -oE '^[0-9]+'`; guard the board call so later steps still run (`|| true` with a logged note). Keep the scripts bash 3.2-safe (see the U5 fix).

### 🟢 Low — H10: the peer-message audit append is detached and unchecked (`src/mcp/handlers/hooks.rs::append_peer_message_trajectory`)

**Issue:** a detached `tokio::spawn` whose failure is swallowed; the hook reports `Applied`. If the stamp fails, no trajectory entry is written for a send that happened.

**Fix:** state "best-effort" in `agent-health.allium: HookPeerMessageSent`, or await it and log at `warn` on failure.

## Changes

| File | Change |
|------|--------|
| `docs/specs/agent-health.allium` | H1 id source; H10 best-effort wording |
| `src/dispatch/mod.rs::agent_launch_env` | add `DISPATCH_TASK_ID` |
| `plugin/hooks/scripts/task-status-hook`, `pr-learnings-hook` | prefer `DISPATCH_TASK_ID`; `grep -oE`; robust board call |
| `src/mcp/handlers/hooks.rs` | optional session check; H10 |
| hook-script tests (see `tests/feed_scripts.rs` style or `src/setup/hooks/tests`) | H1 and H7 cases |

## Verification

- [ ] `cargo test --no-fail-fast` green; `cargo clippy --all-targets -- -D warnings` clean
- [ ] A test fails before the H1 fix and passes after
- [ ] `bash -n` and a bash 3.2-safe read of both scripts
