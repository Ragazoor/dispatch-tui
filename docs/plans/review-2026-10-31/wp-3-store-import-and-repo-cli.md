# Store Import and Repo CLI

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `dispatch store import` leave a running board's store alone, and make the `repo` subcommands write once, keep other hosts' rows and match their docs.

## Context

Findings C1–C6 and C9 from `docs/plans/2026-10-31-codebase-review.md` (fresh area `src/cli/`). Do not run the TUI to test this; the tests use temp directories. A reducer change in `config.rs` is mirrored in `src/sync/memory_caller/` with a conformance case.

## Findings

### 🔴 High — C1: `store import` stops a running board's store (`src/cli/store_import.rs::import_store`; `src/spacetime/managed_store.rs::ManagedStore::bring_up`, `::stop_on_exit`)

**Issue:** when a managed board already runs on 127.0.0.1:3000, `bring_up` takes `AddressAction::Adopt` and sets `held = true`; at the end the import calls `stop_on_exit`, which stops any held store. The board loses its store mid-session. `startup.allium: ImportBringsUpTheManagedStore` says the import leaves the machine as it found it.

**Fix:** stop only a store the import spawned (`stop_spawned_store`), or refuse when the managed address already answers and name `--spacetime-server http://127.0.0.1:3000`. Add a test where the probe reports a store answering.

### 🟡 Medium — C2: `repo set-verify` writes the path before the command is checked (`src/cli/commands.rs::set_verify`)

**Issue:** it calls `save_repo_path`, then `set_verify_command`, which the module refuses on a newline. `dispatch repo set-verify /typo "a\nb"` exits 1 but leaves a new repo-path row. `tests/cli.rs::dispatch_repo_set_verify_rejects_newline` checks only the message.

**Fix:** validate `\n`/`\r` before any write (as `handle_set_verify_command` does), or one reducer that saves and sets in one transaction. Assert nothing was written.

### 🟡 Medium — C3: `repo set-verify` takes over another host's repo row (`spacetime/module/src/config.rs::save_repo_path`; caller `src/cli/commands.rs::set_verify`)

**Issue:** `save_repo_path` stamps `host` on update too. Host B sets a verify command for a path only host A has; the row's host becomes B, and B's next `prune-repo-paths` deletes it, with A's verify command. Breaks `PruneRepoPathsViaCli`.

**Fix:** do not re-stamp `host` on an existing row from `set_verify` (call `save_repo_path` only when absent, or add a "touch without adopting" path).

### 🟢 Low — C4: prune deletes repos on an unmounted disk (`src/cli/commands.rs::prune_repo_paths`)

**Issue:** `Path::exists()` is false on a permission error or a detached disk; the row and verify command are deleted for good.

**Fix:** delete only on `ErrorKind::NotFound` from `try_exists`/`metadata`.

### 🟢 Low — C5: `clear-verify` and blank commands disagree with the docs (`src/store/mod.rs::RepoConfigStore::set_verify_command`)

**Issue:** the doc says unknown paths are inserted on set, ignored on clear, and whitespace means None. The reducer refuses unknown paths and stores a blank command. `repo clear-verify /unknown` fails; `repo set-verify p "  "` prints success. The MCP handler comment "DB normalises them to None" is wrong too.

**Fix:** correct the docs; reject or trim blank commands.

### 🟢 Low — C6: the import's scratch store is not reaped and lives in `/tmp` (`src/cli/store_import.rs::read_data_dir_with`, `::data_dir_in_use`)

**Issue:** SIGTERM with no wait or KILL, then the TempDir drops while the store may write; `store.log` is deleted on a start failure; a full store copy goes to `/tmp` (RAM-backed on some hosts). `data_dir_in_use` compares a canonical `--from` with the raw `--data-dir` text and misses a `spacetime start` with the default data dir.

**Fix:** wait with a timeout then KILL; keep or print the log path on failure; put the scratch dir on disk (e.g. under the data dir); canonicalise both sides and treat a missing `--data-dir` as the default.

### 🟢 Low — C9: the status line can exit non-zero (`src/cli/statusline.rs::run`; `src/main.rs::cmd_caller_headers`)

**Issue:** `print!` panics on a closed pipe; `StatusLineDecorator` promises exit 0.

**Fix:** `let _ = write!(std::io::stdout(), …)`.

## Changes

| File | Change |
|------|--------|
| `src/cli/store_import.rs` | Stop only a spawned store; reap the scratch store; scratch dir on disk; better in-use check |
| `src/cli/commands.rs` | Validate before write; no host re-stamp; `try_exists` in prune |
| `spacetime/module/src/config.rs` + twin | Optional save-and-set reducer / non-adopting touch |
| `src/store/mod.rs` | Fix `set_verify_command` doc; blank handling |
| `src/cli/statusline.rs`, `src/main.rs` | Non-panicking writes |
| `tests/cli.rs`, `src/cli/store_import/tests.rs` | Adopted-store test, nothing-written test, blank command test |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass
- [ ] New test: import with a store already answering does not stop it
- [ ] New test: refused `set-verify` leaves no repo-path row
- [ ] `cargo clippy --all-targets -- -D warnings` clean
