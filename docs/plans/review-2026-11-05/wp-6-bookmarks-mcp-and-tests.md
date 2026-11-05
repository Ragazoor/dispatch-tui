# Bookmarks MCP and tests

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** `update_tasks` with a bookmark is one change or an honest partial result, stray arguments are refused, and the bookmark rules are covered on both stores.

## Context

Findings B2, B3, B4 and B5 from `docs/plans/2026-11-05-codebase-review.md` §3.3 (bookmarks, `732f22e7`, `a3ae7f13`, `b910d6d5`). B1 (loss of sight) is in the "Loss of sight: claims and marks together" package; coordinate if both touch `bookmark.rs`. Read `docs/mcp-tool-design.md` before changing a tool. Spec: `docs/specs/mcp-task-tools.allium: UpdateTaskViaMcp`, `core.allium: Bookmark`.

## Findings

### 🟡 Medium — B2: `update_tasks` with fields and `bookmarked` is two reducer calls (`src/mcp/handlers/tasks/crud.rs::handle_update_task`)

**Issue:** it runs `update_task`, then `set_bookmark`. If the second is refused (identity not settled, or another host closed the task between them), the title is written but the reply is an error and `notify_task_changed` is skipped. `UpdateTaskViaMcp` has both effects in one rule.

**Fix:** carry an optional bookmark in the update reducer (`update_task_with_epic` or a new one; module + twin + conformance; rules-version bump), or check the bookmark's preconditions before the first write and report a partial success honestly. Prefer the single reducer.

### 🟢 Low — B3: a lone `url_type` with `bookmarked` is silently dropped (`handle_update_task`)

**Issue:** the guard `if !fields.is_empty() || bookmarked.is_none()` skips `update_task` for `{url_type: "pr", bookmarked: true}`; `url_type` is neither applied nor refused ("Task N updated (bookmarked)"). Without `bookmarked` the same call is refused.

**Fix:** refuse `url_type` without `url` before branching. Test.

### 🟢 Low — B4: conformance gaps and a dead "done write" (`tests/memory_caller_conformance/bookmarks.rs`; `src/sync/tests/queries.rs`; `spacetime/module/src/feed_ingest.rs::ReducerStore::update_task`)

**Issue:** conformance covers set, clear, a missing task and a merged PR, not `move_epic_to_workspace`, view parity after loss of sight, `batch_delete`/`delete_epic`, or member removal on the twin. The required-source list in `src/sync/tests/queries.rs` does not name `own_bookmarks`. 9450c870's `end_marks_on_finish` in `ReducerStore::update_task` (and the twin's `TablesIngest::update_task`) is on the routing move, which never sets Done.

**Fix:** add the cases and `own_bookmarks` to the list; reword the comment (or remove the dead call) and correct the claim.

### 🟢 Low — B5: `drop_marks_where` scans whole tables; operator exception unstated (`spacetime/module/src/bookmark.rs::drop_marks_where`; `require_own_identity`, `visible_task`)

**Issue:** since 10f17fd9 member removal iterates all of `unseen_tasks` and `bookmarks` with no index (the bookmark half used the `subscriber` index before). `require_own_identity` and `visible_task` pass for the operator, while `SetBookmark` says `subscriber = caller()`.

**Fix:** take an optional subscriber and use the index when given. For the operator, see Decisions.

## Decisions

### B5: may the operator write another person's bookmark? (`docs/specs/core.allium: SetBookmark`)

**Question:** should the operator identity be able to set or clear anyone's bookmark on any task?

**Options:** (a) forbid — `set_bookmark` requires `subscriber == caller` even for the operator; consistent with "bookmarks are personal"; (b) allow and state it in the spec — keeps repair power for the operator; (c) leave as is (code allows, spec silent).

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/mcp-task-tools.allium`, `core.allium` | one-change update with bookmark; operator rule |
| `src/mcp/handlers/tasks/crud.rs` | single write or honest partial; `url_type` refusal |
| `spacetime/module/src/{bookmark,tasks_epics,feed_ingest}.rs` | bookmark on update; index; comment |
| `src/sync/memory_caller/` | mirror |
| `tests/memory_caller_conformance/bookmarks.rs`, `src/sync/tests/queries.rs` | new cases |

## Verification

- [ ] `./scripts/verify.sh` passes
- [ ] MCP handler tests for B2 and B3 fail before the fix
