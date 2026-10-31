# Seed Authorisation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Stop any connected board from using the `seed_*` reducers, or free identity arguments, to bypass the workspace permission model.

## Context

This work package addresses findings V1 and V7 from `docs/plans/2026-10-31-codebase-review.md`. Spec first: the decision below changes `docs/specs/spacetime-seed.allium` and `docs/specs/workspaces.allium` before any code.

## Findings

### 🔴 High — V1: the seed reducers have no authorisation (`spacetime/module/src/seed.rs:438`, `:459`, `:474`)

**Issue:** `seed_user_profiles`, `seed_workspaces`, `seed_memberships`, and also `seed_epics`, `seed_work_packages`, `seed_learnings`, `seed_subscriptions` (and every other `seed_*`) check neither `visibility.rs::is_operator` nor the caller. Any board identity can call `seed_user_profiles([{identity: me, is_super_admin: true}])` to become a super-admin, `seed_memberships` to become admin of any workspace, or `seed_epics` to flip someone's private epic to public or rewrite `created_by`. The spec allows it: `spacetime-seed.allium` declares `actor Operator { identified_by: core/Host where id != "" }` (any host), while `workspaces.allium` (~74-81, ~136-138) names the publisher identity as the trust boundary.

Restore calls these through `spacetime call` with the CLI's logged-in identity (`src/spacetime/cli_store.rs::call`, reducer table ~421). That is the publisher, not the database's own identity. Today's `is_operator` is `ctx.sender() == ctx.database_identity()`, which a CLI call does not satisfy, so gating on it as-is would break restore. Its doc comment ("or the publisher") is also wrong.

**Fix:** After the decision below: record in `spacetime-seed.allium` who `Operator` is, gate every `seed_*` on that, fix `is_operator`'s doc comment (or its body, if the decision makes the publisher an operator), and add one refusal test per seed reducer in `spacetime/module-tests/`. Check that `dispatch` restore still works end to end (the startup smoke test and `tests/spacetime_module.rs` restore cases).

### 🟢 Low — V7: some writes take the identity from the client (`spacetime/module/src/config.rs::subscribe_to_epic`, `spacetime/module/src/unseen.rs::mark_epic_seen`, `spacetime/module/src/visibility.rs::settle_new_epic`, `::check_new_package`)

**Issue:** `subscribe_to_epic` and `mark_epic_seen` take a free `subscriber`; a caller can make a non-member follow an epic or clear another person's unseen marks. `settle_new_epic` keeps a client `created_by` on the operator path and `check_new_package` never stamps `created_by`, so a caller can plant a private sub-epic attributed to Y, which then blocks `move_epic_to_workspace` for everyone but the operator.

**Fix:** Use `caller_identity(ctx)` for these (operator exempt). Mirror each in `src/sync/memory_caller/`, add conformance cases, and bump `RULES_VERSION` and `BOARD_RULES_VERSION` together.

## Decisions

### V1: who may restore, and how the module knows it (`spacetime/module/src/seed.rs`, `docs/specs/spacetime-seed.allium`)

**Question:** Which identity may call the `seed_*` reducers?

**Options:**
- (a) The publisher (owner) identity, recorded by the module at `init` (or read from the host's owner) and recognised as an operator. Restore keeps working with the CLI's login; boards never qualify. Costs a stored owner identity and a migration for existing stores (stamp it on first connect by the publisher).
- (b) Any super-admin. Simple to check with existing data, but a store with no super-admin yet cannot be restored, and super-admins are made by `claim_super_admin`, which widens who can restore.
- (c) Only the database identity (today's `is_operator`). Safest, but the CLI cannot restore at all until restore runs inside the module.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/spacetime-seed.allium` | Tighten `actor Operator` to the chosen identity |
| `docs/specs/workspaces.allium` | Align the trust-boundary text with it |
| `spacetime/module/src/seed.rs` | Gate every `seed_*` reducer |
| `spacetime/module/src/visibility.rs` | Fix `is_operator` doc (or body); stamp `created_by` in `settle_new_epic` / `check_new_package` |
| `spacetime/module/src/config.rs`, `unseen.rs` | Use `caller_identity` for subscriber |
| `src/sync/memory_caller/` | Mirror the V7 changes |
| `spacetime/module/src/rules.rs`, `src/sync/rules_gate.rs` | Bump together |
| `spacetime/module-tests/`, `tests/memory_caller_conformance/` | Refusal tests and conformance cases |
| `spacetime/module/module.wasm`, bindings | Rebuild with `./scripts/build-managed-module.sh`, regenerate bindings |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass, no `skipping:` lines
- [ ] Module tests in `spacetime/module-tests/` pass (see `docs/testing.md`)
- [ ] A board identity calling each `seed_*` is refused; restore via the CLI still succeeds
- [ ] After merge to main: `./scripts/rotate-previous-store.sh`
