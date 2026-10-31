# Git SSH

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Keep bounded git calls non-interactive without overriding the user's own ssh command.

## Context

This work package addresses finding R1 from `docs/plans/2026-10-31-codebase-review.md`, a regression from the P1 fix (8aae4b80).

## Findings

### 🟡 Medium — R1: bounded git calls override the user's ssh command (`src/process.rs:194`)

**Issue:** `git_ssh_command` reads only `GIT_SSH_COMMAND` before appending `-o BatchMode=yes`, and `run_bounded` sets it on every bounded child (including the `sh -c` chain in `src/cli/statusline.rs`). Git's precedence is `GIT_SSH_COMMAND` > `GIT_SSH` > `core.sshCommand`, so setting the env var replaces the other two. An operator with `core.sshCommand = ssh -i ~/.ssh/work_key` now fails every bounded fetch, `ls-remote` and wrap-up pull (`finish.rs::pull_base`, `worktree.rs`). `docs/conventions.md` (~525) claims the existing command is kept.

**Fix:** See the decision. Either way, add a test that a repo with `core.sshCommand` set keeps it, and correct `docs/conventions.md`.

## Decisions

### R1: how to stop prompts without overriding ssh config (`src/process.rs::run_bounded`)

**Question:** Which mechanism keeps bounded git from prompting?

**Options:**
- (a) Drop `GIT_SSH_COMMAND`; keep `GIT_TERMINAL_PROMPT=0` and start the child in a new session (`setsid` via `pre_exec`, which also makes it a group leader for `abandon`). ssh cannot open `/dev/tty` and fails at once. Respects all user config; needs an `unsafe` `pre_exec` (check `docs/invariants.md` unsafe policy) or `std::os::unix::process::CommandExt::process_group` plus a separate `setsid`.
- (b) Resolve the effective command first (`GIT_SSH_COMMAND`, then `GIT_SSH`, then `git config core.sshCommand` in that repo) and append BatchMode to it. No `unsafe`; one extra git call per bounded network call, and `GIT_SSH` (a program, not a command line) needs special handling.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `src/process.rs` | Chosen mechanism in `run_bounded` / `git_ssh_command` |
| `docs/conventions.md` | Correct the `run_bounded` note |
| `src/process.rs` tests | `core.sshCommand` survives a bounded call |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass, no `skipping:` lines
- [ ] Manual: a repo with `core.sshCommand` set fetches through dispatch
