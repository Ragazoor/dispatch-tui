# Epic cards per column (task #4784)

Spec: `docs/specs/board-layout.allium`, "Epic Card Placement" and `FlattenedView`.

## What changes

1. An epic card is drawn in every column where the epic's subtree has a visible
   task of that status, instead of once in the column matching `epic.status`.
   An epic with no visible subtree task falls back to Backlog.
2. Each copy's section and substatus label come from that column's slice of the
   subtree. The progress counts stay whole-subtree and identical across copies.
3. Flattened mode now reaches the Done column. Only Backlog stays unflattened.

## Steps

Tests first at every step (`src/tui/tests/epic_placement.rs`, new module).

1. **Spec** — done. `EpicCardPlacement` added, `unflattened_statuses = { backlog }`,
   cross-references updated in `epics.allium` and `docs/reference.md`.
2. **`TaskStatus::UNFLATTENED`** (`src/models/tasks.rs`) drops `Done`. Update its
   doc comment and the `unflattened_is_backlog_and_done` pin test. Rewrite
   `the_sectioned_and_unflattened_sets_are_answered_separately`
   (`src/models/columns.rs`) — the two sets are no longer exact complements, so
   its `assert_ne!` no longer holds and the pin must say what it now pins.
3. **`EpicPlacement` / `EpicPlacementMap`** (`src/tui/types.rs`): per epic, which
   of the four columns hold a visible subtree task, plus the blocked-running
   count that column's section needs.
4. **`App::compute_epic_placements`** (`src/tui/mod.rs`): one pass over
   `board.tasks`, admitting a task when it is non-archived and passes the repo,
   only-active and search predicates — the same three `tasks_for_current_view`
   applies. Attributes each admitted task to every ancestor epic via the
   children map.
5. **Column build** (`column_items_for_status_with_view_tasks`, hierarchical
   path): replace `epic.status == status` with the placement lookup, and route
   the section through a new per-column `epic_column_section(epic, status, …)`.
6. **`ColumnLayout::build`** takes the map from the layout cache the render
   pass has just warmed, and threads it through the column builds.
7. **Card render** (`src/tui/ui/kanban/cards.rs::render_epic_item`): label from
   the per-column substatus; counts unchanged.
8. **Docs**: `docs/reference.md` flat-view key row (done), plus any snapshot
   tests the new layout moves.

## Caching

The map joins `LayoutCache` next to `epic_stats_cache`. It depends on the repo
filter, the only-active filter and the search query as well as on the board, so
`compute_layout_fingerprint` gains all three — which also closes the same latent
hole for `epic_filter_cache`, derived through the very same filters.

`App::cached_placements()` re-checks the fingerprint on every read rather than
returning the field. Its callers hold `&self` and so cannot reach the
`cached_epic_stats()` self-heal, and a stale placement map does not misorder a
column, it puts cards in the wrong ones.
