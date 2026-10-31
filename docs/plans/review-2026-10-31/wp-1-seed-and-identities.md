# Seed Authorisation and Client-Named Identities

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Stop any connected board from using the `seed_*` reducers, or identity arguments it names itself, to get around the workspace permission model.

## Context

Findings V1, V4 and V5 from `docs/plans/2026-10-31-codebase-review.md`. Spec first: `docs/specs/spacetime-seed.allium` and `docs/specs/workspaces.allium` change before code. Every module change is mirrored in `src/sync/memory_caller/` and covered in `tests/memory_caller_conformance/`. A reducer signature change bumps `RULES_VERSION` (`spacetime/module/src/rules.rs`) and `BOARD_RULES_VERSION` (`src/sync/rules_gate.rs`) together; rebuild `module.wasm` and bindings per CLAUDE.md.

## Findings

### 🔴 High — V1: the seed reducers check no caller (`spacetime/module/src/seed.rs`)

**Issue:** `seed_user_profiles`, `seed_memberships`, `seed_workspaces`, `seed_epics`, `seed_work_packages`, `seed_learnings`, `seed_tasks` and every other `seed_*` call neither `visibility.rs::is_operator`, `caller_identity` nor a super-admin check. Any board can call `seed_user_profiles([{identity: me, is_super_admin: true}])` and become super-admin, `seed_memberships` to admin any workspace, `seed_epics` to flip a private epic public or rewrite `created_by`, `seed_learnings` to pick `author`/`workspace_id`. The specs disagree: `spacetime-seed.allium` has `actor Operator { identified_by: core/Host where id != "" }` (any host); `workspaces.allium` names the publisher identity as the trust boundary. The restore path calls these via `spacetime call` as the CLI's logged-in identity (`src/spacetime/cli_store.rs::call`), so gating on today's `is_operator` (`sender == database identity`) may break restore.

**Fix:** settle the operator identity in both specs, gate every `seed_*` on it, add one refusal test per reducer, and keep a passing restore test.

### 🟡 Medium — V4: the module trusts a client-supplied `owner` and `created_by` (`spacetime/module/src/tasks_epics.rs::move_task_row`, `::set_task_epic`, `::update_task`, `::create_task`; `visibility.rs::settle_new_epic`, `::check_new_package`)

**Issue:** with `epic_id == 0` the task's `owner` comes from the caller's argument. Bob can detach a teammate's task with `owner = Bob` (takes it) or `owner = "x"` (nobody sees it again; `AdoptOwnerless` only adopts empty owners). The spec says an epic-less task lands on the caller's own board. `settle_new_epic` keeps a client `created_by`; `check_new_package` never stamps it, so authorship and visibility rights can be forged.

**Fix:** for non-operators stamp `owner = caller` on a detach or epic-less create, and always stamp `created_by = caller`. Update `WritesRequireSight` guidance. Mirror in `src/sync/memory_caller/tasks_epics.rs` and `visibility.rs`.

### 🟡 Medium — V5: follow and seen reducers trust a client-supplied `subscriber` (`spacetime/module/src/config.rs::subscribe_to_epic`, `::unsubscribe_from_epic`; `unseen.rs::mark_epic_seen`)

**Issue:** `subscriber` is never compared with the caller; `unsubscribe_from_epic` has no sight check. Alice can unsubscribe Bob (releasing his hosts' poll claims) or clear his unseen marks.

**Fix:** derive the subscriber from `caller_identity` (operator exempt); add a sight check to unsubscribe. Signature change → bump both rules versions; update host callers in `src/store/` and `src/service/`.

## Decisions

### Who the operator is (V1)

**Question:** which identity may seed/restore, and how does the module recognise it?

**Options:** (a) the owner identity recorded at `init` (the publisher) — restore must run as the publisher's CLI login; strongest, may need a restore doc change. (b) any super-admin — works with today's restore if the restorer is super-admin, but bootstrapping an empty store needs a carve-out (no super-admin yet). (c) the database's own identity only (`is_operator` as today) — breaks `spacetime call` restore unless restore is moved into a scheduled/init path.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

### Subscriber argument removal (V5)

**Question:** drop the `subscriber` argument (signature change, both versions bump) or keep it and refuse when it differs from the caller (no signature change)?

**Options:** dropping it is cleaner and removes the class; keeping it avoids regenerating bindings and touching host callers now.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/spacetime-seed.allium`, `docs/specs/workspaces.allium` | Settle the operator; stamp owner/created_by/subscriber from the caller |
| `spacetime/module/src/seed.rs` | Gate every `seed_*` |
| `spacetime/module/src/tasks_epics.rs`, `visibility.rs` | Stamp `owner`, `created_by` |
| `spacetime/module/src/config.rs`, `unseen.rs` | Subscriber from caller; sight check on unsubscribe |
| `src/sync/memory_caller/` | Mirror each rule |
| `tests/memory_caller_conformance/` | One case per rule |
| `spacetime/module-tests/` | Refusal test per seed reducer |
| `spacetime/module/src/rules.rs`, `src/sync/rules_gate.rs` | Bump together if any signature or rule changes |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass, no `skipping:` lines
- [ ] `spacetime/module-tests` pass; module checks from `docs/testing.md`
- [ ] A restore still works (`src/spacetime/restore.rs` tests)
- [ ] `cargo clippy --all-targets -- -D warnings` clean
