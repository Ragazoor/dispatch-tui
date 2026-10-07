# Coverage trap and docs refresh

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make local coverage runs safe by default, and fix the stale lines in `CLAUDE.md`, `docs/module-map.md` and `storage.allium`.

## Context

This work package addresses findings from the 2026-10-07 codebase review (`docs/plans/review-2026-10-07/report.md`, §2 and §6). WP1 (remove SQLite) rewrites `storage.allium` and SQLite mentions in the docs. **Leave SQLite-specific wording (the "Stack" line, `src/db` rows, `docs/invariants.md`'s connection model) to WP1**; this package fixes everything else. Merge `main` before wrapping up, as WP1 may land first.

## Findings

### 💡 Local coverage trap (`tests/spacetime_module.rs`, `CLAUDE.md`)

**Issue:** `CLAUDE.md` says "keep `spacetime` off `PATH`" for local coverage but not how. If it is on `PATH` (it installs to `~/.local/bin`, beside other tools), tarpaulin fails all 62 `tests/spacetime_module.rs` tests with `status Some(1)` from `spacetime publish` (task #4909). Reproduced in this review. The test file's module doc already says the Coverage job relies on the skip.

**Fix:** Make the target skip itself under tarpaulin as it does when `spacetime` is absent: tarpaulin sets `cfg(tarpaulin)`, so check `cfg!(tarpaulin)` in the availability check (declare the cfg in `Cargo.toml` `[lints.rust] unexpected_cfgs` if needed). Update the module doc. Then drop the PATH caveat from `CLAUDE.md`'s coverage line, and from `docs/testing.md` and the `ci.yml` comment if they repeat it. Add a test or self-test that the skip predicate is true under `cfg(tarpaulin)` if practical. Spec check: `spacetime-memory-store.allium: ConformanceIsCiGated` must still hold (the Test job still runs them).

### 🔵 Stale `src/sync/` entry (`CLAUDE.md`, "Subsystem entry points")

**Issue:** Says `BoardReads` has "one implementation over SQLite and one over the subscription", picked by `--spacetime-server`, "unset on every board today". Only `SubscriptionBoardReads` (`src/sync/board_reads.rs`) exists and every board runs on a store. Contradicts the "Live data is in SpacetimeDB" line at the top.

**Fix:** Describe the single implementation.

### 🔵 Incomplete `src/cli/` entry (`CLAUDE.md`, `docs/module-map.md`)

**Issue:** Lists `agent_tree`, `caller_headers`, `statusline`; omits `agent_diff`, `agent_tree_agents`, `commands`, `store_import`.

**Fix:** List them, or describe the directory without an enumerated list that goes stale.

### 🔵 Imprecise test citation (`CLAUDE.md`, "Agent Working Directory")

**Issue:** Cites `src/dispatch/tests.rs` for `dispatch_agent_opens_tmux_window_in_worktree_not_parent_repo`; the test is in `src/dispatch/tests/agent_launch.rs`.

**Fix:** Cite `src/dispatch/tests/agent_launch.rs::dispatch_agent_opens_tmux_window_in_worktree_not_parent_repo`.

### 🔵 Module map gaps (`docs/module-map.md`)

**Issue:** No row for `src/spacetime/` (managed_store, cli_store, dump, import, restore, snapshot, store, bindings), `src/startup/` (config, host, launch, retire, store_pin), `src/host_file/`, `src/keybindings.rs` + `keybindings/`, `src/backoff.rs`. The `sync/board_reads.rs` row still says the read source is "chosen at bootstrap by whether a store is configured".

**Fix:** Add rows; fix the `board_reads` row.

### 🔵 `storage.allium` `DataDirectory` omits the pin file (`docs/specs/storage.allium`)

**Issue:** "What lives here" lists the host file, `app.log`, store address record, trajectories and feed scripts, but not the `store-identity` pin file written beside the database (`src/startup/store_pin.rs::STORE_PIN_FILE`, commit 45e9c41a).

**Fix:** Add it. If WP1 has already rewritten `storage.allium` and included it, skip.

## Changes

| File | Change |
|------|--------|
| `tests/spacetime_module.rs` | Skip under `cfg(tarpaulin)`; update module doc. |
| `Cargo.toml` | Declare `tarpaulin` in `unexpected_cfgs` if the lint fires. |
| `CLAUDE.md` | Coverage line, `src/sync/` and `src/cli/` entries, test citation. |
| `docs/testing.md`, `.github/workflows/ci.yml` | Remove the now-unneeded PATH caveat where repeated (keep the CI note that the Coverage job does not install `spacetime`). |
| `docs/module-map.md` | New rows; fix `cli/mod.rs` and `sync/board_reads.rs` rows. |
| `docs/specs/storage.allium` | Add the pin file to `DataDirectory`. |

## Verification

- [ ] `cargo test` — all pass (with `spacetime` on `PATH`, `tests/spacetime_module.rs` still runs)
- [ ] `cargo tarpaulin --engine llvm --exclude-files 'src/spacetime/bindings/*'` **with `spacetime` on `PATH`** now succeeds
- [ ] `allium check docs/specs/storage.allium`
- [ ] `./scripts/check-doc-paths.sh`, `check-doc-symbols.sh`, `check-doc-headings.sh` pass
