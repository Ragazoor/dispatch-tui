# WP9: Splitting `App` — design

Baseline (2026-10-08, after the file splits): **353 methods in 23 `impl App` blocks.**

## The shape today

`App` already holds its state in sub-structs: `board`, `status`, `input`,
`select`, `filter`, `search`, `folds`, `epic_folds`, `layout`, `interaction`.
What is missing is the other half: almost every method sits on `App`, so each
one can reach every field, even when it only uses one.

## The rule

1. A method that reads or writes **one** sub-state moves onto that struct.
   Callers inside `crate::tui` call it there (`self.status.set(..)`,
   `self.input.insert_char(c)`). No pass-through wrapper is left on `App`.
2. A method that needs **several** sub-states stays on `App`. It is a
   handler: it orchestrates and returns `Vec<Command>`.
3. Read-only queries over several sub-states go on a borrowed view struct,
   not on `App` (see Columns).
4. `pub` getters used outside `crate::tui` (runtime, tests) stay on `App`.

No behaviour change. No snapshot change. One slice per commit, each green.

## Slices, in order

| # | Slice | Moves to | Methods (approx.) |
|---|-------|----------|-------------------|
| 1 | Status messages | `StatusState`: `set`, `set_sticky`, `clear` (74 call sites, mechanical) | 3 |
| 2 | Forms | `InputState`: the nine text-editing handlers (char, backspace, delete, caret moves) and the draft-step transitions that touch only `input` | ~12 |
| 3 | Multi-select | `SelectionState`: queries and toggles in `update/selection.rs` and `mod.rs` that touch only `select` | ~8 |
| 4 | Folds | `SectionFoldState` / `EpicFoldState`: drop the `App` wrappers that only delegate | ~5 |
| 5 | Columns | New `BoardView<'a>` borrowing `board`, `filter`, `search`, `folds`, `epic_folds`, `layout`. The `&self` column and placement computations in `columns.rs` move onto it; `app.view()` builds it. The `&mut self` cache methods (`cached_epic_stats`, `invalidate_layout_cache`) stay on `App`. | ~20 |
| 6 | Epics | Board-only queries (`find_epic`, reparent / move targets, subtree walks) → `BoardState`. Epic message handlers stay on `App` — they touch board, input and status together. | ~6 |

Expected result: roughly 353 → 300. Status goes first because it is mechanical
and the form handlers call it; forms then proves the pattern.

## What this does not do

- It does not move handlers that need status + board + input. Splitting those
  would mean passing three sub-states into each, with no gain in clarity.
- It does not change `update()`'s routing or the `Message`/`Command` types.

## Outcome

`impl App` methods: **353 → 297** (24 blocks). Slices 1–4 and 6 landed as
planned. Slice 5 is `BoardView` in `src/tui/columns.rs` (`app.view()`); its
methods take `self` by value, since the view is `Copy`. `compute_epic_stats`,
which nothing called, was deleted. The `&mut self` cache methods stay on `App`.
