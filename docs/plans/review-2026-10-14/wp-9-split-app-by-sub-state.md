# Split App by sub-state

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** `App` stops being one struct with 318 methods over 25 `impl` blocks: forms, epics, selection and columns each own a sub-state struct and the methods that touch only it.

## Context

This work package addresses findings from the 2026-10-14 code review
(`docs/plans/review-2026-10-14/report.md`, §4 "God type", §8 M10). The
2026-10-08 review's work package #40975 is marked done; `App` went from
346 methods / 23 blocks to 318 / 25. This is a larger effort. Write a short
design note first (`docs/superpowers/specs/`), get it approved, then
implement in slices that each pass the suite — the TUI tests assert on
returned `Command`s, so a slice is safe when they are green. The Allium
surfaces in `docs/specs/board-layout.allium`, `board-visuals.allium` and
`split-pane.allium` describe behaviour, not structure, and should not need
to change; if one does, change it first.

Read `docs/invariants.md` ("Layout-cache coherence", "Render dirty flag")
and `docs/architecture.md` (Message→Command) before touching `App`.

## Findings

### 💡 `App` god type (`src/tui/mod.rs` and 23 other files with `impl App`)

**Issue:** 318 non-test methods over 25 `impl App` blocks share every field.
The biggest blocks: `src/tui/mod.rs`, `src/tui/columns.rs` (two blocks),
`src/tui/update/epics.rs`, `src/tui/update/forms.rs`, `src/tui/input/table.rs`.

**Fix:** In order, one slice each:

1. `FormState` — the input-form fields and `src/tui/update/forms.rs` /
   `src/tui/input/table.rs` methods that read only them.
2. `EpicState` — epic fold/selection fields and `src/tui/update/epics.rs`.
3. `Selection` — `src/tui/selection.rs` and `src/tui/update/selection.rs`.
4. `Columns` — `src/tui/columns.rs`'s layout cache, keeping the
   layout-cache coherence invariant inside the new struct.

Each slice: add the struct, move the fields, move the methods, leave thin
`App` forwarders only where an external caller needs them, and delete the
forwarder once callers are updated.

## Changes

| File | Change |
|------|--------|
| `docs/superpowers/specs/2026-10-xx-app-sub-state-design.md` | Design note (approved before code) |
| `src/tui/types/state.rs` (or new `src/tui/state/*.rs`) | `FormState`, `EpicState`, `Selection`, `Columns` |
| `src/tui/mod.rs` | `App` holds the four sub-states; method count drops accordingly |
| `src/tui/update/forms.rs`, `epics.rs`, `selection.rs`, `src/tui/columns.rs`, `src/tui/selection.rs`, `src/tui/input/table.rs` | Methods move onto the sub-state structs |
| `src/runtime/**` | Call sites updated (130 `runtime→tui` references) |
| `docs/architecture.md`, `docs/module-map.md` | Describe the sub-states |

## Verification

- [ ] After every slice: `cargo test --no-fail-fast` green, `cargo clippy --all-targets -- -D warnings` green, insta snapshots unchanged
- [ ] `grep -rc 'impl App' src/tui | grep -v ':0'` shows fewer blocks; count `App` methods (the review's analyzer or `grep -c 'fn ' src/tui/mod.rs`) and record the before/after in the design note
- [ ] `./scripts/check-doc-paths.sh`, `./scripts/check-doc-symbols.sh` green
