# Git ssh Config

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Keep bounded git calls non-interactive without overriding the user's own ssh configuration.

## Context

Findings R1 and R5 from `docs/plans/2026-10-31-codebase-review.md`. R1 is a regression of the P1 fix (8aae4b80). Record the final rule in `docs/conventions.md` (near `run_bounded`).

## Findings

### 🟡 Medium — R1: `GIT_SSH_COMMAND` overrides `core.sshCommand` (`src/process.rs::run_bounded`, `::git_ssh_command`)

**Issue:** every bounded child gets `GIT_SSH_COMMAND` (plain `ssh -o BatchMode=yes` when unset). Git gives it priority over `core.sshCommand` and `GIT_SSH`. A user with `core.sshCommand "ssh -i ~/.ssh/id_work -o IdentitiesOnly=yes"` now fails the wrap-up `pull` (`src/dispatch/finish.rs`) and provisioning `fetch` (`src/dispatch/worktree.rs`) with "Permission denied (publickey)".

**Fix:** set `GIT_SSH_COMMAND` only when the caller already set it (append BatchMode). Otherwise keep the user's config — see Decisions. Add a test with `core.sshCommand` set (use `src/test_git.rs` helpers).

### 🟢 Low (plausible) — R5: the feed runner lacks the non-interactive environment (`src/feed/exec.rs::run_feed_command_bounded`)

**Issue:** it also uses `process_group(0)` but sets no `GIT_TERMINAL_PROMPT=0` or ssh batch mode, so a feed command that prompts is stopped by SIGTTIN and reported as a timeout.

**Fix:** share the environment setup with `run_bounded` once R1's rule is settled.

## Decisions

### How to keep ssh non-interactive (R1)

**Question:** which mechanism, when `GIT_SSH_COMMAND` is not set?

**Options:** (a) resolve `core.sshCommand` per repo (`git config --get`) and pass `-c core.sshCommand="<value> -o BatchMode=yes"` — exact, costs one extra git call per bounded child and only applies to git children; (b) rely on `GIT_TERMINAL_PROMPT=0` plus `SSH_ASKPASS_REQUIRE=never` and the null stdin — no override at all, but ssh may still read `/dev/tty` for a passphrase in some setups; (c) set nothing for ssh and accept the timeout as the backstop.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `src/process.rs` | Stop overriding user ssh config; shared env helper |
| `src/feed/exec.rs` | Use the shared env helper |
| `docs/conventions.md` | Record the rule |
| `src/process.rs` tests | `core.sshCommand` respected |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass
- [ ] New test: a repo with `core.sshCommand` set — the child sees no `GIT_SSH_COMMAND` override
- [ ] `cargo clippy --all-targets -- -D warnings` clean
