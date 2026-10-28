# Cleanups

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix the small residues of the 2026-10-23 fixes: prompting git, sync/wrap-up overlap, editor field trimming and headers, and the CLI's store-pin message.

## Context

This work package addresses findings P1–P5 from `docs/plans/2026-10-28-codebase-review.md`.

## Findings

### 🟢 Low — P1: a remote that prompts now hangs until the timeout (`src/process.rs:269`)

**Issue:** `run_bounded` puts the child in its own process group (`process_group(0)`). A background-group child that reads `/dev/tty` (ssh passphrase, HTTPS credential prompt) is stopped by SIGTTIN, so `dispatch repo sync` reports "timed out after 120s" instead of git's real error.

**Fix:** Set `GIT_TERMINAL_PROMPT=0` and `GIT_SSH_COMMAND='ssh -o BatchMode=yes'` (respecting an existing `GIT_SSH_COMMAND` by appending the option) for bounded git calls; add a line next to `run_bounded` in `docs/conventions.md`.

### 🟢 Low — P2: a wrap-up fast-forward can race a repo sync (`src/dispatch/finish.rs:239`)

**Issue:** `fast_forward_base` runs `git merge --ff-only` in the primary checkout without consulting `App::repo_syncs_in_flight` (`src/tui/mod.rs`). A sync and a wrap-up on one checkout can collide on `index.lock`, and the sync's abort path may abort the other's merge.

### 🟢 Low — P3: single-line editor fields keep leading spaces (`src/editor.rs:172`)

**Issue:** `section_value` no longer trims the start (needed for DESCRIPTION), but it feeds TITLE, REPO_PATH, BASE_BRANCH, PLAN, URL and FEED_COMMAND too (`parse_editor_output`, around lines 510-519; `parse_epic_editor_output`). ` main` in BASE_BRANCH is stored with the space.

**Fix:** Trim the single-line sections where they are read; keep DESCRIPTION as is. Test each field.

### 🟢 Low — P4: a description line equal to a known header ends the description (`src/editor.rs::parse_sections_in`)

**Issue:** A description containing a line `--- STATUS ---` loses the lines after it and can override the real status.

### 🟢 Low — P5: the CLI's store-pin refusal uses the board's wording (`src/store_connection/mod.rs::check_store_identity`)

**Issue:** It returns `StartupAbort::StoreSwitched`, whose text (`src/startup/launch.rs:325-336`) says "the board has not started … run `dispatch tui` again" — wrong for `dispatch repo set-verify` and friends.

**Fix:** A CLI-specific variant or message: "this command wrote nothing; unset `DISPATCH_SPACETIME_SERVER`, or run `dispatch tui --accept-store-switch` to move to this store". Test the message.

## Decisions

### P2: how far the in-flight rule reaches (`src/dispatch/finish.rs:239`)

**Question:** Should a sync and a wrap-up rebase of the same repo exclude each other? This extends `OneSyncInFlightPerRepo` in `docs/specs/repo-sync.allium`.

**Options:**
- (a) Refuse a sync while a wrap-up of that repo runs, and make a wrap-up wait or fail while a sync runs. Costs a spec change and wiring the wrap-up path (`TaskService::serve_session_request`) to the in-flight set.
- (b) Accept the race and document it. Costs nothing; a rare collision stays possible.

### P4: how headers are recognised (`src/editor.rs::parse_sections_in`)

**Question:** Which fix for header-shaped lines inside a description?

**Options:**
- (a) Treat a header as a section start only when it is the next expected section in order. Small; a description can still not contain the *next* section's header.
- (b) Escape such lines on write and unescape on read. Complete; touches both directions and needs round-trip tests.

Ask the user with `AskUserQuestion` before implementing either decision item. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `src/process.rs` (or the git call sites) | P1 non-interactive git env |
| `docs/conventions.md` | P1 note next to `run_bounded` |
| `src/dispatch/finish.rs`, `src/tui/`, `docs/specs/repo-sync.allium` | P2 per decision |
| `src/editor.rs` | P3 trimming; P4 per decision |
| `src/store_connection/mod.rs`, `src/startup/launch.rs` | P5 CLI message |

## Verification

- [ ] New tests fail first, then pass
- [ ] `cargo test --no-fail-fast`
- [ ] `cargo fmt --check && ./scripts/check-doc-paths.sh`
