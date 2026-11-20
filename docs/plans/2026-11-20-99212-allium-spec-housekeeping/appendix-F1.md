# Appendix F1 — tasks, epics

Condensed report of one read-only review agent for task #99212, at commit b03f5ba0 (main, 2026-11-18). Part of docs/plans/2026-11-20-99212-allium-spec-housekeeping.md. Line numbers drift as packages land: search for the cited symbol, not the line. Acceptance checks run from the worktree root.

# F1 — tasks.allium, epics.allium (review at b03f5ba0)

Read in full: tasks.allium 1-2657, epics.allium 1-2310 (every line). Diff 10b68974..HEAD: hunks and every removed line checked; the only dropped declaration is tasks' RespawnPhoenixSuccessorNeedsTheFlag (reducer retired in f856816f; 800-806 now say there is no standalone respawn reducer), and every claim of the old epics prose block (old 1838-1955) is restated in the new rule or its comment (tasks: section reorder, contract moved to top, module write rules before Invariants, EditTask/CopyTask guards; epics: EditEpic guards, new RecalculateEpicStatus rule replacing the prose block, delete-first teardown, MCP search filters). Whole-set `allium check` on a scratch copy: 0 diagnostics for both files (matches slices). Cited `path::symbol`s spot-checked: all exist (reset_unfit_sub_statuses, AgentTracking::clear, validate_epic_deletable, first_undone_task_in, views.rs::delete_check_for, support.rs::derive_epic_status/KNOWN_STATUSES, patch_epic_row, may_change_feed_command, shared.rs::render_top_indicators). RecalculateEpicStatus matches `spacetime/module/src/support.rs::derive_epic_status` + `tasks_epics.rs::recalculate_epic_chain`.

## Carried from Appendix F (98983)
All resolved. tasks: old 2 (1634 `subscriber`), T1 (order now Contracts, Entities, Rules, Module Write Rules, Invariants, Surfaces), T2, T3 (2443), T4 (2540-2542 one sentence; module still runs it: `spacetime/module/src/rules.rs::SUB_STATUS_FIT_RULES_VERSION` = 51), T5, T6 (2160-2222 guarded), T7 (2018-2020), T8 (604-606), T9. epics: E1 (rule RecalculateEpicStatus 1973), E2, E3, E4 (1148 `first_by_order`), E5 (559), E6 (106-139), E7 (371), E8 (2092). **No open items carried.**

## tasks.allium
| code | class | line | problem | remedy |
|---|---|---|---|---|
| F1-T-1 | mech | 586, 640, 1298, 1322 | `when: MoveTask(task, forward)` / `BatchMoveTasks(tasks, backward)`: a when-parameter binds a name, so both rules fire on every move | bind `direction`; `requires: direction = forward` (resp. backward) |
| F1-T-2 | mech | 1749 | `not exists task.worktree` (not exists on a field) and redundant with `not exists task` | delete the clause and `had_worktree`; the worktree's release is already `removed` (TaskTeardown) — say so in the comment. Language ref: `not exists` asserts an entity's removal only |
| F1-T-3 | mech | 1429 | `{task in tasks where task.refused}`: set-builder form (not in the language: `{x}` is a set literal; filtering is `coll where pred`) and undeclared field `refused` | `tasks where refused_by_store(t)`, black box named in a comment |
| F1-T-4 | mech | 1967 | `task.display_agent_name` declared nowhere | black box `display_agent_name(task)` (current agent's name, agents.allium) |
| F1-T-5 | mech | 1971 | `archived_by: caller` (elsewhere `caller()`, 1663) | `caller()` |
| F1-T-6 | mech | 1958 | `url: task.url` puts a core/TaskUrl into ArchivedTask.url: String? | `if task.url != null: task.url.url else: null` |
| F1-T-7 | mech | 1974-1980 | no-op ensures (`l.source_task = task`, `exists r`, `exists v`) — the "= unchanged" pattern | delete; keep the one-line comment above |
| F1-T-8 | mech | 2006-2023, 2568 | CopyTask ensures reads unbound names (confirmed_repo_path, viewed_epic, confirmed_tag, armed_phoenix, confirmed_base_branch, confirmed_wrap_up_mode, confirmed_model) | make them trigger params `CopyTask(source, repo_path, epic?, tag?, phoenix, base_branch, wrap_up_mode?, model?)`; update Board provides |
| F1-T-9 | mech | 929, 1402, 1511 | "EpicStatusRecalculation (invariant)" — now the rule epics.allium RecalculateEpicStatus | name the rule |
| F1-T-10 | bp | 937-938 | history: "since the respawn moved into the module" | "they are copied as stored, normalised when written" |
| F1-T-11 | bp | 1201 | "(today, Done)" dated word | "(Done)" |
| F1-T-12 | bp (N1) | 476, 502, 966, 1833/1888, 2284/2314, 2426 | events no rule listens to: ConfirmTrustRepoQuickDispatch, RepoTrustGranted, BuildOutputCleared, UserInformed, TaskRowWritten, TaskCreated; none has the N1 comment | one-line "observable output; no rule listens" on each; TaskRowWritten/TaskCreated are guard pass-throughs whose guard is already a `requires` (N1-compliant), so the comment suffices |
| F1-T-13 | bp | 269, 431, 448, 697, 1046, 1110, 1134, 1198, 1332, 1416, 1439, 1733, 1844, 2007, 2081 | 15 rules share their trigger's name (CreateTask, SetModel, SetWrapUpMode, ConfirmDone, SelectAllColumn, ClearSelection, ToggleSectionCollapse, ToggleEpicFold, ReorderItem, BatchMoveToEpic, MoveTaskToEpic, DeleteTask, BatchDelete, CopyTask, EditTask) | rename triggers (…Requested/…Pressed); update Board provides, in-file emitters (1308, 1311, 1428, 2204, 2206) and mcp-task-tools.allium 314, 320 |
| F1-T-14 | bp | 548, 559, 565-568 | implementation detail: `InputMode::QuickDispatch`, `build_quick_dispatch_prompt`, `git symbolic-ref` recipe duplicating detect_default_branch (378-385) | "the quick-dispatch picker", "the quick prompt variant", "base_branch is detect_default_branch(repo_path), as CreateTask" |
| F1-T-15 | bp | 1503-1507 | "the service also calls reroute_on_repo_change": the reroute is the module's patch_task (`spacetime/module/src/grouping.rs::reroute_on_repo_change`; epics 1550) | "the store's patch_task re-routes it" |
| F1-T-16 | mech | 1195, 1837, 1926, 2186, 2477 | over-long comment lines | rewrap |
| F1-T-17 | by-design | 1733/1826 | DeleteTask and DeleteTaskRefused share a trigger, split by `requires` on status | leave |

## epics.allium
| code | class | line | problem | remedy |
|---|---|---|---|---|
| F1-E-1 | mech | 265, 303 | `when: MoveEpicStatus(epic, forward)` literal binds a name; both rules fire | bind `direction`; `requires: direction = forward` |
| F1-E-2 | mech | 377 | `not exists task.worktree` (field) — rows go at 395, teardown is TaskTeardown's | replace 374-377 with a comment that each subtree task's worktree is released by TaskTeardown after the delete (the clause is invalid, so nothing formal is lost) |
| F1-E-3 | drift | 462-470 | "cleaned up best-effort … before the DB call" contradicts 440-453 "DELETE FIRST, TEAR DOWN AFTER" and `src/runtime/epics.rs::exec_delete_epic` (delete first since e0e4b62a) | "after the delete"; keep the same-subtree argument |
| F1-E-4 | drift | 1512-1513, 1584 | "ingest paths and the MCP surface call them directly" vs "no second caller today": no MCP handler or runtime path calls them (`src/service/epics.rs::regroup_epic`/`flatten_epic` have no production caller); `flatten_epic` is also called by `spacetime/module/src/feed_ingest.rs::ingest_flat`; `regroup_epic_rows` only by set_epic_group_by_repo (`spacetime/module/src/tasks_epics.rs`, via `grouping.rs::regroup_epic_rows`) | state exactly: FlattenEpic also by flat feed ingest (feeds.allium FlatFeedReconcile); RegroupEpic only by the toggle; drop "today" |
| F1-E-5 | drift | 1823-1828 | "before the index was fixed to be NULL-safe — that index was never built" is SQLite-era; the module holds uniqueness by reducer check (`spacetime/module/src/rules.rs::check_epic_unique_among`) | replace with core's wording: a row already holding a duplicate stays writable for its other fields |
| F1-E-6 | mech | 1218, 1251-1253, 1288-1289 | stop count inconsistent: "Five ways", "fourth … alongside the three below", "stopping condition three" | number once (five), refer to stops by name |
| F1-E-7 | mech | 1705-1706 | garbled "AND that parent, in / or null" | "…AND that parent, or null when none exists" |
| F1-E-8 | bp | 1852-1854 | "Epic Navigation" uses a top-level section banner inside Rules (non-canonical section) | `-- == Epic Navigation ==` |
| F1-E-9 | bp | 2061-2074 | epic-card display prose sits under Invariants with no invariant (rendering; not in board-visuals) | move to EpicBoard @guidance or board-visuals.allium |
| F1-E-10 | bp | 591-597, 717-718, 869, 965-966 | stale history/dated data: trajectory-log stats, "a board that predates the check", "the old managed_feed_config, split out" | present-tense rule only; drop stats and narrative |
| F1-E-11 | bp | 1781 | "the B3 concurrency guard": id defined nowhere | drop the id |
| F1-E-12 | bp | 418, 558, 636, 644, 1117, 1441, 2280, 2283 | implementation names (`EpicService::…`, `ServiceError::NotFound`, `ViewMode::Epic/Board`); 418 cites a path without a symbol | "epic view"/"board view"/"not found"; cite `src/service/epics.rs::ensure_deletable` where a citation is needed |
| F1-E-13 | bp (N1) | 155, 336, 520, 628, 691, 962/1014, 1385, 1388, 1883+, 1500/1526 | events no rule listens to: FeedOwnerTakeoverOffered, ConfirmationShown, UserInformed, EpicRetrieved, EpicsListed, ManagedFeedConfigRetrieved, ChainStallMarked, DesktopNotification, ViewShowing, EpicPatchAccepted | "observable output; no rule listens" each (EpicPatchAccepted as F1-T-12) |
| F1-E-14 | bp | 45, 328, 354, 1561, 1617, 1663, 1974 | 7 rules share their trigger's name (EditEpic, ConfirmDeleteEpic, DeleteEpic, RegroupEpic, FlattenEpic, ProvisionManagedEpics, RecalculateEpicStatus) | rename triggers; emitters: tasks 1872, feeds 2427/2730-2733, pr-workflow 935, store-split 831/943/1254/2005, workspaces 961, learnings 441 (consumer `epics/DeleteEpic`) |
| F1-E-15 | bp | 1426-1438 vs 1490-1506 | ToggleGroupByRepo restates the migration SetEpicGroupByRepoIsOneTransaction owns (duplicated derivation) | toggle ensures `EpicGroupByRepoSetRequested(epic, not epic.group_by_repo)` only |
| F1-E-16 | mech | 410, 427, 500, 1103, 1278, 1284, 1315 | over-long comment lines | rewrap |
| F1-E-17 | QUESTION | 2308 | open question: [R] offered on an append-only epic, refused then restored (text also says "service rule", now the store's) | (a) keep refuse-then-restore, delete the OQ; (b) guard the key in the TUI; (c) render [R] differently. **Rec (a)**: one rule source (store), restore already specified (ToggleGroupByRepo) |
| F1-E-18 | QUESTION | 1830-1843, 2310 | open question: reparenting a managed epic makes the next ProvisionManagedEpics create a duplicate | (a) look up by feed_role alone; (b) store refuses reparenting a feed_role != none epic, like repo-group; (c) accept. **Rec (b)**: same pattern as the repo-group guard, removes the hazard at source (code change in patch_epic) |
| F1-E-20 | bp | 692, 435 | literals beside config: `limit ?? 50` (task_search uses the same 50) and "64 levels" in the depth-guard message next to `core/config.max_epic_depth` | a shared config value for the search default limit; word the message as "more than max_epic_depth levels" |
| F1-E-19 | by-design | 491/354, 610/661 | DeleteEpic/DeleteEpicRefused and Get/ListEpicsViaMcp share triggers, split by `requires` | leave |

Counts: mech 15 (T 10, E 5), drift 3 (E), bp 15 (T 6, E 9), QUESTION 2 (E), by-design 2, entry 0.

## Work packages
Run each check from the package's worktree root; each was confirmed to fail at b03f5ba0.

**WP-F1a — tasks.allium mechanical and best practice.** Files: docs/specs/tasks.allium. Findings F1-T-1..12, 14..16. No question. Independent.
Acceptance (fails now: exit 1, 13 hits):
```
! grep -nE 'when: (MoveTask|BatchMoveTasks)\(tasks?, (forward|backward)\)|not exists task\.worktree|task\.display_agent_name|task\.refused|archived_by: +caller$|EpicStatusRecalculation invariant|respawn moved into|^ +l\.source_task = task$|^ +exists [rv]$' docs/specs/tasks.allium && test "$(grep -c 'observable output; no rule listens' docs/specs/tasks.allium)" -ge 5
```
**WP-F1b — epics.allium drift, mechanical, best practice, and its two open questions.** Files: docs/specs/epics.allium (+ board-visuals.allium if F1-E-9 moves there; patch_epic code if F1-E-18 takes option b). Findings F1-E-1..13, 15..18, 20. QUESTIONS F1-E-17, F1-E-18 (ask first). Independent of WP-F1a.
Acceptance (fails now: exit 1, 19 hits):
```
! grep -nE 'when: MoveEpicStatus\(epic, (forward|backward)\)|not exists task\.worktree|directly-owned ones — before the DB|NULL-safe|B3 concurrency|no second caller today|trajectory logs|old managed_feed_config|fourth normal stopping|condition three \(no backlog|MCP surface call them directly|AND that parent, in$|^-- Epic Navigation$|ViewMode::' docs/specs/epics.allium && test "$(grep -c 'observable output; no rule listens' docs/specs/epics.allium)" -ge 8
```
**WP-F1c — rename triggers that share a rule's name.** Files: tasks.allium, epics.allium, and the out-of-cluster sites (complete list): mcp-task-tools 314, 320; feeds 2427, 2730, 2732, 2733; pr-workflow 935; store-split 831, 943, 1254, 2005; workspaces 961; learnings 441 (consumer `epics/DeleteEpic`). Findings F1-T-13, F1-E-14. Order: after WP-F1a and WP-F1b (same lines); coordinate with the clusters owning those specs. Grep src/ for comments citing a renamed trigger.
Acceptance (fails now: exit 1, 22 names):
```
test -z "$(awk '/^rule /{r=$2} /^ *when: /{t=$2; sub(/\(.*/,"",t); if(t==r) print FILENAME": "r}' docs/specs/tasks.allium docs/specs/epics.allium)"
```
Footer for every package: whole-set `./scripts/check-allium.sh`, no new warning or field.unused; allium:weed on the package's specs.
