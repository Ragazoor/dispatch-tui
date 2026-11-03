# Board Rows Under Read Enforcement

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Close the gaps the per-caller views left on the board side: a missing rules-version bump, false erases on update, request rows that vanish, a lost undecodable mark, and a twin that ignores the views.

## Context

Findings Y1–Y5 from `docs/plans/2026-11-03-codebase-review.md`. Specs: `docs/specs/sync.allium` (`AViewUpdateNeverErasesTheRow`, `SubscribeOnceIdentityIsSettled`), `docs/specs/store-split.allium`. Spec first for Y3, then tests that fail, then code. Rebuild `module.wasm` and bindings after a module change.

## Findings

### 🟡 Medium — Y1: `subscriptions` went private without a rules-version bump (`spacetime/module/src/rules.rs::RULES_VERSION`, `src/sync/rules_gate.rs::BOARD_RULES_VERSION`)

**Issue:** 2e681e34 made `subscriptions` private and moved the board to `own_subscriptions`, but both constants stayed 31 (set by 5edab153). A board built between those commits passes the gate, sends `SELECT * FROM subscriptions WHERE subscriber=…`, which a private table refuses; the subscribe fails and `SyncSession::attempt` retries forever with no "upgrade your board" message. A new board against a 31 module asks for a view that does not exist.

**Fix:** bump both to 32; add to the `RULES_VERSION` doc that a change to the board's subscription set (a table going private, a view added or removed) warrants a bump.

### 🟡 Medium — Y2: agents and poll claims are still erased by an update in an ignored column (`src/sync/rows.rs::SharedRows::retire_agent`, `::retire_poll_owner`; `src/sync/decode.rs::agent`, `::poll_owner`)

**Issue:** both compare decoded rows; the decoders drop `stop_pending_at` (agents) and `claimed_at` (poll owners). `agent_state.rs::try_record_stop` with live subagents rewrites only `stop_pending_at`; old and new decode equal, so the delete half of the update removes the agent and `current_agent(task)` answers `None` until the next write. A forced same-host `write_poll_owner_row` does the same to a claim. 904b8be7 fixed this for tasks and epics only.

**Fix:** keep and compare the raw row, as 904b8be7 did. Graduated check: a test that, for every table behind a view, sends an update that changes only a column the decoder drops and asserts the row survives (model on `view_updates.rs::a_task_update_in_an_ignored_column_*`), so a new table cannot repeat this.

### 🟡 Medium (plausible) — Y3: request and agent rows vanish when a task stops being held (`spacetime/module/src/views.rs::dispatch_requests_of_held`, `::session_requests_of_held`, `::agents_of_held`; `src/sync/sdk_connector/queries.rs::subscription_queries`)

**Issue:** before 87a38ea3 requests were served `WHERE host = …`; now only for held tasks. Scenario: I dispatch a teammate's task from an epic I follow, then unfollow it. `request_wrap_up` writes its row, but the read-back in `callers.rs` never finds it and the agent gets an error; my reactor never serves it, so the window is never torn down; `HostRestartFailsInterrupted*` cannot see it.

**Fix:** serve request rows (and agents on this host) by `host` among the hosts the caller owns (`hosts_owned_by`). State the rule in `sync.allium`. Add a live test: dispatch, unfollow, wrap up.

### 🟢 Low — Y4: an undecodable-to-undecodable update erases the mark (`src/sync/rows.rs::SharedRows::retire_task`, `::upsert_epic`)

**Issue:** fix for B7 (765bd988) → `retire_task` removes `undecodable_tasks[id]` whenever the old row fails to decode; an update is insert(new) then delete(old), so a second undecodable version loses the mark `DeleteEpicRefused`'s pre-check counts. `upsert_epic` keeps the old decoded epic on a decode error, which paired views can leave on the board.

**Fix:** remove the mark only when it belongs to the departing raw row; in `upsert_epic`'s error branch remove the epic, as `upsert_task` does. Tests for both.

### 🟢 Low — Y5: the twin does not apply the views to the board's rows (`src/sync/memory_caller/workspaces.rs::remove_member`, `src/sync/memory_caller/tasks_epics.rs`)

**Issue:** the twin upserts tasks and epics unconditionally, so a test over it still shows a removed member's tasks, a private-flipped epic or an unfollowed subtree; board-side loss-of-sight bugs like Y3 cannot fail there.

**Fix:** after each apply, recompute the twin's board rows from the `own_*`/`covered_*`/held view functions. If that is too large, record the gap in `docs/testing.md` and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `spacetime/module/src/rules.rs`, `src/sync/rules_gate.rs` | Bump to 32; doc line |
| `src/sync/rows.rs` | Raw compare for agents and poll owners; undecodable mark; `upsert_epic` error branch |
| `spacetime/module/src/views.rs`, `src/sync/sdk_connector/queries.rs` | Requests by owned host |
| `docs/specs/sync.allium` | Y3 rule |
| `src/sync/memory_caller/` | Views applied to board rows |
| `src/sync/tests/` | Per-table ignored-column update test; Y4 tests |

## Verification

- [ ] New tests fail before the change and pass after
- [ ] `cargo test --no-fail-fast` green, with `spacetime` on PATH
- [ ] Module checks per `docs/testing.md`; `module.wasm` and bindings rebuilt
- [ ] `cargo clippy --all-targets -- -D warnings` clean
