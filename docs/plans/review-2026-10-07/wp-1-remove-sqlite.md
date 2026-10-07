# Remove SQLite entirely

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Delete the SQLite store from dispatch, so that the store ports and the SpacetimeDB store (real or in-memory) are the only storage left.

## Context

This work package addresses findings from the 2026-10-07 codebase review (`docs/plans/review-2026-10-07/report.md`, §1). The user decided on 2026-10-07 to **remove SQLite entirely**, not gate it behind `test-support`. Live data is in SpacetimeDB, machine identity is in `host.json`, and no production path opens a SQLite database (`storage.allium: StoreInUseNeverOpensSqlite`). What remains is dead weight that production still pays for.

This is a large refactor. Do it in stages, run the full suite after each stage, and commit per stage. Spec first: change `storage.allium` before the code.

## Findings

### 💡 Store ports and SQLite adapter share `src/db` (`src/db/mod.rs`, `src/db/queries/*.rs`)

**Issue:** `src/db/mod.rs` holds 22 live port traits (`TaskStore`, `TaskReadStore`, `SharedReader`, `SharedWriter`, …) and patch/filter types (`TaskPatch`, `EpicPatch`, `LearningFilter`, `UsageQuery`, `RemovedFeedTask`), imported by ~40 production files. Next to them is a SQLite adapter: every query method is `if let Some(port) = self.shared_*() { return … }` followed by a SQLite body (guards: `queries/tasks.rs` 35, `epics.rs` 14, `settings.rs` 14). Production always attaches ports, so the SQLite bodies run only under `Database::open_in_memory_unattached` / `Database::open` tests. The layering is inverted too: `sync` imports `crate::db::bump_decode_fallback` (`sync/rows.rs` ×5, `sync/decode.rs` ×3) and `crate::db::parse_datetime` (`sync/decode.rs`).

**Fix:** Move the ports, patch/filter types and shared helpers into a new `src/store/` module (or keep the `db` name if the churn is not worth it; decide and record why). `Database` becomes a router over the attached ports with no SQLite inside. Delete every SQLite body and the `local_*` settings helpers.

### 💡 Production builds a SQLite schema for a placeholder (`src/runtime/mod.rs::placeholder_database`)

**Issue:** `placeholder_database` calls `Database::open_in_memory_unattached`, which clones `SCHEMA_TEMPLATE`, built by replaying the full migration chain (`src/db/migrations.rs`, 3342 lines). It keeps `rusqlite` (`bundled`, `backup`) and `tokio-rusqlite` as production dependencies (`Cargo.toml`).

**Fix:** Construct the router without a SQLite connection. Delete `migrations.rs`, `SCHEMA_TEMPLATE`, the v71 backup path, `Database::open`, `open_in_memory_unattached`, `spacetime::dump_from_sqlite` (`src/spacetime/dump.rs`, no production caller) and both crates from `Cargo.toml`.

### 💡 Host identity still has SQLite fallbacks (`src/db/queries/settings.rs`)

**Issue:** `ensure_host_identity`, `rename_host`, `user_identity`, `adopt_user_identity*` read `host.json` when `host_file_dir()` is set and fall back to SQLite `settings` rows otherwise.

**Fix:** Make the host file the only source. Tests that relied on the fallback get a temp host-file dir. Check `host.allium` and `cli.allium: CliCommandsNeedAHostFile` still hold.

### 💡 Duplicated "drop undecodable row" block (`src/sync/rows.rs`)

**Issue:** Five copies of bump-counter / `warn!` / drop at the `bump_decode_fallback` call sites.

**Fix:** While moving `bump_decode_fallback` out of `db`, fold the five blocks into one helper that takes the row kind.

## Changes

| File | Change |
|------|--------|
| `docs/specs/storage.allium` | Spec first: drop `LocalStore`, `DatabaseConnection`, journal modes; keep the data directory and its files (add the `store-identity` pin file, `startup/store_pin.rs::STORE_PIN_FILE`). Run `allium check`. |
| `docs/specs/observability.allium` | Remove or retarget `DbCallSlowWarning` if `db_call` goes away. |
| other `docs/specs/*.allium` | `grep -rn -i sqlite docs/specs` and fix every reference. |
| `src/db/mod.rs` → `src/store/` | Move ports, patch/filter types, `parse_datetime`, `bump_decode_fallback`. `Database` keeps only routing. |
| `src/db/queries/*.rs` | Delete SQLite bodies; keep only the routing (or move routing into the router). |
| `src/db/migrations.rs`, `src/db/tests/` | Delete; port any test that checks routing behaviour to the in-memory store. |
| `src/runtime/mod.rs` | `placeholder_database` no longer touches SQLite. |
| `src/spacetime/dump.rs` | Delete `dump_from_sqlite` and its re-export in `src/spacetime/mod.rs`; check `snapshot.rs` doc reference. |
| `src/sync/rows.rs`, `src/sync/decode.rs` | Import from the new module; one drop-row helper. |
| ~35 test files using `open_in_memory_unattached` / `Database::open` | Move to `Database::open_in_memory` (memory store attached) or a store test helper. Includes `src/feed/cycle.rs`, `src/feed/mod.rs`, `src/sync/tests/identity.rs`, `src/spacetime/tests/mod.rs`, `src/setup/purge_tests.rs`. |
| `Cargo.toml` | Remove `rusqlite`, `tokio-rusqlite`. |
| `docs/invariants.md` | Rewrite "DB connection model"; fix the line claiming SQLite bodies serve the in-memory test database. |
| `docs/architecture.md`, `docs/conventions.md`, `docs/module-map.md`, `docs/reference.md`, `CLAUDE.md` | Remove SQLite references ("Stack" line, `src/db` rows). Coordinate with WP3, which edits the same docs. |
| `.github/workflows/ci.yml` | Re-measure coverage after the deletion; raise `--fail-under` deliberately if the figure rises (record measured value and date in the comment). |

## Verification

- [ ] `allium check docs/specs/storage.allium` passes
- [ ] `cargo build` and `cargo clippy --all-targets -- -D warnings` clean
- [ ] `cargo test` — all pass (redirect output, don't pipe)
- [ ] `grep -rn "rusqlite\|open_in_memory_unattached\|SCHEMA_TEMPLATE" src tests Cargo.toml` returns nothing
- [ ] `./scripts/check-doc-paths.sh`, `check-doc-symbols.sh`, `check-doc-headings.sh` pass
- [ ] `cargo tarpaulin --engine llvm --exclude-files 'src/spacetime/bindings/*'` with `spacetime` off `PATH`; record the figure in `ci.yml`
- [ ] Run `allium:weed` on `storage.allium` and the specs you touched
