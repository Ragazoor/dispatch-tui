# Board lens and board-filter tabs (task #77840)

Spec: `docs/specs/board-layout.allium` ("Board Lens", "Hidden Epics and Work
Packages", "Board Filter Overlay Layout"), `work-packages.allium`
(PackageCardPlacement), `keybindings.allium` (namespace rename).

Each step is test-first: write the failing test, run it red, implement, run green.

## 1. Lens replaces `A` and `F`
- Test: Tab steps everything → active → flat → flat_active → everything;
  BackTab steps back; `A` and `F` do nothing on the board; `[active]`/`[flat]`
  badges follow the lens.
- Code: `Lens` enum on `FilterState` (`only_active()`/`flattened()` derived);
  drop `FilterState.only_active` and `BoardState.flattened`; rows
  `cycle_lens_forward` (Tab) and `cycle_lens_backward` (BackTab) in
  board.normal; remove `filter_active` and `toggle_flattened` rows.

## 2. Hidden epics and packages hide cards
- Test: hiding an epic hides its card, sub-epic cards and every subtree task,
  in hierarchical and flat lenses and inside a parent's epic view; a linked
  task under a hidden epic stays hidden; a package whose only link is a hidden
  epic sits in Backlog; a hidden package draws no card while its linked work
  stays; search cannot revive a hidden epic; the layout fingerprint changes on
  a hide.
- Code: `hidden_epics`/`hidden_packages` on `FilterState`; `BoardFilters` carries
  the hidden-epic closure (hidden ids + descendants) and `admits` checks it;
  `visible_epics_for_effective_view` and package placement skip hidden ones;
  fingerprint folds both sets (XOR, order-free).

## 3. Persistence
- Test: closing the overlay persists `hidden_epics` and `hidden_packages`
  settings; startup reads them back; unknown ids are inert.

## 4. Overlay tabs
- Test: Tab/BackTab switch Repos/Epics/Work packages; per-tab cursor;
  Space toggles the row on each tab; `i` flips include/exclude on Repos only;
  `1`-`9` and Backspace act on Repos only; Epics tab lists not-done + hidden
  as an indented tree, `d` shows done ones; `/` filters epics by title keeping
  ancestors; Esc clears the query first, then closes; footer never clipped.
- Code: namespace `board.filter` (+ `board.filter_search` for typing);
  overlay state struct; renderer split per tab.

## 5. Status bar badge and docs
- `[hidden: N epics, M packages]` badge; help overlay and `docs/reference.md`
  key tables; snapshot updates.
