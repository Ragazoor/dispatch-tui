# Split App and Large Files

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Break up `App` by sub-state, and split the largest mixed-concern production files along clear seams.

## Context

This work package addresses the "God type" and "Largest production files" findings in `docs/plans/review-2026-10-08-followup/report.md`. Large and mechanical; do it in several commits, one seam per commit, each green. Start after WP1 and WP5 land (they also touch `runtime/mod.rs`). The `App` split is the bigger piece — write a short design (which sub-state owns which fields and methods) and get the user's agreement before moving code.

## Findings

### 🔵 `App` god type (`src/tui/mod.rs::App`)

**Issue:** 346 non-test methods across 23 `impl App` blocks (was 354 / 22 on 2026-08-28); 26 fields. Largest blocks: `src/tui/mod.rs` 77, `src/tui/columns.rs` 26, `src/tui/update/epics.rs` 26, `src/tui/update/forms.rs` 24. Every method can reach every field.

**Fix:** Give forms, epics, selection and columns each a struct owning its fields, with methods on that struct; `App` delegates. Start with the most self-contained slice (likely forms) to prove the pattern.

### 🔵 `src/runtime/mod.rs` (1,113 code lines)

**Fix:** `LoopEvent`, `apply_loop_event`, `run_loop` (L1495+) → `runtime/event_loop.rs`. (Store bootstrap moves in WP5.)

### 🔵 `src/tui/types.rs` (843 code lines, ~50 types)

**Fix:** `types/state.rs` (`*State` structs), `types/fold.rs` (section and epic fold), `types/layout.rs` (`ColumnItem`, `ColumnLayout`, `EpicPlacement`, `SubtaskStats`). `Message`, `Command`, `InputMode` stay.

### 🔵 `src/keybindings.rs` (1,633 code lines)

**Fix:** ~1,280 lines are the `KEY_BINDINGS` table (L487–1769) → `keybindings/table.rs`.

### 🔵 `src/models/tasks.rs` (737 code lines + ~211 inline test lines)

**Fix:** Agent-event types (`HookEventKind` through `AgentActivity`, L1146–1470) → `models/agent_events.rs`; inline test modules (L748, 792, 1477, 1514) → `models/tasks/tests.rs`.

## Changes

| File | Change |
|------|--------|
| `src/tui/mod.rs`, `src/tui/update/*`, `src/tui/columns.rs`, `src/tui/input/*` | Sub-state structs |
| `src/runtime/mod.rs` | Extract event loop |
| `src/tui/types.rs` | Split into `types/` |
| `src/keybindings.rs` | Extract table |
| `src/models/tasks.rs` | Extract agent events and tests |
| `docs/module-map.md`, `docs/architecture.md` | Update |

## Verification

- [ ] `cargo test --no-fail-fast` after each commit — all pass, no snapshot changes
- [ ] `cargo clippy --all-targets -- -D warnings`; doc checkers pass
- [ ] `impl App` method count drops materially (report the before/after)
