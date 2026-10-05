# Codebase review — 2026-10-05 (task #16666)

Scope: whole repo at `main` 7c19428b. Measured: `cargo clippy --all-targets -- -D warnings` (clean), `cargo test` (5831 passed, 0 failed, 1 ignored), `cargo tarpaulin --engine llvm` run with `spacetime` removed from `PATH`. Static counts come from grep/awk and are approximate. Findings from the 2026-08-16, 2026-08-28 and 2026-10-02 reviews are not repeated; most of the 2026-10-02 items have since landed (TaskBuilder, startup split, long-function splits, test-file splits, `LocalBoardReads` removal, spec additions).

## Executive summary

- The codebase is healthy: clippy clean, all tests green, layering holds. Nothing here is a firefight.
- **The coverage gate has thin slack.** Tarpaulin reports 76.79% overall. That figure includes 2,777 lines of generated SpacetimeDB bindings at 7% covered. Without them, coverage is 88.87% against a CI floor of 88. One mid-size untested file would trip the gate.
- **The uncovered code is mostly the new SpacetimeDB edge.** `sync/sdk_connector.rs` 26%, `cli/store_import.rs` 13%, `main.rs` 53%, `runtime/mod.rs` 69%, `cli/agent_diff.rs` 67%, `cli/mod.rs` 60%.
- **The SQLite layer is now a second, partly dead, store.** `src/db/` (about 20k lines incl. tests, 4.2k in `queries/`) is still compiled and tested, while docs say live data is in SpacetimeDB. `db/queries/learnings.rs` is 35% covered. `storage.allium` still specs SQLite journal modes.
- `CLAUDE.md` is 19.8 KB and still dense. It is also duplicated as `AGENTS.md` through a symlink, which is fine but undocumented.

## 1. Architecture and patterns

Layered, trait-seamed: `models` → store traits → `service` → `mcp`/`tui`/`runtime`, with an Elm-style `App` core. It is applied consistently. The SpacetimeDB layer (`sync`, `spacetime`, `spacetime/module`) is the newest seam, and it is split by domain now.

- **Two stores, one live (Medium).** `Database::open` is still called at `src/runtime/mod.rs:396` and `src/main.rs:607`. `docs/reference.md` says the live data is in SpacetimeDB and `tasks.db` is stale. What `src/db` still owns that production reads is not written down in one place. Decide and record: which tables stay in SQLite, which parts only serve import/tests. Then delete the rest (learnings queries are the obvious candidate: 35% covered, 144 uncovered lines).
- **`storage.allium` is about a backend that no longer holds the board (Medium).** It scopes "the local store" and journal modes. Retitle or trim it to what is still true.
- **`App` is still one wide type (Low, known).** 21 `impl App` blocks outside tests. Not worth a rewrite; keep new state out of it.
- **Env reads are scattered (Low).** 17 `env::var(` reads in production code. Read them once at the entry point and pass a config struct.

## 2. Test coverage

Run: 5831 passed. Coverage (llvm engine, `spacetime` off `PATH`): 20516/25642 lines = 80.0% by this per-file sum (tarpaulin's own header says 76.79%), **88.87% without**.

| File | Lines | Covered | Uncovered |
|---|---:|---:|---:|
| `sync/sdk_connector.rs` | 344 | 26% | 256 |
| `runtime/mod.rs` | 510 | 69% | 156 |
| `db/queries/learnings.rs` | 221 | 35% | 144 |
| `main.rs` | 246 | 53% | 115 |
| `cli/store_import.rs` | 118 | 13% | 103 |
| `db/queries/tasks.rs` | 624 | 84% | 99 |
| `runtime/tasks.rs` | 504 | 82% | 91 |
| `cli/agent_tree.rs` | 485 | 82% | 89 |
| `cli/agent_diff.rs` | 221 | 67% | 72 |

- **Generated bindings drag the number down (Medium).** `src/spacetime/bindings/` (216 files, 2,777 lines, 7% covered) is machine-written. Exclude it from tarpaulin (`--exclude-files 'src/spacetime/bindings/*'`) so the floor measures hand-written code. Then re-calibrate the floor deliberately.
- **Slack of 0.87 points (Medium).** The floor of 88 was set against 90.28%. Measured now is 88.87% without bindings. A single new 100-line untested file would fail CI.
- **`cli/store_import.rs` is 13% (Medium).** It is a one-shot data migration that touches the managed store. Its row logic lives in `spacetime::import_old_store` and is tested; the glue (probe, spawn, wait loop with `thread::sleep`, printing) is not. Add a fake-spawner test.
- **`sdk_connector.rs` 26% (Medium).** Needs a live server for most paths. Cover the decode and reconnect decisions with the fake connector, as the 2026-10-02 review proposed.
- **Behaviour vs implementation.** Tests are mostly behavioural (`TaskBuilder`, snapshots, mock process sequences). Positional `MockProcessRunner` queues are the main implementation-coupled style. `dispatch/mock_sequence.rs` (test-only) is the answer there.
- **Ratio.** About 5,800 unit/in-process tests to roughly 250 integration tests in `tests/`. Fine for this repo.

## 3. Complexity hotspots

Largest production files: `db/migrations.rs` 3340, `setup/plugins.rs` 2974, `models/tasks.rs` 2703, `feed/mod.rs` 2373, `db/mod.rs` 2353, `tui/mod.rs` 2290, `spacetime/bindings/mod.rs` 2160 (generated), `setup/mod.rs` 2060, `dispatch/mock_sequence.rs` 2059, `tui/types.rs` 1902.

Longest functions (heuristic, >120 lines, production only):

| Lines | Function |
|---:|---|
| 211 | `tui/input/table.rs:297 run_row` |
| 191 | `runtime/mod.rs:990 bootstrap_inner` |
| 150 | `cli/agent_tree.rs:1061 dispatch_key` |
| 129 | `runtime/mod.rs:630 run_tui` |
| 123 | `tui/ui/kanban/cards.rs:548 build_task_list_item` |

`bootstrap_inner` and `run_tui` were split on 2026-10-02 but are still among the longest. Functions with 8 parameters: `run_loop`, `build_runtime`, `upsert_feed_item`, and the generated `create_managed_role_epic_then`.

Notes:
- `setup/plugins.rs` is 2974 lines; about 770 are production, the rest is tests. Its tests are inline. Move them to a sibling `tests` module like the rest of the repo.
- `db/migrations.rs` at 3340 lines is append-only history. Large but low risk.

## 4. Code smells

- **Blocking sleeps (Low).** `std::thread::sleep` at `process.rs:257,755,766`, `cli/store_import.rs:135`, `runtime/editor.rs:264`. Fine in sync threads; confirm none run on the async runtime (`store_import.rs:135` is the one to check).
- **Clone density (Low).** `runtime/editor.rs` 65, `feed/mod.rs` 52, `runtime/mod.rs` 44 `.clone()` calls. Not a defect; look only if profiling points there.
- **`#[allow]` hygiene is good.** 107 `non_camel_case_types` and 38 `non_snake_case` are in generated or wire-format code. Only one `dead_code`, one `too_many_arguments`, one `unused_mut` remain. Check each is still needed.
- **Unwrap/expect.** About 3 production unwraps outside test modules (`test_log.rs`, `models/tmux_window.rs`). Good.
- **TODO/FIXME.** 8 in `src/`. Fine.
- **Duplication.** The 40-field task row is restated in the module, the in-memory twin and the SDK rows. A single source remains the main duplication risk. The parity tests (`bindings_parity.rs`, memory-caller conformance) catch drift but skip when `spacetime` is absent (CI Coverage job).

## 5. Magic wand: top 3

1. **Retire or fence off the SQLite store.** Highest impact on maintainability: ~20k lines and a second set of semantics that a reader must learn to ignore. Name the surviving purpose (import source? tests?) and delete the rest, starting with `db/queries/learnings.rs`.
2. **Exclude generated code from coverage and raise the real floor.** Cheap, and makes the gate honest. Pair it with fake-connector tests for `sdk_connector.rs` and `store_import.rs`.
3. **Run the spacetime conformance tests in CI.** `tests/spacetime_module.rs` and `tests/memory_caller_conformance.rs` skip in the Coverage job and fail under tarpaulin locally when `spacetime` is on `PATH`. Drift between the module and its in-memory twin is the biggest bug source left, and the guard is skipped where coverage is measured. A separate CI job with the CLI installed closes that.

## 6. CLAUDE.md improvements

- It is 19.8 KB. The 2026-10-02 slimming helped, but several paragraphs still carry rationale and task history (sandbox, `main` moving, worktree path warnings). Move the "main moves while you work" block (about 35 lines) to `docs/` and keep a two-line pointer plus the exact command pair.
- The "Store seam"/"where is live data" fact is only in `docs/reference.md`. Add one sentence to `CLAUDE.md`: live data is in SpacetimeDB; `tasks.db` is not authoritative. Agents hit this first.
- Document that `AGENTS.md` is a symlink to `CLAUDE.md`, so nobody edits it as a separate file.
- State the tarpaulin caveat (fails with `spacetime` on `PATH`) next to the coverage command. It is in `docs/testing.md` only.
- Say how to get the coverage number the CI gate sees (bindings are counted today).
- Already good: the verify-command pointer, the no-pipe test rule, the tmux socket rule, the spec-first order.

## Prioritised actions

Quick wins
1. Exclude `src/spacetime/bindings/*` from tarpaulin; recalibrate the floor (`.github/workflows/ci.yml`, `docs/testing.md`).
3. Add the live-data sentence and the `AGENTS.md` symlink note to `CLAUDE.md`; move the "main moves" block to `docs/`.
4. Retitle/trim `storage.allium` to match what SQLite still does.
5. Move inline tests out of `setup/plugins.rs`.

Larger efforts
1. Decide the fate of `src/db` and remove dead SQLite paths (learnings queries first).
2. Fake-connector and fake-spawner tests for `sdk_connector.rs`, `cli/store_import.rs`, `cli/agent_diff.rs`.
3. CI job that installs `spacetime` and runs the two conformance test files.
4. Split `run_row`, `bootstrap_inner`, `dispatch_key` further; replace 8-parameter `run_loop`/`build_runtime` with a context struct.
5. Central config struct for the 17 `env::var` reads.
