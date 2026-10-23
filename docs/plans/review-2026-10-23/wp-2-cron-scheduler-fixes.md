# Cron Scheduler Fixes

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make every accepted cron keep recurring, close two edge cases in parsing and the in-memory twin, and settle what happens to a schedule when a task leaves its normal path.

## Context

This work package addresses findings from the code review in `docs/plans/2026-10-23-codebase-review.md` (C1–C5), all in the scheduler added by 054df327. Spec: `docs/specs/schedules.allium`. Every module change needs the twin (`src/sync/memory_caller/`) to match and a case in `tests/memory_caller_conformance.rs`; rebuild `module.wasm` with `./scripts/build-managed-module.sh` and regenerate bindings if the schema moves. A rule change (C4, C5) needs `RULES_VERSION` and `BOARD_RULES_VERSION` bumped together.

## Findings

### 🟡 Medium: An accepted schedule can stop recurring with no message (`spacetime/module/src/schedule.rs::SEARCH_DAYS`)

**Issue:** `SEARCH_DAYS` is `366 * 8 + 2`; its doc says 8 years is the longest gap between matches. That holds only when day of month and day of week combine with OR. A day-of-month field starting with `*` (e.g. `*/31`, which means {1}) combines with AND, and gaps reach 11 years. `0 0 */31 2 1` (1 February when it is a Monday) matches 2027, 2038, 2044. `apply_schedule` searches from now, so set in 2026 it is accepted and fires in 2027; the phoenix respawn then searches from 2027, finds nothing within 8 years, and `session_request.rs::respawn_phoenix_successor_of` returns false with no successor and no error. `schedule_after_failed_launch` re-arms with `next_cron_match(...).unwrap_or_default()` (`""`), and its comment claims a stored schedule always answers. Set in 2028–2037 the same cron is refused as "never matches".

**Fix:** Search the 28-year Gregorian weekday cycle plus the century leap margin (about 366 × 28 + a few days; still cheap, it steps by day), and fix the doc comments on `SEARCH_DAYS` and `VALIDATION_ORIGIN`. Remove the `unwrap_or_default` claim or make the empty result visible as a `schedule_error`. Tests: the AND case above from 2027, and `0 0 29 2 *` across 2096→2104.

### 🟢 Low: The step add in `parse_field` can overflow (`spacetime/module/src/schedule.rs::parse_field`)

**Issue:** `v += step` on `u32`. `1-59/4294967295 * * * *` panics in debug builds (twin, tests) and wraps in release wasm, giving minutes {0, 1} instead of {1}.

**Fix:** `match v.checked_add(step) { Some(n) => v = n, None => break }`. Test the cron above parses to {1}.

### 🟢 Low: The twin's `create_task` leaves a repo sub-epic behind when it refuses a cron (`src/sync/memory_caller/tasks_epics.rs::apply_create_task`)

**Issue:** The twin routes the task (which can create a per-repo sub-epic via `find_or_create_repo_group`) before `write_task` refuses the bad cron. `validate_task_write`, whose doc promises every refusal `write_task` can make, does not check the cron. The module rolls back; the twin keeps the sub-epic.

**Fix:** Run the cron check (`arm_created_task` / `apply_schedule` on a copy) in `validate_task_write` for creates, before routing. Add a conformance case: create into an auto-grouping epic with `0 0 31 2 *`; both sides refuse and neither has a new sub-epic.

### 🟢 Low: A phoenix task that returns to backlog by hand is never re-armed (`spacetime/module/src/schedule.rs::apply_schedule`, `spacetime/module/src/tasks_epics.rs::patch_task`)

**Issue:** A phoenix task fires (`next_due` consumed), the run is abandoned, and the user moves it back to backlog. It has a schedule, no `next_due` and no `schedule_error`: it never fires again and the card shows nothing wrong. The spec allows this (re-arming only on done → respawn or a new schedule), so it is a silent trap, not a divergence. **Decision item** — see below.

**Fix:** Per the decision. Spec first in `docs/specs/schedules.allium`.

### 🟢 Low: A close leaves stale schedule state on Done rows (`spacetime/module/src/session_request.rs::close_task`)

**Issue:** `close_task` → `fail_open_dispatch_requests_of` → `task_after_failed_request`. (1) An open schedule-origin request at close writes `schedule_error = "task closed"` onto the Done row, hidden today only because `schedule_failed()` checks for backlog. (2) An open user-origin request leaves a passed `next_due` on the Done row, so moving it back to backlog fires it at once, against the spec's "a one-off is spent".

**Fix:** Clear `next_due` on every close whatever the prior status (`apply_done_close`, `apply_session_close`), and skip the schedule mark when the task is no longer a bare claim. Mirror in the twin; conformance cases for both scenarios.

## Decisions

### Re-arming a phoenix task moved back to backlog (`spacetime/module/src/schedule.rs::apply_schedule`)

**Question:** When a phoenix task with a schedule comes back to backlog by any write other than the respawn, should the store re-arm it?

**Options:**
- (a) Re-arm on any write into backlog when phoenix and a schedule are set. Fixes the trap; a module rule change, spec edit and `RULES_VERSION` bump; the task fires at its next cron match.
- (b) Leave it unarmed but show "schedule unarmed" on the card (`docs/specs/board-visuals.allium`). No module change; the user must re-set the schedule.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/schedules.allium` | C4 decision; close clears `next_due` (C5) |
| `spacetime/module/src/schedule.rs` | Longer search window, doc comments, `checked_add`, C4 per decision |
| `spacetime/module/src/session_request.rs` | Clear `next_due` on close; no schedule mark on a non-claim |
| `src/sync/memory_caller/tasks_epics.rs` (+ twin session code) | Cron check before routing; mirror C4/C5 |
| `spacetime/module/src/rules.rs`, `src/sync/rules_gate.rs` | Bump together if C4(a) or C5 changes a rule |
| `spacetime/module/src/tests/schedules.rs`, `spacetime/module-tests/tests/schedules.rs`, `tests/memory_caller_conformance/schedules.rs` | Tests per finding |
| `src/spacetime/module.wasm` (+ source hash) | Rebuild |

## Verification

- [ ] Run the task's verify command — all pass
- [ ] `cargo test --no-fail-fast` green; module-tests crate green (`docs/testing.md`, "SpacetimeDB module and CI details")
- [ ] `./scripts/build-managed-module.sh --check` passes
