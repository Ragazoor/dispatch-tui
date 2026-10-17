# TUI Task-Detail Scroll and Small Fixes

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** The task-detail overlay scrolls exactly to the end of the description at every size.

## Context

This work package addresses findings 32 and 61–63 of the 2026-10-17 code
review (`docs/plans/2026-10-17-codebase-review.md`). UI behaviour is an
Allium surface (`docs/specs/board-visuals.allium` / `board-layout.allium`);
spec first if behaviour changes. Snapshot workflow: `docs/testing.md`.

## Findings

### 🟡 Task-detail scroll miscounted and never clamped (`src/tui/ui/kanban/mod.rs:522`, `src/tui/ui/kanban/popups/task_detail.rs:101`)

**Issue:** `wrapped_line_count` uses `line.len().div_ceil(width)` (bytes),
while the paragraph wraps by display cells at word boundaries: non-ASCII text
over-counts (scroll into blank space), prose under-counts (last lines
unreachable). After recomputing `max_scroll`, the overlay renders with the
old `scroll`, so zoom or resize leaves it past the end. Also
`saturating_sub(...) as u16` truncates above 65535.

**Fix:** Count lines the way the paragraph wraps — ratatui's
`Paragraph::line_count(width)` (feature `unstable-rendered-line-info`) or a
`unicode-width` word-wrap simulation. Clamp `scroll` to the new max, render
with it, write it back to `ViewMode::TaskDetail`. Use
`u16::try_from(..).unwrap_or(u16::MAX)`.

### 🟢 Low findings

- `src/tui/ui/kanban/columns.rs:421` — scroll indicator indexes
  `frame.buffer_mut()[(x, y)]` directly; a zero-size area can panic. Guard
  or use `cell_mut`.
- `src/tui/input/normal.rs:354` — doc says quick dispatch errors for zero
  repos; it opens the picker. Check the spec and fix doc or code.
- `src/tui/input/normal.rs:28` — an action with no handler arm is a silent
  no-op. Add a test that every `board.normal` action id is handled (or a
  `tracing::warn!`).

## Changes

| File | Change |
|------|--------|
| `src/tui/ui/kanban/mod.rs` | Correct `wrapped_line_count` |
| `src/tui/ui/kanban/popups/task_detail.rs` | Clamp and write back scroll; safe cast |
| `src/tui/ui/kanban/columns.rs` | Bounds guard |
| `src/tui/input/normal.rs` | Doc; unhandled-action check |
| `Cargo.toml` | ratatui feature, if used |

## Verification

- [ ] Tests first: line count for a Swedish/CJK description and for word-wrapped prose matches the rendered height; zoom after scrolling to the end keeps the last line visible
- [ ] Snapshot tests pass (review any changed snapshots)
- [ ] `cargo test --no-fail-fast` and the task's verify command pass
