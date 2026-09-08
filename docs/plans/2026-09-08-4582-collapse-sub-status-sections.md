# 4582 — Collapse sub-status sections

Board columns Running and Review group their cards under sub-status section
headers (`── awaiting review`, `── approved`, `── parked`, …). A section can grow
without bound — reviews waiting on somebody else, approvals already given — and
push every other section in the column off the screen. This adds a per-section
fold, so the user can hide a pile they are not working on right now.

Spec: `docs/specs/core.allium` ("Column Sections", "Column Section Layout",
"Collapsed Sections") and `docs/specs/tasks.allium`
(`ToggleSectionCollapse`). Those are the source of truth; this file is the
implementation route.

## Agreed behaviour (from elicitation)

- `z` on a card folds the section that card is in. `z`, `Space` or `Enter` on a
  folded section's header unfolds it. No other key does anything on a header.
- A folded section renders its header only, with the hidden-card count and a
  fold marker: `── awaiting review (7) ▸`. An expanded header is unchanged —
  no count, no marker.
- Sections have a stable identity and a fixed label. `stale` and `stale_shell`
  share one section labelled `stale` (today the label is whichever card sorts
  first, so it can flip between `stale` and `shell stale`).
- Folds are keyed per `(column, section)`: Running/conflict and Review/conflict
  fold independently.
- Folds persist across restarts.
- Folds apply everywhere sections render: board view, epic view, flat view.
- A live search query that admits a card inside a folded section renders that
  section expanded. The recorded fold is untouched.
- Select-all reaches only the cards a column renders, so hidden cards are not
  selected. Cards selected before the fold stay selected.
- The column's own header count (`RUNNING 3`) is unchanged by folding: it counts
  cards, hidden ones included, and never counts a section header.
- Folding puts the cursor on the folded header; unfolding puts it on that
  section's first card.

## Why this shape

Two design forks worth recording, because the cheaper answer to each is wrong.

**Sections get an identity of their own (`ColumnSection`), not a label derived
from their first card.** Today a section is a *priority slot*, and the header
label comes from whichever card sorts first inside it. `Stale` and `StaleShell`
share the slot but have different labels, so the same section can be called
`stale` or `shell stale` depending on its contents. Keying a fold on that would
lose the fold when the contents turn over. Keying it on the slot number would
work, but the slots are renumberable internals with deliberate gaps — a poor
thing to write into a settings row.

**Section headers move from the renderer into the data layer.** Today the
flattened path builds `ColumnItem::SubstatusLabel` items but the non-flattened
path injects headers inside `build_task_col_data`, and `columns.rs:114` carries
a comment warning the two must not both fire. A fold needs the header to be
countable (it takes a navigable row) and selectable (it is the only way back
into a folded section), and neither is reachable from the renderer. Unifying
the two paths is a precondition, not a bonus.

## Steps

Each step is test-first: write the failing test, watch it fail, then make it
pass. Run `cargo test` (the lib target is ~10s) between steps.

### 1. `ColumnSection` — the section identity

`src/models/columns.rs`.

Tests first, in that file's `mod tests`:
- every `(TaskStatus, SubStatus)` pair valid per `is_valid_for` maps to exactly
  one `ColumnSection`, except `SubStatus::None`, which maps to none;
- `Stale` and `StaleShell` map to the same section, and its label is `stale`;
- the derived trio still classify as before (reuse the existing
  `DerivedSection::for_task` cases: parked wins over a recorded decision, a
  detached task *with* a PR is `awaiting_review`, the `pr_review`/`dependabot`
  tags produce the `_by_me` sections);
- `ColumnSection::ALL` is ordered by ascending `column_priority()`, and the
  priority of each variant equals the number the corresponding `SubStatus` or
  `DerivedSection` returns today (a pinned-value test, so nothing reorders);
- every variant round-trips through its string form.

Then:
- `enum ColumnSection` with the 13 variants from the spec, `ALL` in render
  order, and `properties()` returning `{ priority, header_label }` — the single
  table, as `SubStatus::properties` and `DerivedSection::properties` are today.
  Move the `PRIORITY_*` consts here verbatim so no ordering changes.
- `ColumnSection::for_task(&Task) -> Option<ColumnSection>`, absorbing
  `DerivedSection::for_task`. Delete `DerivedSection` — its only remaining job
  was classification, and `ColumnSection` does it. Fix the two doc comments in
  `src/models/tasks.rs` that name it and the `pub use` in `src/models/mod.rs`.
- `SubStatus::column_section() -> Option<ColumnSection>`, and rewrite
  `SubStatus::column_priority()` / `header_label()` to delegate through it.
  `SubStatus::None` keeps today's priority via a named `PRIORITY_NO_SECTION`
  const equal to the active slot, with a comment that it never shares a column
  with a sectioned card.
- `EpicSubstatus::column_section() -> Option<ColumnSection>` in
  `src/models/epics.rs` (`Blocked → needs_input`, `Active → active`,
  `InReview → awaiting_review`, rest none), and delegate its
  `column_priority()` / `header_label()` the same way.
- `task_column_section(&Task) -> Option<ColumnSection>`; keep
  `task_column_priority` and `task_header_label` delegating to it so their
  callers do not all have to change in this step.
- `define_str_enum!(ColumnSection, "column-section" { … })` for persistence.

### 2. Fold state on `App`

Tests first, in `src/tui/tests/`:
- a fresh `App` has nothing folded;
- toggling a `(status, section)` pair folds it, toggling again unfolds it;
- Running/conflict and Review/conflict fold independently;
- the serialised form round-trips, and an unknown token in a stored string is
  skipped rather than failing the whole load.

Then:
- A new top-level `App` field `folds: SectionFoldState`, holding
  `BTreeSet<(TaskStatus, ColumnSection)>` (`BTreeSet` so the serialised order is
  stable and snapshots are deterministic). **Not** on `BoardState`: that struct
  holds ephemeral board content, and its one flag of this shape (`flattened`) is
  explicitly session-scoped. The persisted preference belongs beside `filter`
  and `search`, which is where the other persisted preference lives.
- `App::is_section_collapsed(status, section)` and
  `App::toggle_section_collapse(status, section)`.
- Serialise as comma-separated `status/section` using the existing
  `define_str_enum!` string forms — e.g. `review/approved,running/stale`. Skip
  unparseable tokens on load. Persist via the existing
  `Command::PersistStringSetting` under key `collapsed_sections`; load in a new
  `load_collapsed_sections` next to `load_repo_filter` in `src/runtime/mod.rs`.

### 3. One header path in the data layer

Tests first, in `src/tui/tests/navigation.rs` / `rendering.rs`:
- a non-flattened Running column with cards in two sections yields
  `[SubstatusLabel, Task, SubstatusLabel, Task]` from
  `column_items_for_status_with_stats` — headers now come from the data layer;
- a Backlog column yields no `SubstatusLabel` at all;
- an epic card and a task in the same state share one header;
- the flattened path is unchanged (it already emitted these).

Then:
- `ColumnItem::SubstatusLabel(&'static str)` becomes
  `ColumnItem::SubstatusLabel(SectionHeader)` where `SectionHeader` carries
  `{ status, section, collapsed: bool, hidden: usize }`. `hidden` is 0 unless
  collapsed.
- Emit headers in the non-flattened arm of
  `column_items_for_status_with_view_tasks`, grouping on
  `ColumnSection` rather than on the priority number.
- Switch the flattened arm's grouping to `ColumnSection` too, and take its
  label from the section rather than from the first card.
- Delete the header injection from `build_task_col_data` in
  `src/tui/ui/kanban/columns.rs`, along with `show_headers` and the comment at
  line 114. The renderer now only draws what it is handed.

### 4. Hide the folded cards

Tests first:
- with Review/approved folded, the column's items hold the `approved` header
  and no `approved` cards, and every other section is intact;
- the header reports the number of cards it hides, counted after the repo
  filter, the only-active filter and the search query — a hidden card the
  filters would have dropped is not counted;
- a folded section with no cards emits no header;
- in a flattened column, a folded section's `EpicHeader` and `OrphanSeparator`
  items go with its cards;
- with a search query live, a folded section that has matching cards renders
  expanded, and the recorded fold is unchanged;
- with a search query live, a folded section with no matching cards emits no
  header (it has no cards, so it never did).

Then, in both arms of `column_items_for_status_with_view_tasks`: after the
per-section group is known, if the section is folded and no search query is
active, emit the header with `collapsed: true` and the group's length as
`hidden`, and skip the group's cards and decorations.

Note the simplification the search override collapses to: a folded section with
no visible cards renders no header either way, so "expand a folded section that
holds a match" is exactly "ignore folds while a query is live".

### 5. Selectable headers, and the cursor

**The dangerous step.** Three of the sites below stay type-correct and
exhaustive after the change and so produce no compiler error — two of them
`unreachable!()` arms that a filter used to make sound and no longer does.
Do not rely on the compiler to find this step's work.

Tests first, in `src/tui/tests/navigation.rs`:
- `ColumnItem::is_selectable()` is true for a collapsed `SubstatusLabel` and
  false for an expanded one;
- `column_item_count` counts a collapsed header and none of its hidden cards;
- the column header bar renders `RUNNING 3` for three cards with one section
  folded — the count ignores folding and never counts the header;
- with a section folded in the focused column, rendering the board does not
  panic, and the select-all checkbox reflects the cards, not the header
  (regression guard for the `unreachable!()` below);
- folding the section the cursor is in leaves the cursor on that section's
  header — assert it for a card in the *middle* of a multi-card section, not
  just the first, since the first card's index coincides with the header's;
- unfolding leaves the cursor on that section's first card;
- `j`/`k` step onto a collapsed header and past an expanded one;
- the cursor stays on a collapsed header across a data refresh (the anchor
  survives);
- `selected_task()` and `selected_epic_id()` are `None` on a header;
- a folded section whose last card disappears drops its header and the cursor
  clamps inside the same column;
- `J`/`K` on a collapsed header is a no-op (existing guard; pin it).

Then:
- `is_selectable()` returns true for a collapsed `SubstatusLabel`.
- `ColumnAnchor::Section(TaskStatus, ColumnSection)`; emit it from the
  `column_anchor_cache` builder in `src/tui/mod.rs` and handle it in the
  `unreachable!` arm there.
- **`src/tui/ui/kanban/mod.rs::task_column_segment`** — no compiler error here,
  and it panics on the first fold in the focused column. Its checkbox fold
  filters to `is_selectable()` and then matches with
  `EpicHeader | SubstatusLabel | OrphanSeparator => unreachable!()`; the filter
  no longer rules that arm out. Make the fold skip a `SubstatusLabel` instead.
  Its `count` (the `RUNNING 3` figure) also filters on `is_selectable()`, which
  would start counting folded headers and stop counting hidden cards — switch
  it to counting `Task`/`Epic` items plus each collapsed header's `hidden`, so
  the number is unchanged by folding.
- **`src/tui/ui/kanban/columns.rs::build_task_col_data`** — no compiler error,
  and it silently gives the cursor to the wrong row. Its `SubstatusLabel` arm
  `continue`s *before* the `selectable_idx == selected_row` check that sets
  `is_cursor` and `list_selection_idx`, so a collapsed header never takes the
  cursor and the next real card inherits the cursor state meant for it.
  Restructure: a collapsed header participates in `selectable_idx` and can set
  `list_selection_idx`, and `render_substatus_header` gains an `is_cursor`
  argument so the folded header can draw the highlight.
- `column_item_count_with` can no longer compute analytically when the status
  has any folded section: keep the `task_count + epic_count` fast path when it
  does not, and otherwise count `is_selectable()` items from the built list.
- Add the new variant to every `ColumnItem` match that currently lists
  `SubstatusLabel` in a catch-all arm (`src/tui/update/selection.rs`,
  `src/tui/input/normal.rs`, `src/tui/input.rs`, `src/tui/mod.rs`) — these
  *are* compiler-visible once the payload type changes. A header is not a task
  or an epic, so every one of them keeps declining.
- `handle_reorder_item` in `src/tui/update/navigation.rs` needs **no change**:
  it filters to `is_selectable()` before indexing and already returns early on
  a `SubstatusLabel`. Add the test above rather than touching it.

### 6. The `z` binding

Tests first, in `src/tui/tests/navigation.rs` and `usage.rs`:
- `z` on a card in Running folds that card's section, and records the
  `toggle_section_collapse` usage event;
- `z` again on the resulting header unfolds it;
- `Space` and `Enter` on a collapsed header unfold it;
- `Space` and `Enter` on a *card* keep their existing meanings;
- `z` in Backlog, in Done, on the archive column and on the select-all cursor
  position is a silent no-op;
- `x`, `v`, `e`, `L`, `J` and `p` on a collapsed header are silent no-ops;
- `z` on a card in a section that is *recorded* folded but *rendered* open by a
  live search query records it as unfolded — it is a true toggle over the
  recorded state, not "z always folds". Clearing the query then shows the
  section open.

Then:
- `Message::ToggleSectionCollapse` in `src/tui/types.rs`, dispatched in
  `src/tui/dispatcher.rs` to a handler in `src/tui/update/`.
- `KeyCode::Char('z')` in `handle_key_board_normal`. `z` is already bound
  inside the `TaskDetail` overlay (`zoom_detail`), which
  `handle_key_normal` routes away before reaching the board arm — no conflict.
- Extend `handle_key_activate` (Space) and `handle_key_enter_normal` (Enter) to
  route to the same message when the cursor is on a collapsed header.
- The handler: resolve the section from the cursor, toggle the recorded set,
  **set the anchor explicitly** to `ColumnAnchor::Section(status, section)` when
  folding or to the section's first card when unfolding, *then*
  `sync_board_selection()`, and return the persist command.

  The explicit anchor write is load-bearing. `sync_board_selection()` searches
  the rebuilt anchor cache for the anchor it already holds — which, if left
  alone, still names the card that has just been hidden. That search fails and
  the function falls through to `clamp_selection()`, which leaves an in-bounds
  row index *numerically unchanged*. For a card anywhere but first in its
  section that index now resolves to a card in the next section, silently. This
  is the shape every handler in `src/tui/update/navigation.rs` already uses:
  decide the target, write it, then reconcile.

### 7. Render the folded header

Tests first: a snapshot test of a Running column with one section folded, and a
unit test that `render_substatus_header` puts the count and the marker on a
collapsed header and neither on an expanded one.

Then extend `render_substatus_header` in `src/tui/ui/shared.rs` to take the
`SectionHeader` and an `is_cursor` flag rather than a bare label. Review and
re-accept the snapshots the data-layer move in step 3 touched.

The existing `flat_view_*` snapshots (notably
`flat_view_substatus_indicators_above_epic_headers`) are the ones that pin
today's header rendering. Nothing about step 3 should move a pixel in them.

### 8. Docs

- `docs/reference.md`: a `z` row in the Tasks key table, and a paragraph on
  folding under it.
- `src/tui/ui/kanban/popups/help.rs`: the `z` line (mind the length note in
  that file — the `General` section must still render).
- `docs/module-map.md`: `ColumnSection` in the `src/models/columns.rs` row,
  and drop `DerivedSection`.
- `docs/conventions.md` mentions `TaskTag::is_review` having two behavioural
  readers, one of them `DerivedSection::for_task` — retarget it.
- `compute_layout_fingerprint`'s doc comment claims "two boards with the same
  fingerprint necessarily derive the same cached views". Fold state is now an
  input to `column_items_for_status_with_view_tasks` that the fingerprint does
  not see, so fold the `folds` set into it. That keeps the self-heal net over
  the new state rather than narrowing the guarantee to whatever remembers to
  invalidate.
- While in `src/tui/tests/epics.rs` for step 3: the comment on
  `flat_view_review_substatus_label_precedes_epic_header` claims
  "AwaitingReview has column_priority=5; Approved has column_priority=6", which
  is stale (`PRIORITY_APPROVED` is 45, AwaitingReview's slot is 50). The test
  asserts item shape rather than order, so it passes either way. Fix the
  comment.

### 9. Close out

- `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- `allium check docs/specs/*.allium`, then `allium:weed` over `core.allium` and
  `tasks.allium` to confirm spec and code agree.
- `git log --oneline HEAD..main` — merge and re-run if it is non-empty.

## Corrections made while implementing

Recorded because each was a wrong assumption in this plan, not just a detail:

- **The fold marker is `⋯`, not `▸`.** That triangle already means three
  unrelated things on this board — focused column, archive column, and "this
  epic has a plan" — so a fourth reading would have made all four ambiguous.
- **A folded header needs a background lift, not just the cursor white.** A
  card shows the cursor on its frame; a header has none, and a brightness step
  on one already-bold line is too small to find. It takes the cursor near-white
  plus the neutral fill the select-all checkbox uses.
- **There is no separate split-pane layout to exempt.** The plan and an early
  draft of the spec both said the eight-visual-column builder
  (`column_items_for_visual_column`) renders split view, so folding had no
  surface there. It has no non-test caller at all: split view insets the same
  four sectioned columns inside a focus border, and folding works there.
- **`renders_collapsed` needed no per-section card count.** "A folded section
  holding a match opens" and "no section draws folded while a query is live"
  are the same rule, because a section the query empties renders no header
  either way. The spec now states the second, which is what the code does.
- **`task_header_label` and `TaskStatus::has_substatus_sections` were deleted.**
  Both became test-only once `ColumnSection` owned the labels and the section
  mapping answered "does this column have sections". The claim each carried is
  now pinned by a test derived from the mapping instead.

## Risks

- **Selection plumbing is the deep part, and the compiler will not help.**
  Making a decorative item selectable touches count, clamp, anchor and every
  cursor consumer. Two `unreachable!()` arms (`task_column_segment`, the anchor
  cache builder) were sound only because a filter ruled them out, and stay
  type-correct once it doesn't; `build_task_col_data`'s `continue` silently
  misassigns the cursor. All three are named in step 5. Treat "it compiles" as
  no evidence at all for this step.
- **`column_item_count_with` loses its analytic shortcut** whenever a fold is
  active. The fallback builds the item list, which is what the shortcut exists
  to avoid, and it runs on the navigation path. Gate it on "this status has a
  fold" so an unfolded board pays nothing.
- **Snapshot churn.** Moving headers into the data layer should be
  pixel-identical, but any drift shows up as a snapshot diff. Treat a diff in
  step 3 as a bug, not as a snapshot to re-accept.
