# Give Done its own completion timestamp (task #4878)

Split out of #4875. `sort_order` currently carries two unrelated meanings:
manual/feed ordering, and the completion-recency rank stamped on the way into
Done (a negated millisecond timestamp). The Done column reads only the second
and cannot tell them apart.

This replaces the rank with a real `completed_at: Timestamp?` on Task and Epic.

## Decisions (agreed with the user, 2026-09-18)

1. **Done orders on `completed_at` descending.** No negation trick, no
   sign-dependent fallback. `sort_order` goes back to meaning one thing.
2. **Manual reorder in Done swaps `completed_at`.** For task cards *and* epic
   cards. The current outright refusal of an epic-card reorder in Done goes
   away: the epic now owns a field the column reads. The cost is that a
   reorder rewrites a factual timestamp — accepted, because it is the user
   explicitly overriding the recorded order.
3. **Epic card key in Done: its own `completed_at` when set, else the freshest
   `completed_at` among the done tasks that placed it in this column.** An
   explicit statement about the card beats the derived one. A still-running
   epic has none, so it falls back to the subtask key — today's behaviour.
4. **Leaving Done keeps `completed_at`.** It means "last time this finished".
   Re-entering Done overwrites it. Status transitions stop touching
   `sort_order` altogether, in both directions.
5. **A card born in Done is stamped** — `create_task`/`create_epic` with status
   done, and feed upserts that land in done — with `completed_at = now`.
6. **Migration moves and clears.** For done rows with a *negative*
   `sort_order`: `completed_at = -sort_order` (ms), `sort_order = NULL`.
   Positive values are real manual ordering and are left alone.

## Consequences worth noting

- The leaving-done `sort_order = null` clears disappear from every rule that
  has one. A task keeps whatever manual ordering it had before Done.
- `feeds.allium`'s "re-poll `sort_order` update is skipped when the task is
  done" rule exists only because of the overload. It can go: the feed's
  `sort_order` is now safe to apply to a done task.
- `spacetime/module/src/lib.rs` mirrors the tasks/epics columns positionally
  (`src/spacetime/tests/module_schema.rs`), so both need the new column.

## Steps

Spec first, then tests, then code — per CLAUDE.md.

### 1. Spec (`allium:tend`)

- `core.allium`: add `completed_at: Timestamp?` to `Task` and `Epic`; retire
  the "shared namespace" note on `sort_order`.
- `tasks.allium`: `ConfirmDone` stamps `completed_at`; `CreateTask` stamps it
  when the task is born done; drop the leaving-done `sort_order` clears
  (MoveTaskBackward, UpdateTaskViaMcp, the editor); give `ReorderItem` its
  Done branch (swap `completed_at`).
- `epics.allium`: `MoveEpicForward`, `UpdateEpicViaMcp`, `CreateEpic`, and the
  `EpicStatusRecalculation` invariant.
- `pr-workflow.allium`: `PrMerged`, `ExitSession`'s non-pr branch.
- `mcp-task-tools.allium`: `close_session`, `exit_session`, `update_task`.
- `feeds.allium`: drop the done-task `sort_order` skip; stamp `completed_at`
  on a feed upsert landing in done.
- `board-layout.allium`, "Done Column Ordering": rewrite. The two accepted
  costs are now fixed, so they come out; `done_sort_key` becomes
  `epic.completed_at ?? freshest subtask completed_at`, descending; the
  manual-reorder paragraph gains the epic branch.

### 2. Tests (`allium:propagate`), confirmed red before any code

Expect them in `src/tui/tests/done_ordering.rs`, `src/models/` unit tests,
`src/service/tasks/tests/crud.rs`, `src/service/epics.rs`, migration tests in
`src/db/migrations.rs`, and `src/tui/tests/navigation.rs` for the reorder.

### 3. Code

- Migration v98: add `completed_at INTEGER` to `tasks` and `epics`; move and
  clear negative done `sort_order`s.
- `src/models/tasks.rs`: replace `sort_order_for_status_transition` with a
  `completed_at` equivalent; replace `fold_newest_done_rank` with a max-over-
  `completed_at` fold.
- `TaskPatch`/`EpicPatch`: new nullable field via `patch_struct!`.
- `src/tui/types.rs` (`EpicPlacement`), `src/tui/mod.rs`
  (`flattened_group_keys`), `src/tui/update/navigation.rs`
  (`handle_reorder_item`).
- `src/service/tasks/crud.rs`, `src/service/epics.rs`, `src/db/queries/epics.rs`.
- `spacetime/module/src/lib.rs` + `./scripts/check-spacetime-module.sh`.

### 4. `allium:weed` to confirm spec and code agree.
