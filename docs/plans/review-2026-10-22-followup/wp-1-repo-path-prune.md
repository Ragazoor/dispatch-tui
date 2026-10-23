# Repo-Path Prune Safety

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Stop `dispatch prune-repo-paths` from deleting other hosts' saved repo paths and their verify commands on the strength of one machine's disk.

## Context

This work package addresses findings from the code review in `docs/plans/2026-10-22-codebase-review-followup.md` (A1, T1).

## Findings

### 🟡 Medium: `prune-repo-paths` judges a shared table by one machine's disk (`src/cli/commands.rs:174`)

**Issue:** `prune_repo_paths` (`src/cli/commands.rs::prune_repo_paths`) deletes each `RepoPath` row whose path does not exist on this host. `RepoPath` (`spacetime/module/src/tables.rs:269-279`) is a public, store-wide table with no host column, and `verify_command` lives in the same row. The spec rule `PruneRepoPathsViaCli` in `docs/specs/cli.allium` says "Existence is judged on THIS machine's filesystem" but the delete is store-wide. Scenario: operator A runs `dispatch prune-repo-paths`; host B saved `/home/b/code/x` with a verify command; the path does not exist on A, so the row and B's verify command are gone for everyone. An unmounted drive on one host has the same effect.

**Fix:** See Decisions. Spec first (`docs/specs/cli.allium`), then tests, then code.

### 🟢 Low: No test pins what prune does to a row with a verify command (`src/cli/commands.rs:174`)

**Issue:** Whatever the decision, nothing tests whether a pruned row's `verify_command` is kept or lost.

**Fix:** Add a test beside the existing `prune_repo_paths` tests that seeds a missing path with a non-empty verify command and asserts the chosen behaviour.

## Decisions

### How prune treats rows other hosts saved (`src/cli/commands.rs:174`)

**Question:** What should `prune-repo-paths` do with a missing path when the row may belong to another host?

**Options:**
- **(a) Host-scoped rows.** Add a host column to `RepoPath` and prune only this host's rows. Correct, but a module schema change, a twin mirror, conformance tests and a `RULES_VERSION` bump. Depends on the open trust-model decision (one operator per store or not).
- **(b) Keep rows with a verify command unless `--force`.** Small CLI-only change; other hosts' plain paths can still be pruned.
- **(c) Dry-run list then confirm.** Print what would go and ask y/N (non-interactive: refuse without `--yes`). Cheapest; relies on the operator to recognise foreign paths.

**Outcome (2026-10-23): (a), confirmed by the user.** Repo paths are per host; boards are shared. `path` stays the row identity; `host` is a stamp written by every `save_repo_path` (insert and update), so a row saved before the stamp (`host = ""`) is adopted by the first host to save it again and is never pruned until then. `list_repo_paths` is not host-filtered (picker, feeds and verify lookup are unchanged); prune uses `list_repo_paths_saved_by_this_host`. `RULES_VERSION`/`BOARD_RULES_VERSION` are 12.

## Changes

| File | Change |
|------|--------|
| `docs/specs/cli.allium` | Update `PruneRepoPathsViaCli` to the chosen behaviour |
| `src/cli/commands.rs` | Implement the chosen option in `prune_repo_paths` |
| `src/cli/` tests | Test for a row with a verify command (T1) |
| `spacetime/module/src/tables.rs`, `src/sync/memory_caller/`, `tests/memory_caller_conformance.rs`, `RULES_VERSION` pair | Only for option (a) |

## Verification

- [ ] Run the task's verify command — all pass
- [ ] `cargo test --no-fail-fast` green
- [ ] New test fails before the code change and passes after
