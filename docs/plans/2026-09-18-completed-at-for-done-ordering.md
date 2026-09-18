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
7. **Every remaining undated done row is dated from `updated_at`** (added
   after the weed pass flagged it). Without it the residual set is exactly the
   rows this split exists to rescue — a done task whose rank a feed's positive
   `sort_order` had overwritten — and such a card would sit at the bottom of
   Done for good, permanently refused a manual reorder. A seconds-scale
   approximation is the same one v79 used, and a far better answer than that.

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

- Migration **v99** (v98 was taken by `migrate_v98_create_subscriptions`, which
  landed on `main` mid-session): add `completed_at TEXT` to `tasks` and
  `epics`; move and clear negative done `sort_order`s, then date the rest from
  `updated_at`.
- `src/models/tasks.rs`: replace `sort_order_for_status_transition` with a
  `completed_at` equivalent; replace `fold_newest_done_rank` with a max-over-
  `completed_at` fold.
- `TaskPatch`/`EpicPatch`: new nullable field via `patch_struct!`.
- `src/tui/types.rs` (`EpicPlacement`), `src/tui/mod.rs`
  (`flattened_group_keys`), `src/tui/update/navigation.rs`
  (`handle_reorder_item`).
- `src/service/tasks/crud.rs`, `src/service/epics.rs`, `src/db/queries/epics.rs`.
- `spacetime/module/src/lib.rs` + `./scripts/check-spacetime-module.sh`, and
  `./scripts/regenerate-spacetime-bindings.sh` afterwards.

### 4. `allium:weed` to confirm spec and code agree.

## What changed against the plan

- **The migration is v99, not v98.** `main` moved eight commits during the
  session and took slot 98.
- **`spacetime/module`'s `completed_at` is a `String` with a `""` sentinel, not
  an `Option`.** A commit that landed on `main` mid-session established that
  absence in the shared module is a sentinel, because SpacetimeDB SQL cannot
  filter on an optional column and a subscription is a `WHERE` clause. The
  column is registered in `SharedTable::sentinel_columns` for both tables, which
  is what drives the dump/restore conversion and the parity test.
- **It sits last in the module, after the module-only `owner`,** even though
  SQLite has it right after `host`. SpacetimeDB only ever appends. The parity
  test in `src/spacetime/tests/module_schema.rs` was rewritten to compare the
  SHARED columns in order and check module-only ones by presence — once a
  module-only column is published, the two column orders cannot stay identical,
  and the old `module_only_columns_sit_at_the_end_of_their_table` test asserted
  that they could.
- **The ordering key became a `CardOrderKey` enum** rather than a bare `i64`.
  "Descending" and "undated sorts last" are expressed once in its `Ord`, so
  nothing has to negate a stored value to get the direction it wants.
- **The layout fingerprint gained `completed_at`** on both tasks and epics. It
  is a placement-cache input now, and the fingerprint is what makes
  `cached_placements()` self-heal.
