# Shared Git Helper

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Give git invocations one run-and-check helper in `src/git.rs`, as `src/tmux.rs` already has for tmux.

## Context

This work package addresses finding S1 from `docs/plans/review-2026-10-08-followup/report.md`. WP8 (agent-tree consolidation) depends on this one; finish it first.

## Findings

### 💡 Git plumbing has no shared helper (`src/cli/agent_tree.rs:581`, `:594`; `src/dispatch/worktree.rs`; `src/dispatch/finish.rs`; `src/repo_sync.rs`)

**Issue:** `run_git` and `git_error` live in `src/cli/agent_tree.rs` (L581, L594); `src/cli/agent_diff.rs` imports them from there. `src/dispatch/worktree.rs` (8 sites), `src/repo_sync.rs` (6) and `src/dispatch/finish.rs` (5) each hand-build `run_with_timeout("git", &["-C", …])` and check the exit status manually. Near-verbatim duplicates: `worktree prune` at `worktree.rs:627` and `:871`; `rebase --abort` / `merge --abort` at `finish.rs:202` and `repo_sync.rs:320`. `src/tmux.rs` already has the pattern (`run_checked`, `run_checked_stdout`).

**Fix:** Move `run_git` and `git_error` into `src/git.rs` (adding a `-C <dir>` / timeout variant if the dispatch sites need it). Route the 19 hand-built sites through them. Keep each site's existing error wording where tests or the Allium specs depend on it. Keep the existing `ProcessRunner` injection so mock tests still see the same argv.

## Changes

| File | Change |
|------|--------|
| `src/git.rs` | Add `run_git`, `git_error` (and a timeout/`-C` variant if needed) |
| `src/cli/agent_tree.rs`, `src/cli/agent_diff.rs` | Import from `crate::git` |
| `src/dispatch/worktree.rs` | Route 8 sites through the helper; dedupe `worktree prune` |
| `src/dispatch/finish.rs` | Route 5 sites |
| `src/repo_sync.rs` | Route 6 sites; share the `--abort` helper with `finish.rs` |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass, no `recorded_calls()` argv assertion changed
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `grep -rn 'run_with_timeout("git"' src` lists only intentional exceptions
