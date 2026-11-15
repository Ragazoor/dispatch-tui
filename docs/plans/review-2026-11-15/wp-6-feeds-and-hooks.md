# Feeds and hooks: kept feed tasks, the Stop hook, hook authority

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** a kept feed task can be worked on again, a Stop is never lost to an odd payload, and the hook reducers judge time and authority in the module.

## Context

Findings J1–J4 and V4 from `docs/plans/2026-11-15-codebase-review.md` §3.2 and §3.5. Spec first (`docs/specs/feeds.allium: KeepWorkedOnFeedTask`, `agent-health.allium` hook rules), then tests, then code. Mirror module changes in `src/sync/memory_caller/`; bump both rules versions together.

## Findings

### 🟡 Medium — J1: a kept feed task someone resumes is moved back to done every cycle, killing its agent (`spacetime/module/src/feed.rs::left_feed_moves_to_done`, `::keep_left_feed_task`; twin `src/sync/memory_caller/feed.rs::keep_left_feed_task`) — decision item

**Issue:** `left_feed` is cleared only when the item returns, and `left_feed_moves_to_done` answers "finishing" for any running or review task. A merged review's card sits in done; the operator resumes it; the next cycle patches it to done and writes a close row, killing the window mid-work. Moving the kept card to backlog deletes it next cycle.

**Fix:** see Decisions. Conformance case for the resume.

### 🟢 Low — J2: the Stop hook can exit before recording the Stop (`plugin/hooks/scripts/task-status-hook`, `Stop)` arm)

**Issue:** under `set -euo pipefail`, `COUNTS=$(… jq -r '[.background_tasks // [] | .[].type] …')` exits on any non-object element, before any `dispatch hook … stop`. `stop_pending` stays false, `expire_background_wait` requires it, so the task stays running until moved by hand. The fallback plain stop fires on any failure, not only an old binary's usage error, and can flip a still-waiting task to review.

**Fix:** `COUNTS=$(… 2>/dev/null) || COUNTS=""` and `.type?`; fall back only on exit code 2. A test in `tests/hook_scripts.rs` with a string element in `background_tasks` (spawn through `plugin_script_command`).

### 🟢 Low — J3: feed predict-then-verify reads a task that left the view as deleted (`src/store/queries/mod.rs::confirm_removed`) — plausible

**Issue:** a local feed task whose epic goes private or moves during the ingest round trip is reported removed, and `FeedCycle` tears down its worktree and window while the task exists.

**Fix:** confirm absence through a sight-independent read (or the ingest's own outcome), or skip the teardown when the row is merely invisible.

### 🟢 Low — J4: a PreToolUse hook for an undecodable task is dropped whole and logged per call (`src/service/tasks/crud.rs::TaskService::record_hook_event`; `src/mcp/handlers/hooks.rs::observe`)

**Issue:** since e0ec5e61, when this board cannot decode the task the PreToolUse arm returns before `record_pre_tool_use`; the agent's activity and sub_status freeze on every board, and `app.log` gets one warning per tool call.

**Fix:** still stamp the activity (the reducer checks `running` itself); warn once per task id.

### 🟢 Low — V4: the hook reducers trust the caller's clock and host (`spacetime/module/src/agent_state.rs::expire_background_wait`, `::background_wait_cutoff`, `::try_record_stop`)

**Issue:** the cutoff derives from a client `now` while the comment says "a board cannot shorten the wait"; none checks `require_acting_for` on the task's host, so any viewer can expire another person's wait or hold their task running for 30 minutes with `shells = 1`.

**Fix:** use `ctx.timestamp`; require the caller to act for `task.host` (an agent's hooks reach the store through its own host's board); correct the comment; conformance case.

## Decisions

### J1: what does a kept feed task do once someone works on it again?

**Question:** a task kept in done after its feed item left (`left_feed = true`) is resumed or moved by hand. What should later feed cycles do with it?

**Options:** (a) move it to done only on the first absence (`!task.left_feed`), never again — smallest change; a resumed task is then never closed by the feed; (b) clear `left_feed` on any status change made by hand, so the next absence counts as new — the feed may close it again after the follow-up ends; (c) a hand move out of done detaches the task from the feed (clears `external_id`) — it becomes an ordinary task, and the backlog-move deletion goes away too.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/feeds.allium`, `agent-health.allium` | J1 decision; hook time and authority |
| `spacetime/module/src/{feed,agent_state}.rs` | J1; V4 |
| `src/sync/memory_caller/{feed,agent_state}.rs` | mirror |
| `plugin/hooks/scripts/task-status-hook`, `tests/hook_scripts.rs` | J2 |
| `src/store/queries/mod.rs` | J3 |
| `src/service/tasks/crud.rs`, `src/mcp/handlers/hooks.rs` | J4 |
| `spacetime/module/src/rules.rs`, `src/sync/rules_gate.rs` | bump together |

## Verification

- [ ] `./scripts/verify.sh` passes
- [ ] The J2 hook test fails before the script fix
