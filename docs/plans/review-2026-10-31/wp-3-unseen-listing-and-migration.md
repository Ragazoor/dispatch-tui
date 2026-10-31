# Unseen Listing and Migration

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make every unseen private epic reachable by a super-admin, and make the workspace migration backfill rows it skipped.

## Context

This work package addresses findings V5 and V6 from `docs/plans/2026-10-31-codebase-review.md`. V6 is a live-data question first: check the real store before writing code.

## Findings

### 🟡 Medium — V6: the v17 migration is skipped when a workspace exists (`spacetime/module/src/workspace.rs:343`)

**Issue:** `migrate_store_to_workspaces` returns early when any workspace exists, even when `previous == 16` (called from `lib.rs::stamp_rules_version_on_connect` for `previous < 17`). v16 shipped `create_workspace`, so a super-admin may have made one before v17. Then every epic and package stays at `workspace_id 0`, every write to them is refused as missing, and after the private flip nobody can read them. The store is already at `RULES_VERSION` 23, so if this happened it has already happened.

**Fix:** Step 1: query the live store (MCP `list_epics`, or `spacetime sql` as the publisher) for epics and packages with `workspace_id = 0`, and report what you find to the user. Step 2: when `previous < 17`, backfill rows with `workspace_id == 0` into "Default" (create it if absent) whether or not other workspaces exist; mirror in the twin (it calls the same function) and add a module test for "a workspace existed before the migration". Step 3: if live rows exist, repair them with the user's agreement (a one-off reducer or a re-run on the next bump).

### 🟡 Medium — V5: an orphaned private sub-epic under a private parent is never listed (`spacetime/module/src/visibility.rs:602`)

**Issue:** `unseen_items_of` excludes every private epic under a private ancestor (`under_private`), even when that ancestor is not itself listed. Y makes private S under X's public P; X makes P private; Y leaves the workspace. S is seen by nobody and not listed; X cannot delete P (`check_subtree_all_seen` refuses) and the workspace cannot be deleted. The spec contradicts itself: `core.allium`'s nearest-holder `private_holder` lets Y see S under X's private P, while D5 says a private epic hides its whole subtree.

**Fix:** After the decision, exclude S only when its nearest private ancestor is itself listed (its creator is not a member). Mirror and add a conformance case.

## Decisions

### V5: nearest holder vs whole-subtree hiding (`docs/specs/core.allium`, `docs/specs/workspaces.allium`)

**Question:** When a private epic sits under another person's private epic, who sees it?

**Options:**
- (a) Nearest holder (today's code): its creator sees it, the outer holder does not. Keep the code; fix D5's wording and the `under_private` filter.
- (b) Whole subtree (D5): the outer holder sees everything below, inner privacy is moot. Simpler listing; changes views and `private_holder`, and the inner creator loses sight of their own epic.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `spacetime/module/src/workspace.rs` | Backfill `workspace_id == 0` rows whenever `previous < 17` |
| `spacetime/module/src/visibility.rs` | `unseen_items_of` listing rule |
| `docs/specs/core.allium`, `docs/specs/workspaces.allium` | Resolve nearest-holder vs D5; migration wording |
| `src/sync/memory_caller/visibility.rs` | Mirror |
| `tests/memory_caller_conformance/`, `spacetime/module-tests/` | Stranded sub-epic listed; migration with a prior workspace |
| `rules.rs` / `rules_gate.rs`, `module.wasm`, bindings | Bump together; rebuild |

## Verification

- [ ] Live-store check done and reported before any code change
- [ ] `cargo test --no-fail-fast` — all pass, no `skipping:` lines
- [ ] Module tests pass
- [ ] After merge to main: `./scripts/rotate-previous-store.sh`
