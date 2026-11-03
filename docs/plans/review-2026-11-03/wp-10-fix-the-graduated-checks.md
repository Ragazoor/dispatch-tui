# Fix the Graduated Checks

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** The two checks added in 4c85d7bd to stop repeated finding classes actually catch those classes.

## Context

Findings G1 and G2 from `docs/plans/2026-11-03-codebase-review.md`. The class "a sibling path forgets the rule its fix added" appeared in three review rounds; these checks were meant to stop it and do not. **Mutation-test each check**: re-introduce the original bug (for G1, remove `clear_unseen_for_task` from `grouping.rs::relink_task`; for G2, add a second wrapped `self\n.db\n.<write>(` call to an allow-listed-free service method) and confirm the check fails, then restore. Two related class tests ship with their fixes in sibling packages ("Feed Command Authority and Poll Claims": loss-of-sight releases claims; "Board Rows Under Read Enforcement": ignored-column updates per view table).

## Findings

### 🟡 Medium — G1: the move-path guard checks the opposite of the bug class (`tests/memory_caller_conformance/move_paths.rs::every_task_move_site_in_the_module_is_in_the_table`)

**Issue:** it collects functions that *call* `clear_unseen_for_task(` and requires each in the table. N1 and R2 were move paths that *forgot* the call, so R2's `relink_task` would have passed. The sight rule is exercised only with a missing epic (999_999), never another user's private one.

**Fix:** find the sites in `spacetime/module/src/` that write a task's `epic_id` (an update or insert of a `Task` with a changed `epic_id`) and require each in the table; every table entry then runs the unseen-mark and sight assertions. Add a private, unseen destination case.

### 🟡 Medium — G2: `check-single-write-service.sh` misses wrapped calls and free functions (`scripts/check-single-write-service.sh`; `src/service/managed_feeds.rs::write_managed_feed_settings`)

**Issue:** the awk matches `self.db.<m>(` on one line; rustfmt wraps many calls as `self\n.db\n.m(` (both `add_member` calls in `set_member`, `update_task`, `set_task_epic_id`, `patch_task` …), so they are not counted. It resets on any column-0 `fn` and counts only `self.db.`, so `write_managed_feed_settings` (four `db.set_*` writes) is invisible. Writes inside a loop count once; an allow-listed function can gain writes. Joining lines surfaces a false positive in `session.rs::report_session_failure` (`session_request` is a read).

**Fix:** normalise `self\s*\.\s*db\s*\.` across lines before counting (or rewrite the check as a Rust test using `syn`); count any `db.` receiver in every `fn`; record the expected write count in the allow-list; add `session_request` to READS. Then make `write_managed_feed_settings` one reducer call, or allow-list it with a reason.

## Changes

| File | Change |
|------|--------|
| `tests/memory_caller_conformance/move_paths.rs` | Guard on `epic_id` writers; private destination case |
| `scripts/check-single-write-service.sh` | Multi-line, free functions, counts in allow-list, READS |
| `src/service/managed_feeds.rs` | One write or allow-listed |
| `docs/testing.md` | Describe what each check catches, if it lists them |

## Verification

- [ ] Each check fails on the re-introduced bug, then passes once restored (record both runs at wrap-up)
- [ ] `./scripts/check-single-write-service.sh` clean on the tree
- [ ] `cargo test --no-fail-fast` green
- [ ] `cargo clippy --all-targets -- -D warnings` clean
