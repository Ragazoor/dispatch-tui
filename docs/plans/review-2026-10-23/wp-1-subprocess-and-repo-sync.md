# Subprocess and Repo Sync

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make a timed-out subprocess die with its children, and stop two repo syncs from running on one checkout at once.

## Context

This work package addresses findings from the code review in `docs/plans/2026-10-23-codebase-review.md` (A1, A2, A3, A4, §2.8). Spec first for A2: the "one sync per repo" rule belongs in `docs/specs/repo-sync.allium` before the code.

## Findings

### 🟡 Medium: `run_bounded` kills only the direct child on timeout (`src/process.rs::abandon`, `src/process.rs::run_bounded`)

**Issue:** `abandon` calls `child.kill()`; `run_bounded` never puts the child in its own process group. When `git fetch` over ssh hangs, git is killed at 120s but its `ssh` / `git-remote-https` child keeps running and holds the stdout/stderr pipes. The `drain` reader threads block until it dies: one leaked thread pair per hung fetch, and startup fetches once per saved repo (`exec_refresh_all_repo_sync`). The same grandchild makes the final `stderr_rx.recv_timeout(remaining())` wait out the whole deadline after git has exited and return empty stderr, so `repo_sync::is_push_rejection` misreads a rejected push as `Other`.

**Fix:** Spawn with `std::os::unix::process::CommandExt::process_group(0)` and kill the whole group in `abandon`, as `src/feed/exec.rs` does (`kill_process_group`). After the child has exited, wait for stderr only briefly (about 100 ms) instead of `remaining()`. Test through the `ProcessRunner` seam or a real short-lived shell child that spawns a sleeping grandchild (no wall-clock sleeps in tests: see `docs/testing.md` and `scripts/check-no-test-sleep.sh`).

### 🟡 Medium: Two syncs of the same repo can run at once (`src/tui/update/repo_sync.rs::handle_open_repo_sync_prompt`)

**Issue:** The prompt only checks `has_drift()`. Nothing records a sync in flight, and the drift marker stays until the result arrives (the fetch alone can take 120 s). Pressing sync, `y`, sync, `y` starts two runs; the second `git merge` fails on `index.lock` or `MERGE_HEAD`, and `src/repo_sync.rs::merge_origin_base` → `abort_after_conflict` runs `git merge --abort`, aborting the first run's merge.

**Fix:** Spec a rule in `docs/specs/repo-sync.allium` ("a repo with a sync in flight refuses another"), then keep a set of repo paths with a sync in flight in `App`, add on dispatch of the sync command, remove on success or failure, and refuse the prompt with a status message while the repo is in the set. Test the refusal in `src/tui/tests/`.

### 🟢 Low: A merge killed at the timeout is left half-done (`src/repo_sync.rs::merge_origin_base`)

**Issue:** The `GitFailure::Spawn` arm (which includes a timeout kill) returns without aborting. A merge killed mid-way leaves `MERGE_HEAD`; every later sync reports the checkout as dirty.

**Fix:** Call `git::abort_merge_if_in_progress` on that arm before returning. Test with the mock runner in `src/repo_sync/tests.rs`.

### 🟢 Low: `measure_repo` reads the default branch without expanding `~` (`src/repo_sync.rs::measure_repo`)

**Issue:** It passes `repo_path` raw to `detect_default_branch`, unlike its siblings. Latent: today's callers pass expanded paths, but a `~/x` caller would silently get base `main`.

**Fix:** `expand_tilde` at the top of `measure_repo`.

### 🟢 Low: No written rule that bounded children run in their own process group (`docs/conventions.md`)

**Issue:** Nothing tells an agent that a child which can spawn its own children (git over ssh/https) must run in its own process group.

**Fix:** After A1 lands, add one sentence next to the `run_bounded` convention in `docs/conventions.md` naming the rule and pointing at `src/process.rs::run_bounded`.

## Changes

| File | Change |
|------|--------|
| `src/process.rs` | `process_group(0)` on spawn; group kill in `abandon`; short stderr wait after exit |
| `docs/specs/repo-sync.allium` | Rule: one sync in flight per repo |
| `src/tui/update/repo_sync.rs` (+ `App` state) | Track syncs in flight; refuse a second prompt |
| `src/repo_sync.rs` | Abort merge on the `Spawn` arm; `expand_tilde` in `measure_repo` |
| `docs/conventions.md` | Process-group sentence |
| tests (`src/process` tests, `src/repo_sync/tests.rs`, `src/tui/tests/`) | Cover each finding |

## Verification

- [ ] Run the task's verify command — all pass
- [ ] `cargo test --no-fail-fast` green
- [ ] `cargo clippy --all-targets -- -D warnings` clean; `scripts/check-no-test-sleep.sh` and the doc checkers pass
