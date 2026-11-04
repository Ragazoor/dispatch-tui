# Feed teardown and requests

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make cross-host feed teardown kill the agent's real window, stop other members from aiming it, and make request serving survive transport errors and duplicate boards.

## Context

Findings Q1–Q9 from `docs/plans/2026-11-04-codebase-review.md` §3.2, all about 247dddce (cross-host feed requests). Behaviour changes start in the spec (`docs/specs/store-split.allium`, `feeds.allium`), then tests, then code. Every module change is mirrored in `src/sync/memory_caller/` with a case in `tests/memory_caller_conformance/`; bump `RULES_VERSION` (`spacetime/module/src/rules.rs`) and `BOARD_RULES_VERSION` (`src/sync/rules_gate.rs`) together when a reducer rule changes. Rebuild the module (`./scripts/build-managed-module.sh`) and regenerate bindings if the schema changes.

## Findings

### 🔴 High — Q1: teardown rows never name the agent's window (`spacetime/module/src/feed_request.rs::new_teardown_request`, `::teardown_request_owed`; `feed.rs::delete_stale_feed_tasks_in_epic`)

**Issue:** the snapshot copies `task.tmux_window`, but `agent.rs::without_session_facts` blanks that column on every task write; the window lives on the current agent row. The row is also written after `delete_task_side_effects`, which deletes the agents. Every real teardown row has `tmux_window = ""`: host A1 removes the worktree while the agent keeps running in a deleted directory. A task with a window but no worktree owes no row. `StaleDeleteRequestsTeardownOnTheHoldingHost` says `current_agent(task).tmux_window`. Tests pass only because `spacetime/module-tests/tests/feed_requests.rs::hold` writes `tmux_window` on the task row.

**Fix:** in `delete_stale_feed_tasks_in_epic`, read the window from the agent tables (`session_window(&AgentTables(ctx), task.id)` or equivalent) before `delete_task_side_effects`, and pass it to `teardown_request_owed`/`new_teardown_request`. Mirror in `src/sync/memory_caller/feed_requests.rs`. Change the test helper to set the window on an agent row, and add a case for a window-only task.

### 🟡 Medium — Q2: any viewer can aim another host's teardown at an unrelated worktree (`feed_request.rs::write_teardown_request_if_owed`; `tasks_epics.rs::patch_task`, `rules.rs::check_task_patch`)

**Issue:** the snapshot takes `host`, `repo_path` and `worktree` from the task row, and `TaskPatch` lets anyone who sees the task set all three. Bob patches a feed task in Alice's epic to `host = A1`, `worktree = /home/alice/proj/.worktrees/999-other`; when the item leaves the feed, A1 runs `git worktree remove --force` and `git branch -D` on a live worktree.

**Fix:** see Decisions.

### 🟡 Medium — Q3: a failed claim drops a request for the life of the board (`src/feed/requests.rs::rows_to_serve`, `src/feed/mod.rs::serve_feed_requests`)

**Issue:** the id enters `served_requests` before `claim_feed_request` returns. On `Err` the row stays `requested` and is never looked at again; the same for a `running` row whose done/failed report fails. Teardown rows have no backstop. The set is never trimmed.

**Fix:** insert only after `Ok(true)`; remove the id on `Err`; drop ids whose row is no longer open in `rows`. Test with a caller that fails the first claim.

### 🟡 Medium — Q4: two boards on one host id can both serve a request (`feed_request.rs::validate_feed_request_claim`) — plausible

**Issue:** re-claiming a `running` row is always accepted. Two boards sharing a host id both run the teardown; the row ends `failed` though it succeeded. Refresh rows run two cycles.

**Fix:** accept a re-claim of a `running` row only after a grace period on `claimed_at` (state the constant in the spec), or stamp a per-process claim token and require it on report.

### 🟢 Low — Q5: `store-split.allium` contradicts itself on pruning

**Issue:** `RequestRowsArePruned` guidance says an ingest "prunes nothing"; `StaleDeleteRequestsTeardownOnTheHoldingHost` and `write_teardown_request_if_owed` prune. The open questions "Feed teardown rows a board never serves" and "prune emitters" are answered by the code.

**Fix:** reconcile the guidance; close or reword both open questions. Run `allium check`.

### 🟢 Low — Q6: a foreign-owner teardown is logged at debug as written (`src/feed/cycle.rs::FeedCycle::run`)

**Issue:** for a task on another person's host the module writes no row (KNOWN GAP), but the log says "a teardown request was written", at `debug`.

**Fix:** when the holding host's owner is not this board's identity (from the board's host rows), log at `warn` that the teardown is stranded; otherwise word the debug line truthfully.

### 🟢 Low — Q7: the twin's `request_feed_refresh` skips the sight check (`src/sync/memory_caller/feed_requests.rs::request_feed_refresh_body`)

**Fix:** use the twin's visible-epic helper; add a conformance case for a refresh on an unseen epic.

### 🟢 Low — Q8: a second `r` after the owner ran is answered "Requested" but nothing re-runs (`feed_request.rs::request_feed_refresh`)

**Fix:** coalesce only into a `requested` row; otherwise write a new row. Test both.

### 🟢 Low — Q9: `ingest_feed` does not check poll ownership (`spacetime/module/src/feed_ingest.rs::ingest_feed`)

**Issue:** a host whose claim is overridden mid-cycle still reconciles, overlapping the new owner.

**Fix:** refuse an ingest from a host that does not hold the epic's claim (mind the manual-refresh and setter paths), or state in `feeds.allium` why the overlap is safe.

## Decisions

### Q2: who may aim a teardown

**Question:** how to stop a viewer from steering a host's teardown?

**Options:**
- Refuse `host`/`repo_path`/`worktree` in `TaskPatch` unless the caller acts for the task's current host (and the new host). Small change to `check_task_patch`; may break any client that patches these today (check the board's own writes in `src/sync/encode.rs`).
- Snapshot the teardown from the module-stamped dispatch request row (`DispatchRequest.worktree`/`host`) instead of the task row. Leaves patching open but makes teardown trust only module-written data; needs the request row to still exist.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/store-split.allium`, `docs/specs/feeds.allium` | Q1 window source, Q2 rule, Q4 re-claim rule, Q5 reconcile, Q8 coalesce, Q9 ownership |
| `spacetime/module/src/feed_request.rs`, `feed.rs`, `feed_ingest.rs`, `rules.rs`, `tasks_epics.rs` | Q1, Q2, Q4, Q8, Q9 |
| `src/sync/memory_caller/feed_requests.rs` (and siblings) | mirror; Q7 sight |
| `src/feed/requests.rs`, `src/feed/mod.rs`, `src/feed/cycle.rs` | Q3, Q6 |
| `spacetime/module-tests/tests/feed_requests.rs` | window on agent row; new cases |
| `tests/memory_caller_conformance/` | Q1, Q2, Q7, Q8 cases |

## Verification

- [ ] `cargo test --no-fail-fast` green; `cargo clippy --all-targets -- -D warnings` clean
- [ ] `spacetime/module-tests` reducer tests green (needs `gcc-c++`, `openssl-devel`)
- [ ] `./scripts/check-spacetime-module.sh` green; module rebuilt and bindings regenerated if the schema changed
- [ ] A test fails before the Q1 fix (window on the agent row, teardown row names it)
