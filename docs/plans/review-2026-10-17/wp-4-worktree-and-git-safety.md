# Worktree and Git Safety

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Git operations in the operator's primary checkout fail closed, clean up after themselves, and never act on the wrong repo or branch.

## Context

This work package addresses findings 5–9, 32a, 33–35 of the 2026-10-17 code
review (`docs/plans/2026-10-17-codebase-review.md`). Relevant spec:
`docs/specs/dispatch.allium` (`PresenceIsNotYetReuse`), `docs/specs/repo-sync.allium`.
Spec first where behaviour changes.

## Findings

### 🟡 Husk directory treated as a reusable worktree (`src/dispatch/worktree.rs:521`, `:617`)

**Issue:** `worktree_is_reusable` only checks "exists and is a directory". A
leftover non-git dir at `.worktrees/<id>-<slug>` skips `git worktree add`;
`fast_forward_reused_branch` then runs `git -C <husk> merge --ff-only`, and
git discovers the parent repo, so the merge runs in the primary checkout.

**Fix:** Require `<path>/.git` to exist and `git -C path rev-parse --show-toplevel`
to equal the path; otherwise refuse with a clear message (or remove an empty
husk, if the spec says so).

### 🟡 Wrap-up pull leaves the primary checkout mid-merge (`src/dispatch/finish.rs:150`)

**Issue:** `pull_base` runs `git pull --no-rebase origin <base>` with no
`merge --abort` on failure, unlike `rebase_branch` and `merge_origin_base`.

**Fix:** On failure call the existing `abort(..., "merge", ...)`; or switch to
fetch + `merge --ff-only`.

### 🟡 Default branch with a slash truncated (`src/git.rs:178`)

**Issue:** `refname.rsplit('/').next()` turns `release/2024` into `2024`.

**Fix:** `refname.strip_prefix("refs/remotes/origin/")`.

### 🟡 Git preconditions fail open (`src/git.rs:288`, `:268`)

**Issue:** `dirty_files` and `current_branch` use `git_in` and ignore a
non-zero exit, so a failing `git status` reads as clean and branch `""`.

**Fix:** Use `git_checked` / `run_git`; return `Err` with stderr.

### 🟡 Every push failure reported as "origin moved, retry" (`src/repo_sync.rs:324`)

**Issue:** `push_base` maps any non-zero exit to retryable `PushRejected`.

**Fix:** Classify on stderr (`rejected`, `non-fast-forward`, `fetch first`)
or `--porcelain`; everything else to `SyncError::Other` with stderr.

### 🟡 `.worktrees/` never ignored in target repos (`src/dispatch/worktree.rs:576`, `src/git.rs:288`)

**Issue:** Nothing writes `info/exclude` and `parse_porcelain_files` does not
filter `.worktrees/`. In a repo whose `.gitignore` lacks it, `git status`
shows `?? .worktrees/` and every wrap_up/sync is refused as dirty.

**Fix:** When creating the `.worktrees` root, append `.worktrees/` to
`<git-common-dir>/info/exclude` idempotently. Verify first with a real git
repo in a temp dir that the dirty check trips today.

### 🟢 Low findings

- `src/dispatch/worktree.rs:596` — `git worktree add -B` resets an existing
  branch that may hold unpushed commits when the dir is gone. Refuse (or
  reuse) if the branch exists and is not an ancestor of the start point.
- `src/models/tasks.rs:929` (`slugify`) — no length cap; long or CJK titles
  exceed 255 bytes. Truncate on a char boundary (~50 chars), re-trim dashes.
  Check callers that rely on full slugs.
- `src/dispatch/finish.rs:67` — `FinishContext::timeout` doc says 60s;
  `SUBPROCESS_TIMEOUT` is 120s.

## Changes

| File | Change |
|------|--------|
| `docs/specs/dispatch.allium` | Reuse requires a registered worktree; exclude rule |
| `src/dispatch/worktree.rs` | Reuse check; `info/exclude`; `-B` guard |
| `src/dispatch/finish.rs` | Abort on failed pull; doc fix |
| `src/git.rs` | `detect_default_branch`; checked `dirty_files`/`current_branch` |
| `src/repo_sync.rs` | Push error classification |
| `src/models/tasks.rs` | `slugify` cap |

## Verification

- [ ] Tests first for each fix (mock runner sequences; real git in temp dirs where the behaviour is git's own)
- [ ] `cargo test --no-fail-fast` and the task's verify command pass
