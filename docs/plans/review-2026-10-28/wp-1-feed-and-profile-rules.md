# Feed and Profile Rules

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make a feed move end a task's unseen marks, and make the display-name check refuse every format character the spec names.

## Context

This work package addresses findings N1 and N2 from `docs/plans/2026-10-28-codebase-review.md`. Both are module rules: change `spacetime/module/src/`, mirror in `src/sync/memory_caller/`, cover the pair in `tests/memory_caller_conformance/`. Write each failing conformance case first.

## Findings

### 🟡 Medium — N1: a feed move between review buckets keeps the unseen mark (`spacetime/module/src/feed_ingest.rs:445-452`)

**Issue:** `route_and_move` moves an existing feed task with `IngestStore::update_task` (`ReducerStore::update_task`, `feed_ingest.rs:111-113`; twin `src/sync/memory_caller/feed.rs:440-443`), which only writes the row. `UnseenMarkEndsWithTheTaskLeavingItsEpicOrFinishing` (`docs/specs/tasks.allium`) says any epic change ends every mark; `set_task_epic` honours it via `clear_unseen_for_task`. A PR marked in My Reviews that moves to Team Reviews stays marked. The repo-group moves in `ingest_grouped` have the same gap.

**Fix:** When `update_task` changes `epic_id`, clear the task's unseen marks (module and twin). Add a conformance step "a move out of My Reviews clears the mark" to `tests/memory_caller_conformance/feeds.rs`, plus one through a repo group.

### 🟡 Medium — N2: the display-name check misses most format characters (`spacetime/module/src/user_profile.rs:25-37`)

**Issue:** `is_format_char` hand-lists part of Unicode's Cf category. Tag characters U+E0001 and U+E0020–E007F, U+FFF9–FFFB, U+0600–0605, U+06DD, U+070F, U+08E2 and U+1D173–1D17A pass, as do U+2028/U+2029 (line and paragraph separators). `SetDisplayName` in `docs/specs/workspaces.allium` refuses all control and format characters. The twin shares the function, so conformance cannot catch it.

**Fix:** Refuse the complete Cf set (a full range table, or `unicode-general-category` if it builds for `wasm32-unknown-unknown`) plus U+2028/U+2029. If the spec does not name the separators, add them via `allium:tend` first. Add unit tests per range and a tag-character conformance case in `tests/memory_caller_conformance/scenario.rs`.

## Changes

| File | Change |
|------|--------|
| `spacetime/module/src/feed_ingest.rs` | `ReducerStore::update_task` clears unseen marks on an epic change |
| `src/sync/memory_caller/feed.rs` | Same in the twin |
| `spacetime/module/src/user_profile.rs` | Full Cf set plus U+2028/U+2029 |
| `tests/memory_caller_conformance/feeds.rs` | Move-out-of-My-Reviews case |
| `tests/memory_caller_conformance/scenario.rs` | Tag-character display-name case |
| `spacetime/module/module.wasm`, bindings | Rebuild with `./scripts/build-managed-module.sh`; regenerate bindings if the schema changed |
| `spacetime/module/src/rules.rs`, `src/sync/rules_gate.rs` | Bump `RULES_VERSION`/`BOARD_RULES_VERSION` together if the module's constant doc says this warrants one |

## Verification

- [ ] New conformance cases fail before the fix and pass after
- [ ] `cargo test --no-fail-fast` with `tmux` and `spacetime` on `PATH`; no `skipping:` lines
- [ ] Module checks from `docs/testing.md` ("SpacetimeDB module and CI details"), including `spacetime/module-tests/`
- [ ] `cargo fmt --check && ./scripts/check-doc-paths.sh`
