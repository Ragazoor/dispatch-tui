# Runtime editor, refresh and drain thread

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** The task editor writes only what the user changed and never shows a refused edit, quitting does not hang, the tick refresh reads every table, and runtime helpers stop splitting one change into two writes or blocking the drain.

## Context

Findings W3, W4, W5, W6, W9 and W10 from `docs/plans/2026-11-05-codebase-review.md` §3.4 (fresh area `src/runtime/`). Read `docs/architecture.md` (command queue draining, editor session invariant) first. UI behaviour changes start in the spec (`docs/specs/tasks.allium` editor rules, `epics.allium`).

## Findings

### 🟡 Medium — W3: saving the editor writes a stale snapshot (`src/runtime/editor.rs::finalize_task_edit`, `apply_task_editor_fields`, `::finalize_epic_edit`)

**Issue:** the `EditKind::TaskEdit` snapshot is taken when the editor opens; the text always carries `--- STATUS ---`; `UpdateTaskParams` always sends status, title, model, base branch, tag, wrap-up mode and phoenix. An agent wraps up to Review while you fix a description; saving puts it back to Running. A `set_model` made meanwhile is reverted. `finalize_epic_edit` does the same for title, description, feed command and interval.

**Fix:** compare parsed fields with the snapshot and send only those the user changed (`None` for the rest). Test: a status change between open and save survives a description-only save.

### 🟡 Medium — W4: a refused task edit is still shown (`editor.rs::finalize_task_edit`)

**Issue:** on `Err` from `update_task` (e.g. `check_task_aim`), it reports the error and still applies `TaskMessage::Edited` and calls `exec_save_repo_path`. `finalize_epic_edit` returns early and documents why.

**Fix:** return early after reporting. Test with a failing service.

### 🟡 Medium — W5: quitting can hang on blocking threads (`src/runtime/mod.rs::spawn_input_thread`; `src/main.rs`) — plausible

**Issue:** the input loop leaves only when `key_tx.send` fails on a key event; `main` drops the multi-thread runtime with no `shutdown_timeout`, which waits for every `spawn_blocking` task. After quit the process waits for the next keypress, or for an open pop-out editor (the watcher in `exec_pop_out_editor`) to close.

**Fix:** check `key_tx.is_closed()` (or a shutdown flag) every poll; stop the editor watcher on shutdown; `runtime.shutdown_timeout(..)` in `main`.

### 🟡 Medium — W6: the tick refresh is not the backstop the docs say (`src/runtime/tasks.rs::exec_refresh_from_db`; `reload_board`; `docs/architecture.md`)

**Issue:** the tick path reads tasks, epics, unseen, bookmarks and dispatch requests; `reload_board` also reads work packages and links, repo paths and base branches. `docs/architecture.md` says "Both paths read the same set of tables".

**Fix:** one shared table list used by both paths (preferred), or correct the doc. A test that both paths request the same tables.

### 🟢 Low — W9: partial writes and dropped errors (`src/runtime/epics.rs::exec_toggle_epic_group_by_repo`; `src/runtime/tasks.rs::exec_quick_dispatch`)

**Issue:** the toggle writes `update_epic(group_by_repo)` then `regroup_epic`/`flatten_epic`; a failed second call leaves the flag set and tasks unmoved. Quick dispatch does `let _ = save_repo_path(..)` and leaves a Backlog task when the dispatch request fails after `create_task`.

**Fix:** one reducer that sets the flag and regroups (module + twin + conformance; rules-version bump); in quick dispatch, log the save failure and report the orphan task (or delete it) on a failed request.

### 🟢 Low — W10: blocking work on the command-drain path (`tasks.rs::grant_trust`, `::check_trusted`; `src/tui/update/system.rs::handle_repo_paths_updated`)

**Issue:** `grant_trust`/`check_trusted` await `spawn_blocking` on `~/.claude.json` (several MB) on the drain; `handle_repo_paths_updated` calls `Path::is_dir()` per path inline on every pushed row. A stalled mount freezes the board.

**Fix:** spawn and report through `msg_tx`, as sibling `exec_*` helpers do; compute directory existence off the update loop and send the result.

## Changes

| File | Change |
|------|--------|
| `src/runtime/editor.rs` | changed-fields-only updates; early return on refusal |
| `src/runtime/mod.rs`, `src/main.rs` | input loop exit; shutdown timeout |
| `src/runtime/tasks.rs` | shared table list; quick-dispatch errors; trust off the drain |
| `src/runtime/epics.rs` + module/twin | one-reducer group toggle |
| `src/tui/update/system.rs` | directory checks off the update loop |
| `docs/architecture.md` | keep the refresh claim true |

## Verification

- [ ] `./scripts/verify.sh` passes
- [ ] New tests for W3, W4, W6 fail before the fix
