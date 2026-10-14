# Move inline tests out and split oversized test files

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** No production file carries an inline test module over 200 lines, and no test file is over 2,000 lines. Pure moves; no test changes.

## Context

This work package addresses findings from the 2026-10-14 code review
(`docs/plans/review-2026-10-14/report.md`, §3 "Placement", §8 H6, M7).
`docs/testing.md:117-129` says which tests belong inline; these have outgrown
it. The 2026-10-08 work package "move inline tests" did not reach them.
Moving a module to a `tests/` submodule also makes `check-no-test-sleep.sh`
see it by the file rule rather than the column-0 region heuristic.

Keep each test's name and body unchanged so `git log --follow` and the insta
snapshots keep working. If a test reaches a private item, widen it to
`pub(super)`/`pub(crate)` rather than rewriting the test.

## Findings

### ⚠️ Inline test modules over 500 lines in production files

| File | Inline lines |
|---|---:|
| `src/store/mod.rs` | 1,472 |
| `src/dispatch/worktree.rs` | 1,305 |
| `src/dispatch/bump.rs` | 1,157 |
| `src/setup/hooks.rs` | 751 |
| `src/sync/memory_caller/mod.rs` | 733 |
| `src/runtime/mod.rs` | 720 |
| `src/tui/ui/kanban/mod.rs` | 715 |
| `src/models/epics.rs` | 551 |
| `src/service/learnings.rs` | 521 |
| `src/runtime/editor.rs` | 503 |

**Fix:** Move each to `<module>/tests/<topic>.rs` (`src/store/tests/` and
`src/runtime/tests/` already exist — add files there; create
`src/dispatch/tests/worktree.rs`, `src/dispatch/tests/bump.rs`,
`src/setup/tests/hooks.rs`, `src/sync/memory_caller/tests/store.rs`,
`src/tui/tests/rendering/kanban.rs`, `src/models/tests/epics.rs`,
`src/service/tests/learnings.rs`). Keep the
`#[allow(clippy::unwrap_used, clippy::expect_used)]` header
(`docs/testing.md:157`).

### 💡 Oversized test files

| File | Lines | Split by |
|---|---:|---|
| `tests/spacetime_module.rs` | 6,273 (145 tests, 58 s) | table/reducer family: `tasks`, `epics`, `feeds`, `requests`, `learnings`, `settings` under `tests/spacetime_module/` with a shared `common` |
| `tests/memory_caller_conformance.rs` | 3,723 | same families |
| `src/tui/tests/navigation.rs` | 2,634 | per view |
| `src/dispatch/prompts/tests.rs` | 2,624 | per prompt kind |
| `src/mcp/handlers/tests/tasks/wrap_up.rs` | 2,616 | ready-checks / request flow / verify |
| `spacetime/module/src/tests.rs` | 2,514 | per source file it tests |

**Fix:** Split along the named seams. For the two integration files, keep
one `tests/<name>.rs` entry that declares the submodules so the target names
in `docs/testing.md` and CI stay valid.

## Changes

| File | Change |
|------|--------|
| The ten production files above | Remove the inline module; add `#[cfg(test)] mod tests;` or extend the existing `tests/mod.rs` |
| New `tests/<topic>.rs` files per module | Receive the moved tests verbatim |
| `tests/spacetime_module.rs`, `tests/memory_caller_conformance.rs` | Become thin entry files over `tests/<name>/*.rs` |
| `src/tui/tests/navigation.rs`, `src/dispatch/prompts/tests.rs`, `src/mcp/handlers/tests/tasks/wrap_up.rs`, `spacetime/module/src/tests.rs` | Split along the seams above |
| `docs/testing.md` | Add a line: an inline module over ~200 lines moves to `tests/`; update any path it cites |

## Verification

- [ ] Test count unchanged: `cargo test 2>&1 | grep 'test result'` sums to the same totals before and after (5,917 lib / 372 integration); `cd spacetime/module && cargo test` still 172
- [ ] `cargo test --no-fail-fast` green; `cargo clippy --all-targets -- -D warnings` green
- [ ] `./scripts/check-doc-paths.sh` and `./scripts/check-doc-symbols.sh` green (moved paths cited in docs updated)
- [ ] No production file has a `#[cfg(test)] mod … {` block over 200 lines
