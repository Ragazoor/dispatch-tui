# Poll claims and held tasks

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Stop pre-rule and task-scoped poll claims from stalling polls, and serve the task a host's request rows refer to.

## Context

Findings M9, M10, M12 and M15 from `docs/plans/2026-11-04-codebase-review.md` §3.3; all are regressions in fixes for the 2026-11-03 findings M2, M4, Y3 and Y4. Spec first (`feeds.allium`, `workspaces.allium`, `sync.allium`). Mirror every module change in `src/sync/memory_caller/` with a conformance case; bump `RULES_VERSION` and `BOARD_RULES_VERSION` together (a change to the board's subscription set or views warrants a bump).

## Findings

### 🟡 Medium — M9: claims held before the setter rule stall shared feeds for good (`src/models/poll_action.rs::decide_feed_poll_action`, `spacetime/module/src/agent_state.rs::check_feed_claim_by`, `src/feed/mod.rs::FeedRunner::this_host_polls`)

**Issue:** only new claims are gated by the setter rule. Alice set a shared feed's command; Carol's host claimed it before rules 32. After the upgrade Carol's host may not run it and skips; Alice's host sees Carol's claim and skips. The feed stops silently; only a manual override recovers it.

**Fix:** see Decisions. Add the case under `feeds.allium: FeedCommandRunsOnSettersHost`.

### 🟡 Medium — M10: task-scoped poll claims outlive sight (`spacetime/module/src/config.rs::release_poll_claims`)

**Issue:** `release_poll_claims` filters to epic scope, while a task claim now needs `visible_task`. Carol holds task T's PR-poll claim and is removed from the workspace (or T's epic moves under Bob's private epic); nobody else may claim T and its PR status stops updating.

**Fix:** extend `ClaimsFollowSight` to task scope over the tasks in the affected subtree or workspace; mirror in `src/sync/memory_caller/config.rs`; add task-scope cases to both loss-of-sight tests (`spacetime/module-tests/tests/feed_authority.rs`, `src/sync/memory_caller/tests/feed_authority.rs`).

### 🟡 Medium — M12: the request arrives but its task does not (`spacetime/module/src/views.rs::held_tasks_of`, `::subagents_of_held`; `src/service/tasks/session.rs::serve_rebase`, `src/service/tasks/dispatch.rs`)

**Issue:** a9f80e70 serves request and agent rows by owned host, but `held_tasks_of` is own + covered only. I dispatch a teammate's task from an epic I follow, then unfollow; the agent calls `wrap_up`; my reactor gets the request, `get_task` answers None, the rebase fails as "task deleted". Subagent rows of that task vanish from the agent tree.

**Fix:** serve a task that has a live agent or an open request on a host the caller owns (an arm by agent/request host); same rule for `subagents_of_held`. Views test in the module; mirror in `src/sync/memory_caller/views.rs`; state the rule in `sync.allium`.

### 🟢 Low — M15: Y4's sibling error branches keep stale rows (`src/sync/rows.rs::upsert_learning`, `::upsert_learning_retrieval`, `::upsert_work_package`, `::upsert_work_package_link`)

**Fix:** on a decode error remove the row, as `upsert_task`/`upsert_epic` now do. One test per branch, or one table-driven test.

## Decisions

### M9: what to do with a claim held by a non-setter host

**Question:** how should a pre-rule claim held by a host whose owner is not the feed's setter be treated?

**Options:**
- Treat it as absent: `decide_feed_poll_action` on the setter's host proceeds, and `claim_poll_owner` by the setter replaces it. Permanent rule, no migration; small module and board change.
- One-time sweep: release such claims when the store moves to the new rules version. No lasting rule, but a sweep that must not skip rows and needs a version bump.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/feeds.allium`, `workspaces.allium`, `sync.allium` | M9, M10, M12 rules |
| `spacetime/module/src/agent_state.rs`, `config.rs`, `views.rs` | M9, M10, M12 |
| `src/models/poll_action.rs`, `src/feed/mod.rs` | M9 board side |
| `src/sync/memory_caller/{config,views,agent_state}.rs` | mirrors |
| `src/sync/rows.rs` | M15 |
| `tests/memory_caller_conformance/`, module-tests | cases for each |

## Verification

- [ ] `cargo test --no-fail-fast` green; `cargo clippy --all-targets -- -D warnings` clean
- [ ] `spacetime/module-tests` green; `./scripts/check-spacetime-module.sh` green; module rebuilt, bindings regenerated if needed
- [ ] Rules versions bumped together if a rule or view changed
