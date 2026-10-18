# Git, Worktree and File-Write Safety

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Finish the git error handling and atomic-write fixes that landed only partly.

## Context

This work package addresses findings from the 2026-10-18 codebase review (`docs/plans/2026-10-18-codebase-review.md`, finding numbers match that report). Verify each finding in the code before fixing; it may have moved.

## Findings

### Medium: Hook and protected-branch push failures still "retryable" (`src/repo_sync.rs:346`)

**Issue:** `is_push_rejection` markers match `[remote rejected] … hook declined`, so these become retryable `PushRejected`.

**Fix:** Match `[rejected]` (not `remote`), `non-fast-forward`, `fetch first` only; send `[remote rejected]` to `SyncError::Other`. Add auth, network and hook tests in `src/repo_sync/tests.rs`.

### Low: Atomic write replaces a symlinked file (`src/atomic_write.rs:48`)

**Fix:** Write to the canonical target when the path is a symlink. Add a symlink test.

### Low: Concurrent-update window remains (`src/dispatch/trust.rs`, `src/setup/mod.rs`)

**Fix:** Re-read immediately before write to narrow the window; correct the docs that claim it is closed.

### Low: `add_worktree` matches English stderr (`src/dispatch/worktree.rs:~650`)

**Fix:** Use `rev-parse --verify refs/heads/<name>` to choose `-b` or `-B`; do not parse stderr.

### Low: `pull_base` aborts blindly (`src/dispatch/finish.rs:~150`)

**Fix:** Abort only when a merge is in progress; report an abort failure. Confirm a test covers the ahead-branch refusal in `src/dispatch/tests/worktree_safety.rs`.

### Low: `ensure_worktrees_excluded` (`src/dispatch/worktree.rs:~602`)

**Fix:** Write atomically; resolve the git common dir so repos with a `.git` file get the exclude.

### Low: Uninstall prompts error on EOF (`src/setup/uninstall.rs:50`, `:192`)

**Fix:** Treat EOF as a decline.

## Changes

| File | Change |
|------|--------|
| `src/repo_sync.rs`, `src/repo_sync/tests.rs` | Push classification and tests |
| `src/atomic_write.rs` | Symlink-aware persist |
| `src/dispatch/trust.rs`, `src/setup/mod.rs` | Narrow read-modify-write window |
| `src/dispatch/worktree.rs` | Branch existence check, atomic exclude |
| `src/dispatch/finish.rs` | Conditional merge abort |
| `src/setup/uninstall.rs` | EOF declines |

## Verification

- [ ] Run existing tests — all pass
- [ ] New tests for each change above
- [ ] Run the repo's verify command from get_task
