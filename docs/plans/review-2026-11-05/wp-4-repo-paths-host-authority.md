# Repo paths: host authority and upgrade

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Only a host's own board may write or delete its repo-path rows and verify commands, existing stores reach the per-(path, host) model, and verify-command reads pick this host's row.

## Context

Findings A3, A4 and A7 from `docs/plans/2026-11-05-codebase-review.md` §3.2, all residues of 8376c7e9 (repo paths per (path, host), N3). Behaviour changes start in `docs/specs/core.allium: SavedRepoPath` (and `cli.allium` for `repo set-verify`), then tests, then code. Mirror module changes in `src/sync/memory_caller/` with conformance cases; bump both rules versions together.

## Findings

### 🟡 Medium — A3: the repo-path reducers trust the client's `host` (`spacetime/module/src/config.rs::save_repo_path`, `::adopt_repo_path`, `::delete_repo_path`, `::set_verify_command`)

**Issue:** none calls `require_acting_for`, though `save_setting` and `set_managed_feed_settings` do. Any store user calls `delete_repo_path(p, "alice-host")`: Alice's row and its verify command are gone and her feeds stop resolving `p`. Rows can be forged under her host, and any user can change any repository's verify command, which every agent of that repository runs.

**Fix:** `require_acting_for(ctx, &host)` in all four; give `set_verify_command` a `host` argument. Same in the twin. Conformance case: a call naming another person's host is refused. Decide in the spec who may set a verify command shared across rows of one repository (the row's host owner is the obvious answer).

### 🟡 Medium — A4: N3 has no upgrade path for existing rows (`config.rs::save_repo_path`; `src/cli/commands.rs::set_verify`)

**Issue:** on an upgraded store the second machine of a shared path still has no row, so its feeds stay unresolved until it saves through the TUI. `set_verify` skips the save when `list_repo_paths()` already has the path (its comment "saving would re-stamp it" is stale). Legacy rows keep `github_path = ""`; when a later save links two rows, both keep their old, differing `verify_command`, against `SavedRepoPath` ("every row of one repository holds the same value").

**Fix:** `set_verify` saves when `list_repo_paths_saved_by_this_host` lacks the path. For the rest, see Decisions.

### 🟢 Low — A7: verify-command reads and writes pick a row by path alone (`config.rs::set_verify_command`; `src/sync/rows.rs::verify_command`) — plausible

**Issue:** `find(|r| r.path == path)` and "first row by path, any host". Two machines holding different repositories at the same path read or set each other's command; module iteration order differs from the twin's BTreeMap, so the stores can diverge.

**Fix:** select this host's row; take the sibling set from that row's `github_path`.

## Decisions

### A4: bringing existing rows onto the new model

**Question:** how do legacy rows get a `github_path` and a second host's row, and which verify command wins when two rows are linked?

**Options:** (a) the board re-saves its own existing paths once at startup (stamps `github_path`, creates the second host's row), and a save that links siblings copies the most recently set command — automatic, one write per path per start; (b) do nothing automatic; the TUI shows "unlinked" paths and the user re-saves — no hidden writes, more friction; (c) a one-time module backfill keyed on rules version — cannot run `git remote` in the module, so it can only unify commands, not stamp `github_path`.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/core.allium`, `cli.allium` | authority on repo-path reducers; upgrade rule |
| `spacetime/module/src/config.rs` | `require_acting_for`; `host` on `set_verify_command`; row selection |
| `src/sync/memory_caller/` | mirror |
| `src/sync/writes.rs`, `src/sync/rows.rs`, `src/store/` | pass host; read this host's row |
| `src/cli/commands.rs` | `set_verify` saves a missing own row |
| `tests/memory_caller_conformance/` | foreign-host refusal; row selection |
| `spacetime/module/src/rules.rs`, `src/sync/rules_gate.rs` | bump together; regenerate bindings |

## Verification

- [ ] `./scripts/verify.sh` passes
- [ ] Conformance cases fail before the fix
