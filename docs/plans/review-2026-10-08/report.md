# Codebase review — 2026-10-08 (task #36870)

Scope: whole repo at `9a05ecd3` (branch `36870-codebase-review`, on top of `main`). Measured: `cargo clippy --all-targets -- -D warnings` (clean); `cargo test --no-fail-fast` (5815 passed, 0 failed, 1 ignored); `cargo tarpaulin --engine llvm` (90.49% = 19126/21136 hand-written lines; 77.34% raw, because the 863-line generated `src/spacetime/bindings/mod.rs` and its table files count as uncovered). Static counts are grep/awk and approximate. Findings from earlier reviews (latest: `review-2026-10-07`, all five work packages landed) are not repeated.

## Executive summary

- **Healthy.** Clippy clean, tests green, SQLite fully removed, `sdk_connector` split, every function over 120 lines split. Layering holds: no `sync`/`spacetime` imports in `tui`/`mcp`.
- **One flaky test.** The first full `cargo test` failed `tests/spacetime_module.rs::drop_closed_retired_feed_items_drops_what_the_keep_set_omits` ("Connection reset by peer" from `spacetime publish`). It passed on rerun. Plain `cargo test` stops at the first failing target, so the failure also hid every later target.
- **Coverage has 6 points of slack** (90.49% vs floor 84) once bindings are excluded. The floor can move up.
- **Weak spots are now the wiring layers:** `runtime/mod.rs` (413/603), `main.rs` (133/241), `sync/sdk_connector/wiring.rs` (0/74), `sync/sdk_connector/callers.rs` (11/91), `cli/agent_tree.rs`.
- **Big inline test modules remain** in `cli/agent_diff.rs` (1200 lines), `repo_sync.rs` (1190), `runtime/mod.rs` (930), `editor.rs` (1025), `agent_tree.rs` (750), `process.rs` (455).

## 1. Architecture and patterns

Layered and trait-seamed: `models` → store ports (`src/store`) → `service` → `mcp`/`tui`/`runtime`, Elm-style `App` core, SpacetimeDB behind `sync`. Consistent.

- **`Database` is a router that keeps no data (Low).** `src/store/mod.rs` (1845 lines) holds a type named `Database` that only forwards to attached ports. The name and the `--db`/`DISPATCH_DB` flag (names a directory; "the file it names is never opened") both point at a database that no longer exists. Rename on a quiet day (`Store`/`--data-dir`) or keep and document; at minimum the flag help should say "data directory".
- **MCP handlers still read through `state.db` in places (Low).** `mcp/handlers/poll_ownership.rs` and `mcp/handlers/tasks/dispatch.rs` call `state.db.get_task`/`get_epic` directly. Reads, so the writes-through-services invariant holds, but each bypasses service-level behaviour (decoding, tag derivation). Prefer `task_svc`/`epic_svc` so one read path exists.
- No SQLite in `Cargo.toml`; docs (`CLAUDE.md`, `docs/invariants.md`, `testing.md`) now match. Leftover-file wording about `tasks.db` is accurate.

## 2. Test coverage

90.49% hand-written. ~5,800 in-process tests; ~250 in `tests/`. Tests are behavioural (builders, snapshots, fake connectors).

| File | Covered | Uncovered |
|---|---:|---:|
| `runtime/mod.rs` | 413/603 (68%) | 190 |
| `cli/agent_tree.rs` | 476/599 (79%) | 123 |
| `main.rs` | 133/241 (55%) | 108 |
| `runtime/tasks.rs` | 404/504 (80%) | 100 |
| `sync/sdk_connector/callers.rs` | 11/91 (12%) | 80 |
| `sync/sdk_connector/wiring.rs` | 0/74 (0%) | 74 |
| `cli/mod.rs` | 81/134 (60%) | 53 |

- **`sdk_connector/callers.rs` 12% and `wiring.rs` 0% (Medium).** The split moved the decision logic into covered modules, but the table wiring and reducer callers are only exercised by the live-store tests, which skip under tarpaulin. Add a fake `RemoteTables`/caller seam, or accept and document that these are live-only.
- **Flaky live-store test (Medium).** `tests/spacetime_module.rs` publishes a module per test to a real server; a connection reset fails the test. Retry `spacetime publish` once on a transport error, or serialise publishes. Also `cargo test` stops at the first failing target, so CLAUDE.md should recommend `cargo test --no-fail-fast` when triaging.
- **Floor is loose (Low, quick win).** 90.49% vs 84. Raise toward 88 deliberately in CI (`.github/workflows/ci.yml`) so regressions are caught; do not set it automatically.

## 3. Complexity hotspots

Largest production files (code before `#[cfg(test)]`): `tui/mod.rs` ~2300, `spacetime/bindings/mod.rs` 2160 (generated), `setup/mod.rs` ~1970, `store/mod.rs` ~1840, `runtime/mod.rs` ~1000 (+930 tests), `tui/types.rs` ~1660, `models/tasks.rs`, `keybindings.rs` ~1770, `tmux.rs` ~1520.

Longest production functions now (rough, by span): `cli/agent_tree.rs::run_tree_action` 126, `service/tasks/crud.rs::update_task` 115, `runtime/commands.rs::dispatch_task` 107, `sync/sdk_connector/wiring.rs::wire_tables` 100, `runtime/epics.rs::exec_toggle_epic_group_by_repo` 98. None over 130; the 120-line limit from the last review holds except `run_tree_action`.

- `tui/mod.rs` and `setup/mod.rs` are the two largest hand-written files and mix several concerns (state struct plus view-building; setup steps). They are the next split candidates.
- 116 non-test places take a bare `i64` id (`task_id`, `epic_id`, `id`) although `TaskId`/`EpicId` exist.

## 4. Code smells

- **Inline test modules in large files (Low).** `cli/agent_diff.rs` (tests from line 778 of 1984), `repo_sync.rs` (486/1680), `editor.rs` (478/1503), `agent_tree.rs` (573/1321), `runtime/mod.rs` (1001/1934), `process.rs` (808/1263). Move to sibling `tests` modules as the last review did for others.
- **Primitive ids (Low).** As above; convert at the service boundary first.
- **`#[allow]` hygiene is good.** No `dead_code`/`too_many_arguments`; remaining allows are test helpers or documented invariants. No `TODO`/`FIXME` pile (5).
- **Doc: `docs/reference.md` is 913 lines, `conventions.md` 790.** Large but indexed from CLAUDE.md; consider splitting `reference.md` (CLI, config, store, troubleshooting).

## 5. Magic wand: top 3

1. **Make live-store tests deterministic.** Retry or serialise `spacetime publish` in `tests/spacetime_module.rs`; one flaky target stops the whole `cargo test` run and erodes trust in green.
2. **Split `tui/mod.rs` and `setup/mod.rs`**, and move the inline tests out of the six big files above. Biggest readability gain per line moved.
3. **Rename `Database`/`--db` to what they are** and route the remaining `state.db` reads through services, so the code stops describing a database that was removed.

## 6. CLAUDE.md improvements

`CLAUDE.md` is 136 lines and accurate; cited paths exist. Suggestions:

- Add: use `cargo test --no-fail-fast` when triaging, because plain `cargo test` stops at the first failing target and hides later ones.
- Add: `tests/spacetime_module.rs` can fail on a one-off "Connection reset by peer"; rerun the single test before suspecting your change.
- Add: coverage figures — raw 77% includes generated bindings; compare hand-written figures only.
- The `--db` flag line should say it names a data directory, not a database file.

## Prioritised action items

**Quick wins**
1. Retry/serialise `spacetime publish` in `tests/spacetime_module.rs`; add the two CLAUDE.md test notes.
2. Route `poll_ownership.rs` and `tasks/dispatch.rs` reads through the services.
3. Raise the CI coverage floor deliberately after re-measuring.

**Larger efforts**
4. Move inline test modules out of `agent_diff`, `repo_sync`, `editor`, `agent_tree`, `runtime/mod`, `process`; split `run_tree_action`.
5. Split `tui/mod.rs` and `setup/mod.rs` by concern.
6. Cover or fake `sdk_connector/wiring.rs` and `callers.rs`; lift `runtime/mod.rs` and `main.rs`.
7. Rename `Database`/`--db`; adopt `TaskId`/`EpicId` in the 116 bare-`i64` signatures.
