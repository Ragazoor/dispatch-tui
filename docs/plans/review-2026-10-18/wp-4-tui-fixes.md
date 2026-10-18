# TUI Fixes

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix the task-detail scroll bound and a latent non-ASCII panic.

## Context

This work package addresses findings from the 2026-10-18 codebase review (`docs/plans/2026-10-18-codebase-review.md`, finding numbers match that report). Verify each finding in the code before fixing; it may have moved.

## Findings

### Medium: Scroll bound double-counts the separator (`src/tui/ui/kanban/popups/task_detail.rs:~95`)

**Issue:** `header_height` already includes the separator row. Body height is `inner.height - header_height`. The code subtracts `2 + header_height + 1`, so `max_scroll` is one too large and the last line stops one row above the bottom.

**Fix:** Drop the extra `+ 1` (fix the comment too). Add a test that the last line sits on the bottom row at `max_scroll`. Add tests for `body_height` on a short terminal and `render_scroll_indicators` in a zero-size or one-row area.

### Low: `push_hint_spans` byte-slices a label (`src/tui/ui/shared.rs:472`)

**Fix:** Use `label.strip_prefix(first_char)` or `char::len_utf8`. Add a non-ASCII test.

## Changes

| File | Change |
|------|--------|
| `src/tui/ui/kanban/popups/task_detail.rs` | Body height fix |
| `src/tui/tests/task_detail.rs`, `src/tui/ui/kanban/tests.rs` | New tests |
| `src/tui/ui/shared.rs` | Char-safe slicing |

## Verification

- [ ] Run existing tests — all pass (update snapshots only if the change is intended)
- [ ] Run the repo's verify command from get_task
