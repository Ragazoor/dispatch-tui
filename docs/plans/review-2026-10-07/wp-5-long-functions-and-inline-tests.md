# Long functions and inline tests

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Split the four production functions over 120 lines and move large inline test modules into sibling `tests` modules.

## Context

This work package addresses findings from the 2026-10-07 codebase review (`docs/plans/review-2026-10-07/report.md`, §3 and §4). Behaviour-preserving refactor; existing tests and snapshots are the safety net. No spec change expected. `src/runtime/mod.rs` is also touched by WP1 (`placeholder_database`); merge `main` before wrapping up.

## Findings

### 🔵 Functions over 120 lines

| Lines | Function |
|---:|---|
| 188 | `src/tui/mod.rs::column_items_for_status_with_view_tasks` (new) |
| 179 | `src/runtime/mod.rs::bootstrap_inner` (was 191 on 2026-10-05) |
| 152 | `src/tui/input/normal.rs::run_normal` (new) |
| 124 | `src/tui/ui/kanban/popups/task_detail.rs::render_task_detail_overlay` (new) |

**Issue:** Long functions with deep nesting (`tui/mod.rs` has 44 lines at 6+ indent levels).

**Fix:** Extract named helpers per section (per column kind, per action group, per overlay block, per bootstrap phase). Keep each under ~80 lines. Do not change rendering output: snapshot tests must pass unchanged.

### 🔵 Inline test modules in large production files

| File | Inline test lines (approx.) |
|---|---:|
| `src/models/tasks.rs` | ~1950 |
| `src/feed/mod.rs` | ~1860 |
| `src/runtime/editor.rs` | ~970 |
| `src/service/epics.rs` | ~870 |
| `src/tui/types.rs` | from line ~1664 |

**Issue:** Production files read as 2000+ lines because most of their length is tests.

**Fix:** Move each `#[cfg(test)] mod tests { … }` to a sibling file (`#[cfg(test)] mod tests;`), as done for `src/setup/plugins.rs` in d9bd7a35. Pure moves: no test edits beyond `use` paths. Check `docs/testing.md` for where tests belong.

## Changes

| File | Change |
|------|--------|
| `src/tui/mod.rs` | Split `column_items_for_status_with_view_tasks`. |
| `src/runtime/mod.rs` | Split `bootstrap_inner`. |
| `src/tui/input/normal.rs` | Split `run_normal`. |
| `src/tui/ui/kanban/popups/task_detail.rs` | Split `render_task_detail_overlay`. |
| `src/models/tasks.rs`, `src/feed/mod.rs`, `src/runtime/editor.rs`, `src/service/epics.rs`, `src/tui/types.rs` | Move inline tests to sibling `tests` files. |
| docs citing moved test symbols | Update `path::symbol` citations (the doc checkers will flag them). |

## Verification

- [ ] `cargo clippy --all-targets -- -D warnings` clean
- [ ] `cargo test` — all pass; test count unchanged (compare before/after)
- [ ] No snapshot changes (`cargo insta pending-snapshots` empty)
- [ ] `./scripts/check-doc-paths.sh` and `./scripts/check-doc-symbols.sh` pass
