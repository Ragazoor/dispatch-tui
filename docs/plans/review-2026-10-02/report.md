# Codebase review — 2026-10-02 (task #4475)

Scope: whole repo at `main` 72c25734. Static review (no coverage run). Counts come from grep/awk and are approximate. Findings from the 2026-08-16 and 2026-08-28 reviews are not repeated.

## Executive summary

- The layering holds (`models` leaf, `tui`/`mcp`/`service` seams, `TaskServiceApi` mutation boundary). The new weight is the SpacetimeDB layer: `src/sync`, `src/spacetime`, `spacetime/module`.
- Two files are god files: `spacetime/module/src/lib.rs` (4463 lines) and its in-memory twin `src/sync/memory_caller.rs` (4598). The 40-field task row literal is restated in 4+ places.
- Test volume is large (~5,400 `#[test]` in `src`, ~250 in `tests/`) but tests live in a few 4–5k-line files, and `make_task`-style fixtures are defined in 15 files.
- Specs lag the code: no `cli.allium`; nothing covers `sdk_connector`, `writes`, `rows`, `snapshot`, `managed_store`, `restore`, `dump`.
- `CLAUDE.md` is 32 KB of dense paragraphs with task-history prose, and its "Store seam" paragraph is stale: `src/db/mod.rs:1004` says there is no shared/local seam any more (task #4916).

## 1. Architecture and patterns

Layered, trait-seamed architecture: `models` → `db` (family of `*Store` traits, one writer + read pool) → `service` (`TaskServiceApi` etc.) → `mcp`/`tui`/`runtime`. Consistently applied; boundaries are documented and mostly compiler-enforced.

- **Stale seam in docs (Medium).** `CLAUDE.md` "Store seam" paragraph describes `SharedDomainStore`/`LocalStore`; neither trait exists (`grep "trait SharedDomainStore"` is empty). `src/db/mod.rs:1004` says so. Fix `CLAUDE.md`, `docs/conventions.md` ("The store seam") and any spec still implying two backends.
- **`LocalBoardReads` is a test-only stand-in (Medium).** `src/sync/board_reads.rs` — `poll_owner` always answers "unclaimed", so `src/runtime/editor.rs` tests (:1233, 1351, 1391, 1428, 1466) need ad-hoc wrappers. Replace with an in-memory `SharedRows`-backed `BoardReads`.
- **Stringly-typed poll scope (Low).** `poll_owner(scope: &str, ..)` at `board_reads.rs:52`, `rows.rs:587`, `memory_caller.rs:1270`, validated at runtime (`require_poll_scope`, :1306). Use `enum PollScope`.
- **`startup.rs` mixes concerns (Medium).** ~1000 production lines: launch planning, board-window retirement, config drift, host label, abort messages. Split into `startup/{launch,retire,config,host}.rs`.

## 2. Test coverage

Not measured (no tarpaulin run; it also fails locally when `spacetime` is on `PATH`, see CLAUDE.md).

- **Unit-to-integration ratio** ≈ 21:1 (5,413 vs 252 attributes); most "integration" is in-process (`src/tui/tests`, `src/db/tests`, `src/mcp/handlers/tests`).
- **Oversized test files (Medium):** `src/db/tests/migrations.rs` 5665, `db/tests/tasks.rs` 5173, `dispatch/tests.rs` 4874, `service/tasks/tests/crud.rs` 4811, `mcp/handlers/tests/tasks/crud.rs` 4283, `tui/tests/epics.rs` 3662, `rendering.rs` 3549, `input_handlers.rs` 3468. Split by behaviour; make migration tests table-driven per version.
- **Inline test modules in huge source files (Medium):** `src/dispatch/prompts.rs` (3623), `src/sync/memory_caller.rs` (≈2000 test lines from :2603), `src/tmux.rs` (tests from ~:2249), `src/cli/agent_tree.rs` (:1482), `src/startup.rs` (:1016). Move to sibling `tests/` modules like the rest of the repo.
- **Duplicated task fixtures (Medium):** `make_task`/`test_task` variants in `src/db/tests/mod.rs:73`, `dispatch/tests.rs:67`, `dispatch/prompts.rs:3282`, `feed/ingest/routing.rs:150`, `mcp/handlers/tests/poll_ownership.rs:4`, `tasks/watch.rs:4`, `mcp/handlers/tasks/mod.rs:450`, `models/epics.rs:382,653`, `service/tasks/tests/mod.rs:88`, `tui/tests/helpers.rs:104`, `editor.rs:598`. Add one shared `TaskBuilder` under `cfg(any(test, feature = "test-support"))`.
- **Brittle assertions (Low):** `src/tui/tests` has ~287 `contains(`/`len()` asserts against 58 `insta` snapshots. Convert rendering `contains` checks to snapshots.
- **Likely untested files (Low–Medium):** `src/sync/writes.rs` (1439), `src/spacetime/snapshot.rs` (793), `managed_store.rs` (857), `cli_store.rs` (515). Not verified against coverage output; run a per-file coverage report.
- **Coverage depends on skippable tests (Low):** sync/spacetime behaviour is exercised mainly by `tests/spacetime_module.rs` and `tests/memory_caller_conformance.rs`, which skip in the Coverage job. Add an in-process fake-connector conformance test so the floor reflects sync code.
- **`#[allow]` repetition (Low):** ~106 per-module `allow(clippy::unwrap_used/expect_used)` in test code. Replace with `clippy.toml` `allow-unwrap-in-tests = true`. Review the 3 `dead_code` allows.

## 3. Complexity hotspots

Largest non-test files: `sync/memory_caller.rs` 4598, `cli/agent_tree.rs` 3772, `tmux.rs` 3745, `dispatch/prompts.rs` 3623, `db/migrations.rs` 3311, `setup/plugins.rs` 2975, `models/tasks.rs` 2708, `feed/mod.rs` 2353, `db/mod.rs` 2286, `tui/mod.rs` 2278. `spacetime/module/src/lib.rs` is 4463.

Longest functions:

| Function | Lines |
|---|---|
| `db/migrations.rs:2917 migrate_v106_archived_status_migration` | 322 |
| `runtime/mod.rs:865 bootstrap_inner` | 303 |
| `tui/input/normal.rs:32 handle_key_board_normal` | 277 |
| `runtime/commands.rs:134 dispatch_task` | 208 |
| `db/queries/tasks.rs:1436 upsert_feed_tasks_inner` | 189 |
| `service/epics.rs:352 update_epic` | 175 |
| `runtime/mod.rs:486 run_tui` | 168 |
| `sdk_connector.rs abandon` | ~151 |
| `db/queries/tasks.rs:581 patch_task` | 147 |
| `mcp/handlers/tasks/wrap_up.rs:283 handle_exit_session` | 140 |

Nesting: `tui/input.rs:271` in `handle_key_activate` has 21 lines nested 8+ deep. Raw SQL with `{locally_owned}` interpolation and positional `?1..?5` params at `db/queries/tasks.rs:1081-1135`.

Other 100–125-line functions: `dispatch/worktree.rs:604 provision_worktree`, `repo_sync.rs:206 sync_repo`, `cli/agent_tree.rs:1067 dispatch_key`, `setup/mod.rs:726 run_uninstall_in`, `dispatch/finish.rs:83 finish_task`, `feed/cycle.rs:87 run`, `feed/mod.rs:339 tick`, `dispatch/agents.rs:374 dispatch_with_prompt`, `mcp/handlers/learnings.rs:74 handle_record_learning`.

## 4. Code smells

- **God files (High):** `spacetime/module/src/lib.rs` (≈4000 production lines, 130 fns, six `// ----` banners at :147, 670, 776, 808, 1099, 1425) and `src/sync/memory_caller.rs` (60-method `ReducerCaller` impl at :1318-2600). Split both along the same domain lines: tasks/epics, learnings, feed, agent state, config.
- **Duplicated schema (Medium):** the ~40-field task row literal in `memory_caller.rs:988,1033` (`blank_module_task/epic`), `spacetime/module/src/lib.rs:926,976` (`blank_task/epic`), `src/sync/tests/decode.rs:490`, `memory_caller.rs:2647`. A new column needs 4+ edits. Make the module's `blank_*` public, or give the row types `Default`.
- **Boilerplate (Medium):** `src/sync/sdk_connector.rs` (1781 lines) has ~40 near-identical `awaiting_answer(.., move |tx| connection.reducers.X_then(..))` blocks (:856, 888, 944, 1077, 1192, 1222, 1239, 1274, 1415, 1457, 1578). Only `answered_call!` (:847) is macro-ised. Add sibling macros; move the reducer impl into its own file. `wire_rows` (~126 lines) and `connect` (~84) should split too.
- **Primitive obsession on `ReducerCaller` (Medium, `src/sync/writes.rs:108-370`):** `TaskId` for some methods but raw `i64` for others (`create_epic`, `delete_epic`, `batch_delete`, `subagent_clear`, `try_record_stop`, `delete_learning`); timestamps as `String` (`last_used`, `cutoff`, `stop_pending_at`); `sub_status: String`. Use `EpicId`/`TaskId`, a timestamp type and the status enums; encode only in `encode.rs`.
- **Stringly-typed enums (Medium):** `memory_caller.rs:1947,2166` match `"helped"/"wrong"` and `"clear"/"raise"`; `dispatch/mod.rs:199` matches `"OPEN"/"MERGED"/"CLOSED"` by hand (`PrState: FromStr`); `mcp/handlers/dispatch.rs:736` matches `req.method.as_str()`.
- **Path/pane primitives (Low):** `repo_path: String` and `tmux_window: Option<String>` in ~55 places; pane IDs as bare `&str` (`tmux.rs:1241`); `expand_tilde` is called at each use site (`dispatch/agents.rs:385`, `repo_sync.rs:211`). Add `PaneId`/`RepoPath` newtypes; add `resolve_repo(task)` to share the empty-check + expand (`dispatch/agents.rs:374`, `dispatch/mod.rs:199`; check other call sites first).
- **Dead scaffolding (Low):** `memory_caller.rs` `is_complete()`, `covered_domain_count()`, `COVERED_DOMAINS` are referenced only in-file, and the `is_complete` doc at :139 ("Always false today") contradicts the module header.
- **Duplicated parsing (Low):** server-name trim/blank-as-none in `spacetime/managed_store.rs:117 select_store` and `startup.rs:208 store_server_or_managed`. One `normalize_server` helper.
- **Silent error swallowing (Low):** ~15 `let _ = tx.send(..)` and `let _ = connection.disconnect()/unsubscribe()` in `sdk_connector.rs` (:266, 291, 311, 433, 617). Wrap in a `fire(tx, v)` helper that logs at `debug`.
- **Wide parameter lists (Low):** 6–7 parameter functions around `runtime/mod.rs:829-871` (`bootstrap_*`, with an `fn`-pointer `build_store`), `db/mod.rs:~564`, `feed/ingest/grouped.rs:~1014`, `tui/ui/kanban/cards.rs:~1057`. Verify by name; bundle into param structs.

## 5. Magic wand: top 3

1. **Split the SpacetimeDB twin files by domain** (`spacetime/module/src/lib.rs`, `src/sync/memory_caller.rs`) and derive the row blanks from one source. Biggest cut in change cost for every new column or reducer, and the area changing fastest.
2. **Type the `ReducerCaller` boundary** (`src/sync/writes.rs`): ids, timestamps, statuses, verdicts, poll scope. Removes a class of string-mismatch bugs that the in-memory twin currently re-implements by hand.
3. **One shared test-support layer**: `TaskBuilder`, the `unwrap` allow in `clippy.toml`, and split 4–5k-line test files. Cuts fixture drift and makes failing tests findable.

## 6. CLAUDE.md improvements

- **Stale:** "Store seam" paragraph (see section 1). Re-check "Layout-cache coherence" and "board's read source" against code, since the seam was removed in #4916.
- **Too dense (Medium):** 193 lines but ~32 KB; paragraphs of 700–1700 characters (lines 19, 21, 29, 64, 66, 68, 72, 76, 80, 82). Contains changelog prose ("no longer unchecked", "Pre-existing and not a regression in your branch", "See task #4909"). Move the tarpaulin/spacetime caveats, the CI job description and the spacetime module details into `docs/testing.md`; rewrite the rest as short imperative bullets with no task numbers.
- **Missing (Low):** a short "Testing" pointer (where fixtures live, unit vs `tests/`, when to snapshot) and a one-line specs index; `docs/specs` has 25 files.
- All `src/...rs` and `docs/...` paths cited in CLAUDE.md exist.

## Specs

- **No `docs/specs/cli.allium` (High):** `src/cli/agent_tree.rs`, `statusline.rs`, `caller_headers.rs`, `agent_diff.rs` have command contracts, exit codes and caller-identity headers with no spec. Use `allium:distill`.
- **`src/sync` thinly specified (Medium):** only `sync.allium` and `spacetime-memory-store.allium`. Nothing for `sdk_connector`, `writes`, `rows`, `session`, reconnect and write ordering.
- **`src/spacetime` thinly specified (Medium):** `managed_store`, `snapshot`, `restore`, `dump`, `cli_store` have no spec. Add a snapshot/restore/store-selection spec.

## Prioritised actions

**Quick wins**
1. Fix stale "Store seam" text in `CLAUDE.md` and `docs/conventions.md`; trim `CLAUDE.md` changelog prose.
2. `clippy.toml` `allow-unwrap-in-tests`; delete the ~106 per-module allows.
3. Delete `is_complete`/`COVERED_DOMAINS` scaffolding; `normalize_server` helper; `fire(tx, v)` helper.
4. `enum PollScope`; `FromStr` for verdict, clear-mode and `PrState`.

**Larger efforts**
5. Split `spacetime/module/src/lib.rs` and `memory_caller.rs` by domain; share the row blanks.
6. Type the `ReducerCaller` boundary; `sdk_connector` macros and function splits.
7. `TaskBuilder` fixture; split oversized test files; move inline test modules out.
8. Decompose the long functions in section 3 (`bootstrap_inner`, `handle_key_board_normal`, `dispatch_task`, `update_epic`, `migrate_v106`, `upsert_feed_tasks_inner`); split `startup.rs`.
9. Specs: `cli.allium`, sync/spacetime snapshot specs.
10. Replace `LocalBoardReads` with an in-memory `BoardReads`; add a fake-connector conformance test; per-file coverage report.
