# Visibility Conformance

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Pin module-vs-twin agreement for the visibility reducers and views, and end unseen marks on repo-group relinks.

## Context

This work package addresses findings V10 and R3 from `docs/plans/2026-10-31-codebase-review.md`, plus the `not_applicable` version-bump question. The twin runs every MCP and TUI test, so without these cases the twin can drift from the module unseen. Coordinate with WP "Private moves" and "Unseen listing and migration" (same epic): they add cases to the same files; merge `main` often.

## Findings

### 🟡 Medium — V10: conformance gaps for the visibility pairs (`tests/memory_caller_conformance/`, `src/sync/memory_caller/visibility.rs`, `src/sync/memory_caller/views.rs`)

**Issue:** Only `move_epic.rs` and `learning_placement.rs` run with `with_workspace_rules()`. No module-vs-twin case covers `set_epic_visibility`, `set_work_package_visibility`, `super_admin_delete_unseen_item`, the unseen-is-missing refusals on writes, the creator stamp on private epic creation, 06874206's refusal for someone else's private sub-epic, dropping follows/marks/links on a move, or the views. `src/sync/memory_caller/views.rs`'s module doc claims a live comparison (`tests/memory_caller_conformance.rs`) that does not exist; `view_keys` is used only in `src/sync/memory_caller/tests/views.rs`. The twin's `Tables::watchers_of` returns `Vec::new()` while the module reads both watcher indexes.

**Fix:** Add one scenario per reducer above, comparing rows (including `rows.unseen_items()`) on both sides. Either add a live view comparison (subscribe as two identities, compare each view's keys with `view_keys`) or correct the doc. Implement `watchers_of` in the twin.

### 🟡 Medium — R3: repo-group relinks keep the unseen mark (`spacetime/module/src/grouping.rs:131`)

**Issue:** `relink_task` changes `epic_id` without `clear_unseen_for_task`, against `tasks.allium::UnseenMarkEndsWithTheTaskLeavingItsEpicOrFinishing`. Used by `reroute_on_repo_change` (from `patch_task_row` on a repo change) and `regroup_epic`. The twin `src/sync/memory_caller/grouping.rs` has the same gap. N1 (2e523c8d) fixed only the feed path.

**Fix:** Clear marks in `relink_task`, module and twin; conformance case "a repo edit that moves a marked task clears the mark". Add one line to `docs/invariants.md`: every write that changes `epic_id` calls `clear_unseen_for_task`.

## Decisions

### Does widening accepted input need a `RULES_VERSION` bump? (`spacetime/module/src/rules.rs`)

**Question:** a76abc16 made the module accept a new `rate_learning` verdict (`not_applicable`) without bumping `RULES_VERSION`. A new board on an older module passes the version gate and is then refused. Should accepting a new value count as a rule change?

**Options:**
- (a) Yes: say so in the `rules.rs` doc comment; this WP's own bump covers it from now on. Costs nothing extra now.
- (b) No: record the exemption in the `rules.rs` comment, next to the existing "bug fix inside a rule" one.

Ask the user with `AskUserQuestion` before implementing this decision. Do not pick an option yourself. If the user does not answer, leave it unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `tests/memory_caller_conformance/` | New visibility and relink scenarios |
| `src/sync/memory_caller/visibility.rs` | `watchers_of` reads both indexes |
| `src/sync/memory_caller/views.rs` | Correct the module doc (or add live comparison) |
| `spacetime/module/src/grouping.rs`, `src/sync/memory_caller/grouping.rs` | `relink_task` clears marks |
| `docs/invariants.md` | Epic change ends unseen marks |
| `rules.rs` / `rules_gate.rs`, `module.wasm`, bindings | Bump together; rebuild |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass, no `skipping:` lines
- [ ] The relink case fails before the fix and passes after
- [ ] After merge to main: `./scripts/rotate-previous-store.sh`
