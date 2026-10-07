# Codebase review — 2026-10-07 (task #16666)

Scope: whole repo at `main` 2e8d0715. Measured: `cargo clippy --all-targets -- -D warnings` (clean), `cargo test` (5979 passed, 0 failed, 1 ignored), `cargo tarpaulin --engine llvm --exclude-files 'src/spacetime/bindings/*'` with `spacetime` off `PATH` (86.27%, 21138/24501). Static counts come from grep/awk and are approximate. Findings from the 2026-08-16, 2026-08-28, 2026-10-02 and 2026-10-05 reviews are not repeated. All five 2026-10-05 work packages have landed (epic #12317).

## Executive summary

- Healthy. Clippy clean, all tests green, module boundaries hold (no `sync`/`spacetime` imports in `tui`/`mcp`, no `state.db` writes in `mcp`). The coverage gate now has 3.3 points of slack (86.27% vs floor 83).
- **`src/db` is now two things in one module.** The store *ports* (22 traits, patch/filter types) are live and imported by ~40 files. The *SQLite adapter* behind them (`migrations.rs` 3342 lines, `queries/` ~4k lines) never answers in production, yet production still builds a full SQLite schema at startup for a placeholder `Database`. This is the largest remaining structural debt.
- **`sync/sdk_connector.rs` is a 2058-line god file** (1830 production lines) and the least-covered file (42%, 197 uncovered lines).
- **Docs lag the store migration.** `CLAUDE.md` and `docs/module-map.md` still describe a SQLite `BoardReads` implementation that no longer exists, and the module map has no row for `src/spacetime/` or `src/startup/`.
- **Local coverage has a trap.** "Keep `spacetime` off `PATH`" fails 62 tests hard if missed; the binary lives in `~/.local/bin` beside other tools.

## 1. Architecture and patterns

Layered, trait-seamed: `models` → store traits (`db`) → `service` → `mcp`/`tui`/`runtime`, with an Elm-style `App` core and the SpacetimeDB store behind `sync`. Applied consistently.

- **Ports and SQLite adapter share `src/db` (Medium).** Every query method is `if let Some(port) = self.shared_*() { return … }` then an SQLite body (guards: `queries/tasks.rs` 35, `epics.rs` 14, `settings.rs` 14). Production always attaches ports (`src/runtime/mod.rs::StoreParts`), and `Database::open_in_memory` attaches the memory store too, so the SQLite bodies run only under `open_in_memory_unattached` tests. The layering is also inverted: `sync` imports helpers from `db` (`crate::db::bump_decode_fallback` in `sync/rows.rs` ×5 and `sync/decode.rs` ×3; `crate::db::parse_datetime` in `sync/decode.rs`). **Fix:** move the traits, patch/filter types and shared helpers to a `store` module; gate the SQLite adapter, `migrations.rs` and `queries/` behind `#[cfg(any(test, feature = "test-support"))]`.
- **Production builds a SQLite schema for a placeholder (Medium).** `src/runtime/mod.rs::placeholder_database` calls `Database::open_in_memory_unattached`, which clones `SCHEMA_TEMPLATE`, built by replaying the full migration chain (`src/db/mod.rs`). It keeps `rusqlite` (`bundled`, `backup`) as a production dependency. Falls out of the item above.
- **`Database::open(path)` is `pub` and ungated with no production caller (Low).** Only tests use it. Gate it with the adapter.
- **MCP context resources read through two paths (Low).** `src/mcp/handlers/context.rs` lists the own task via `state.db.get_task` but reads it via `state.task_svc.get_task`; learnings are read via `state.db.get_learning` while every learnings handler uses `state.learning_svc`. Route both through the services so list and read cannot disagree.
- **`list_entries` loads every approved learning and skill per page (Low).** `context.rs::list_entries` builds the full list, then filters by cursor. O(N) per page; fine today, grows with the knowledge base.

`storage.allium` now correctly scopes SQLite to the test fixture (the 2026-10-05 finding is resolved). MCP resources and store pinning are specced.

## 2. Test coverage

86.27% (21138/24501), bindings excluded. Floor 83.

| File | Covered | Uncovered |
|---|---:|---:|
| `sync/sdk_connector.rs` | 147/344 (42%) | 197 |
| `runtime/mod.rs` | 403/572 (70%) | 169 |
| `main.rs` | 133/241 (55%) | 108 |
| `db/queries/tasks.rs` | 537/636 (84%) | 99 |
| `runtime/tasks.rs` | 409/504 (81%) | 95 |
| `cli/agent_tree.rs` | 398/486 (81%) | 88 |
| `db/queries/epics.rs` | 223/291 (76%) | 68 |
| `cli/mod.rs` | 81/134 (60%) | 53 |
| `cli/store_import.rs` | 78/124 (62%) | 46 |
| `db/queries/usage.rs` | 15/57 (26%) | 42 |

- **Gating the SQLite adapter would also lift the figure.** `src/db/` is 2436 coverable lines; the dead SQLite bodies inside count against the floor. Re-measure and recalibrate after the move, never automatically.
- **`sdk_connector.rs` 42% (Medium).** Up from 26%, still the worst. Split it (§3) and cover the decision logic with the fake connector.
- **Local coverage trap (Medium, quick win).** `CLAUDE.md` says "keep `spacetime` off `PATH`" but not how, and getting it wrong fails all 62 `tests/spacetime_module.rs` tests ("status Some(1)" from `spacetime publish` under instrumentation, task #4909). Make the target skip itself under tarpaulin (`#[cfg_attr(tarpaulin, ignore)]` or an early return on `cfg!(tarpaulin)`), as CI already expects a skip there. Then the PATH caveat can go.
- Tests are behavioural (builders, snapshots, fake connectors). ~5,700 in-process tests to ~250 integration tests in `tests/`.

## 3. Complexity hotspots

Largest production files: `db/migrations.rs` 3342, `models/tasks.rs` 2703 (≈1950 lines are inline tests), `db/mod.rs` 2395, `feed/mod.rs` 2373 (≈1860 inline tests), `tui/mod.rs` 2319, `sync/sdk_connector.rs` 2058, `setup/mod.rs` 1980, `tui/types.rs` 1902, `runtime/mod.rs` 1858.

Functions over 120 lines (production):

| Lines | Function |
|---:|---|
| 188 | `tui/mod.rs::column_items_for_status_with_view_tasks` (new) |
| 179 | `runtime/mod.rs::bootstrap_inner` (was 191) |
| 152 | `tui/input/normal.rs::run_normal` (new) |
| 124 | `tui/ui/kanban/popups/task_detail.rs::render_task_detail_overlay` (new) |

`run_row`, `dispatch_key`, `run_tui` and `build_task_list_item` are now under 120. No function takes 8+ parameters. Deepest nesting (6+ levels): `db/queries/tasks.rs` 49 lines, `tui/mod.rs` 44, `runtime/tasks.rs` 43.

## 4. Code smells

- **God file: `sync/sdk_connector.rs` (Medium).** 1830 production lines: connection loop, decode, reducer callers, reconnect. Split by concern as `memory_caller/` already was.
- **Inline test modules in large production files (Low).** `feed/mod.rs` (~1860), `models/tasks.rs` (~1950), `runtime/editor.rs` (~970), `service/epics.rs` (~870), `tui/types.rs`. Move to sibling `tests` modules as was done for `setup/plugins.rs`.
- **Duplicated "drop undecodable row" block (Low).** Five copies in `sync/rows.rs` (bump counter, `warn!`, drop). One helper taking the row kind.
- **Hex encode/decode written three times (Low).** `mcp/handlers/context.rs`, `setup/plugins.rs`, `spacetime/managed_store.rs`. One small shared helper.
- **Primitive ids and statuses (Low).** `ContextUri::Learning(i64)`/`Task(i64)` wrap to newtypes only at use; ~33 non-test fns take `epic_id: i64`/`task_id: i64` though `EpicId`/`TaskId` exist (e.g. `subscribed_epics -> Vec<i64>`); `spacetime/module/src` defines status constants in `support.rs` but `blanks.rs` and `tasks_epics.rs` use literals.
- `#[allow]` hygiene is good: no `dead_code` or `too_many_arguments` allows left in hand-written code.

## 5. Magic wand: top 3

1. **Split `src/db` into `store` (ports) and a test-only SQLite adapter.** Removes ~7k lines from the production build, the startup schema replay and `rusqlite` from release builds, fixes the `sync → db` inversion, and stops dead bodies counting against coverage.
2. **Split `sync/sdk_connector.rs` by concern and cover it with the fake connector.** The last god file on the live data path, and the least-tested one.
3. **Make the docs describe the store that exists.** `CLAUDE.md`, `docs/module-map.md`, `docs/invariants.md` still describe a SQLite board read path; agents start every session from these.

## 6. CLAUDE.md improvements

`CLAUDE.md` is 15.7 KB / 137 lines (down from 19.8 KB). Every cited path and symbol exists.

- **Stale: the `src/sync/` entry** says `BoardReads` has "one implementation over SQLite and one over the subscription", chosen by `--spacetime-server`, "unset on every board today". Only `SubscriptionBoardReads` exists and every board runs on a store. Contradicts line 7.
- **Stale: "Stack"** lists "SQLite (rusqlite)" and not SpacetimeDB.
- **Incomplete: the `src/cli/` entry** lists `agent_tree`, `caller_headers`, `statusline`; it omits `agent_diff`, `agent_tree_agents`, `commands`, `store_import`.
- **Imprecise:** the working-directory paragraph cites `src/dispatch/tests.rs`; the test is in `src/dispatch/tests/agent_launch.rs`.
- **Coverage line:** say how to keep `spacetime` off `PATH` (it is in `~/.local/bin`), or drop the caveat once the test target self-skips.

Related docs: `docs/module-map.md` has no row for `src/spacetime/`, `src/startup/`, `src/host_file/`, `src/keybindings*`, `src/backoff.rs`; its `src/cli/mod.rs` and `sync/board_reads.rs` rows are stale as above. `docs/invariants.md` says the SQLite bodies serve "the test suite's in-memory database", but `open_in_memory` attaches the memory store; only `open_in_memory_unattached` reaches them. `storage.allium` `DataDirectory` omits the `store-identity` pin file (`startup/store_pin.rs::STORE_PIN_FILE`).

## Prioritised action items

**Quick wins**
1. Self-skip `tests/spacetime_module.rs` under tarpaulin; update the `CLAUDE.md` coverage line.
2. Fix the stale `CLAUDE.md`, `docs/module-map.md`, `docs/invariants.md` lines and the `storage.allium` pin file entry.
3. Route MCP context reads through `task_svc`/`learning_svc`.
4. Dedupe the `sync/rows.rs` drop block and the hex helpers; use status constants in the module.

**Larger efforts**
5. Split `src/db` into a `store` ports module and a test-gated SQLite adapter; re-measure coverage and recalibrate the floor deliberately.
6. Split `sync/sdk_connector.rs` and raise its coverage.
7. Split the three new long functions (`column_items_for_status_with_view_tasks`, `run_normal`, `render_task_detail_overlay`) and `bootstrap_inner`; move inline test modules out of the large production files.
