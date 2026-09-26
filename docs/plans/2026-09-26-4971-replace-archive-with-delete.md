# Replace archive with permanent delete (task #4971)

Decided with the user in an `allium:elicit` session on 2026-09-26. Follows the
keybinding cleanup in #4969, where `x` on a Done card was the archive gesture.

## Decisions

1. **`x` on a Done task or a qualifying epic deletes it permanently**, after a
   y/n confirmation. The `archived` status, the Archive edge column (nav index
   5), its keys and its view are removed. `x` on a non-Done task still routes
   to ConfirmDone, unchanged.
2. **Retired feed items.** Deleting a feed task records
   `RetiredFeedItem(feed_epic, external_id)`, where `feed_epic` is the nearest
   epic in the task's chain (itself or an ancestor) that carries a
   `feed_command` — the epic whose cycle emits the item. Ingest never creates a
   task for a retired id.
   - Append-only epics keep the record forever.
   - Mirror (trusted, non-additive) cycles drop records whose `external_id` is
     absent from the emission: the upstream item closed, so a later reopen
     shows up again. Additive cycles (append-only or degraded) never drop.
   - Deleting the feed epic itself drops its records (FK cascade).
   - A task not under any feed epic records nothing.
   - No un-retire action.
3. **Migration of existing rows.** One migration:
   - writes a retired record for every archived task with an `external_id`
     under a feed epic;
   - turns an archived task that still holds a worktree (teardown failed) into
     `done`, so the user can delete it and retry teardown from the board;
   - deletes every other archived task;
   - deletes archived epics whose subtree holds no remaining task; any archived
     epic that still holds a task becomes `done` and is recalculated;
   - rebuilds the status CHECK constraints without `archived`.
4. **No MCP delete tool.** Permanent delete stays a human TUI action.
5. **Managed feeds.** The "archived managed epic = opted out" branch goes.
   Deleting a managed epic re-provisions it on the next ensure while its
   command is configured; clearing the command is the opt-out.
6. **Epic delete guard.** `x` on an epic deletes it only when every task in its
   whole subtree, at any depth, is `done` (an empty subtree qualifies). A batch
   skips epics that fail the guard and reports the count.

## Refinements from the adversarial review

- **Mirror drop uses the full parsed emission.** Role-routed sync filters items
  (`excluded_from_reviews`) before building its keep-set. A retired record is
  dropped only when its `external_id` is absent from the *parsed* items, before
  any exclusion filter, so an excluded-but-open PR stays retired.
- **Archived managed root epics keep their opt-out.** In the migration, an
  archived `reviews_parent` / `cve` root epic clears the matching managed-feed
  command and interval settings before it is deleted; otherwise the next
  `ensure_managed_epics` would re-provision it. An archived role sub-epic
  (`my_reviews`, `team_reviews`, `bots`) has no config equivalent: its tasks are
  retired under the root and the sub-epic is deleted, so new items in that role
  appear again.
- **Deleting a feed epic is a reset.** Its retired records go with it (FK
  cascade). For a managed epic that is re-provisioned, still-open upstream
  items the user had deleted reappear. Accepted: it is what decision 5 means.
- **Moving a feed task out of its feed detaches it**, as today (the feed would
  already insert a fresh row for that id). Deleting a detached task records
  nothing for the original feed; deleting one moved under another feed records
  under that feed. Retirement is keyed by the task's chain at delete time.

## Answers to the tend pass's open questions

- **A batch is all-or-nothing.** If any item in an `x` selection fails its
  check (a non-Done task, or an epic whose subtree is not all done), nothing is
  deleted and the status bar says why. This supersedes "skip and report" in
  decision 6.
- **Every archived managed root clears its config** in the migration, whether
  the row is deleted or kept as done.

## Consequences to carry through

- `TaskStatus::Archived` removed from the Rust enum, SQLite CHECKs, the
  SpacetimeDB module constants, MCP status enums and schema text.
- Remove: `ArchiveTask`, `BatchArchive`, `ArchiveEpic`, `ConfirmArchiveEpic`,
  `ReviveEpicChainOnUnarchive`, `ReviveParentChainOnEpicUnarchive`,
  `ArchivedEpicHoldsNoLiveWork` and the service guards that only exist for it
  (`ensure_epic_accepts_work` refusing archived targets).
- `DeleteTask` keeps its teardown gating (`WorktreeReleaseIsGated`); its
  precondition becomes `status = done` (TUI-enforced).
- Feed cycle's "archived epic runs no cycle" refusal goes; grouped/role-routed
  archived sub-epic handling goes; `delete_if_empty_repo_group` unchanged in
  shape.
- Watchers: finished means `done` only. Pre-existing gap fixed in passing:
  feed purges and epic deletes leave `task_watchers` rows dangling.
- Epic progress totals no longer count archived tasks.
- Learnings' own `archived` status is a separate concept and is untouched.
- Telemetry key names follow the gesture (`confirm_delete_*`).
- `RetiredFeedItem` is a SHARED table (feed tasks are shared rows): add it to
  `spacetime/module/`, regenerate bindings, and route writes through the
  shared writer like other shared tables.

## Order of work

Spec first (`tasks.allium`, `epics.allium`, `feeds.allium`, `core.allium`,
`board-layout.allium`, `board-visuals.allium`, `mcp-task-tools.allium`,
`task-watchers.allium`, `storage`/`sync` as needed), then failing tests, then
code, then `allium:weed`.
