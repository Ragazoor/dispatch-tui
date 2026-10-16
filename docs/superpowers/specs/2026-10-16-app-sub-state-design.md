# App sub-state split — design note (approved: option A)

Task #77834, work package 9 of the 2026-10-14 review. Plan:
`docs/plans/review-2026-10-14/wp-9-split-app-by-sub-state.md`.

## What the code looks like today

The plan names four new structs. Three have an equivalent already:

| Plan name | Already exists as | Fields on `App` |
|-----------|-------------------|-----------------|
| `FormState` | `InputState` (`src/tui/types/state.rs`): mode, buffer, caret, drafts, repo cursor, copy flow | `input` |
| `EpicState` | `EpicFoldState` (`src/tui/types/fold.rs`) plus `BoardState.epics` and the reparent picker in `InteractionState` | `epic_folds`, `board`, `interaction` |
| `Selection` | `SelectionState` (multi-select sets) and `BoardSelection` (cursor, per view mode) | `select`, `board.view_mode` |
| `Columns` | `LayoutCache` (`src/tui/types/layout.rs`) | `layout` |

So the field move is mostly done. What is left is the **methods**: 318 `fn`
over 25 `impl App` blocks, 63 of them in `src/tui/mod.rs`.

## Measurement: do the methods touch one field?

A scan of `self.<field>` per method in the six files the plan names:

- `update/forms.rs` (15 methods): every one touches `input`; 11 also touch
  `status`, and a few touch `board` or `interaction`. Only
  `handle_default_branch_detected` touches `input` alone.
- `update/epics.rs` (28): almost all touch `board`; most of those also call
  `invalidate_layout_cache`, `sync_board_selection` or touch `status`/`input`.
  The reparent methods touch `input` + `interaction` + `status`.
- `update/selection.rs` and `selection.rs`: cursor methods need `view()`,
  which reads `board`, `filter`, `search`, `folds`, `epic_folds` and `layout`
  together. Only the toggle handlers touch `select` alone.
- `columns.rs` (39): `view()` builds a borrow of six fields. Layout methods
  read `board` + `filter` + `search` + `folds` + `layout`.
- `input/table.rs` (22): dispatch glue. Nearly every method calls back into
  `App` handlers. Nothing here reads only form fields.

Conclusion: the `handle_*` methods are message handlers. They write the status
line, read the board and return `Command`s. They cannot move onto a sub-state
struct without that struct holding `&mut StatusState` and `&BoardState` too.
Only a minority of methods (pure state transitions) move cleanly.

## Proposal

Do not create four new structs. Move methods onto the existing ones, in this
order, only where a method reads that one struct alone:

1. `InputState`: `handle_default_branch_detected`, the draft step helpers
   (`enter_repo_path_step` minus its status write), `picker_candidates`.
2. `SelectionState`: `handle_toggle_select*`, `handle_select_all_column`
   state part.
3. `EpicFoldState` / `SectionFoldState`: the `persist_*` and fold-toggle state
   changes.
4. `LayoutCache`: `invalidate_layout_cache` and the fingerprint helpers, which
   already hold the coherence invariant.

Handlers that need several structs stay on `App`. For those, split by
borrow: a handler takes the pieces it needs as arguments (for example
`fn submit_title(input: &mut InputState, status: &mut StatusState, v) -> ...`)
and the `App` method becomes a one-line call. That lowers the `App` method
count only if the thin wrapper is dropped. A `match` arm in `update` can call
the free function directly.

Each slice: tests first (state-level tests on the struct, no `App`), then
move, then run `cargo test --no-fail-fast`, clippy and the doc checkers.

## Question for the approver

The plan's goal is a smaller `App`. The honest result of the scan is that a
large drop in method count needs the argument-passing change, not a field
move. Pick one:

- **A (recommended):** the proposal above. Move pure methods onto existing
  structs; convert multi-field handlers to functions over their fields,
  slice by slice, stopping when a slice stops paying.
- **B:** keep handlers on `App` and only move the pure-state methods. Small,
  safe, drops perhaps 20 methods.
- **C:** do the plan literally and introduce new `FormState`, `EpicState`,
  `Selection`, `Columns` wrappers around the existing structs. This adds a
  layer and moves few methods.

## Outcome of this pass

Moved, each with state-level tests written first:

- `InputState::apply_default_branch`, `InputState::enter_repo_path_step`
  (the `App` handler keeps only the status write).
- `SelectionState::toggle_all` (select-all for a column).
- `ReparentPickerState::selected_id` and `navigate`.

`LayoutCache` already owns its invalidation (`LayoutCache::invalidate`).
`App::invalidate_layout_cache` stays: about 60 callers, including tests and
the invariant in `docs/invariants.md`, name it. Renaming it is churn with no
state moved.

Counts: before, 318 `fn` in 25 `impl App` blocks; after, unchanged
(24 files with an `impl App` block by `grep -rc`). The moves pulled logic
out of handlers but kept each handler as the `App` entry point, so the method
count does not fall. A fall needs the `update` dispatch to call free
functions over fields directly, which touches every handler's call site in
`src/tui/messages/`. That is the next step if the count is to be the goal;
it is left as a follow-up.

## Outcome of the follow-up pass (#81925)

The `update` dispatch now calls free functions over fields directly; the
`App` wrappers are gone. Behaviour is unchanged.

- `src/tui/update/forms.rs`: every form step (`start_new_task`, `copy_task`,
  `cancel_input`, `submit_title`, `submit_description`, `submit_repo_path`,
  `submit_base_branch`, `submit_tag`, `arm_phoenix`, the quick-dispatch steps,
  `cancel_retry`, `default_branch_detected`) takes `&mut InputState` and
  `&mut StatusState`, plus the one extra field it reads. Two steps end in an
  `App` call that needs the whole app (`finish_task_creation`,
  `handle_quick_dispatch`); the route splits them: the free function does the
  field part (`apply_wrap_up_mode`, `take_quick_dispatch_repo`) and the route
  makes the `App` call.
- `src/tui/update/epic_input.rs` (new): epic title/description entry, the
  feed-owner take-over prompt and the reparent flow. `start_reparent` takes the
  tree items already built, because `App::view()` borrows every field.
- `src/tui/update/selection.rs`: `toggle_epic_auto_dispatch` and
  `toggle_epic_group_by_repo` take `&mut [Epic]`. The select toggles and
  `ClearSelection` are inline in their routes.
- `InputState::reopen_link_picker` replaces `App::cancel_new_package_title`.

Counts of `fn` inside `impl App` blocks (same scan as before, by
`awk` over non-test blocks): before 318, after 286 on the branch point. After merging a later `main`
(336 methods there, other work added some), the branch has 304: 32 fewer than
its base.

Not converted, and why: `columns.rs` and the cursor methods in `selection.rs`
build `view()`, a borrow of six fields, so a free function would take the same
six arguments and gain nothing. `input/table.rs` and `input/confirm.rs` are
dispatch glue that call back into `App`. `invalidate_layout_cache` stays, as
agreed above.
