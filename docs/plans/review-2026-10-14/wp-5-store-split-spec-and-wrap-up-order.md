# Store-split spec, wrap-up order, stale comments

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** `docs/specs/store-split.allium` describes the M7 state the code is in, the M7 leftovers are gone, `handle_wrap_up` checks in the spec's order, and the comments that describe SQLite or pre-X1 request kinds as live are corrected.

## Context

This work package addresses findings from the 2026-10-14 code review
(`docs/plans/review-2026-10-14/report.md`, §2 "Specification alignment",
§8 H7, M1, M8), produced by an `allium:weed` pass over `store-split.allium`.
Spec first: edit the spec with `allium:tend`, then the tests, then the code;
finish with `allium:weed` on the same file.

## Findings

### ⚠️ Spec header and D11 are stale (`docs/specs/store-split.allium:7-9`, `:281-283`, `:497`)

**Issue:** The header says M7 "is landing (task #53260)" and line 497 says
"LANDING"; `RULES_VERSION = 6` (`spacetime/module/src/rules.rs:42`) and
`BOARD_RULES_VERSION = 6` (`src/sync/rules_gate.rs:21`) are the post-M7
values and `close_task`, `ensure_managed_epics` and the `patch_task`
transition rules are all present. D11 says the poll-ownership and settings
reducers "do not check the host yet"; they do
(`spacetime/module/src/agent_state.rs:158`, `:174`;
`spacetime/module/src/config.rs:119`, `:135`), and `claim_backlog_task` /
`release_backlog_claim` no longer exist anywhere.

**Fix:** Record M7 as landed with its task id; rewrite D11 to describe the
current host checks and point at the "Host-scoped writes" section.

### ⚠️ Enforced rule missing from the spec (`docs/specs/store-split.allium:955-997`; `spacetime/module/src/dispatch_request.rs:636-640`; `src/sync/memory_caller/dispatch_requests.rs:215-219`)

**Issue:** `report_dispatch_provisioned` refuses an empty worktree in the
module and the double; `HostReportsDispatched` states no such requirement.

**Fix:** Add `requires: worktree != ""` with the refusal wording to
`HostReportsDispatched`.

### ⚠️ M7 leftovers with only test callers (`spacetime/module/src/feed.rs:272`, `:415`, `:431`; `spacetime/module/src/tasks_epics.rs:838`)

**Issue:** `upsert_feed_tasks`, `create_repo_group_sub_epic`,
`create_managed_role_epic` (8 parameters) and `recalculate_epic_status` are
reducers nothing in `src/` calls; they are not in X5's delete list.

**Fix:** Delete them and their tests, remove their callers in
`src/sync/memory_caller/feed.rs` (`apply_create_managed_role_epic`) and
`src/sync/sdk_connector/callers.rs` if present, regenerate the bindings
(`./scripts/regenerate-spacetime-bindings.sh`), rebuild the module
(`./scripts/build-managed-module.sh`), and list them in X5. If one is kept
on purpose, say why in X5 instead.

### 💡 W2 check order (`src/mcp/handlers/tasks/wrap_up.rs:194-211`; `docs/specs/store-split.allium:348-352`)

**Issue:** W2 says: wrappable → worktree names a branch (rebase) → committed
→ verify passes. `handle_wrap_up` runs `check_wrap_up_ready` (commit check
plus a verify run of up to 900 s) first and the branch check last, so a
rebase whose worktree names no branch is refused only after a full verify
run.

**Fix:** Move the branch check above `check_wrap_up_ready`; add a test in
`src/mcp/handlers/tests/tasks/wrap_up.rs` that a branchless rebase request
is refused without the verify command running (assert on
`recorded_calls()` being empty for the verify binary).

### 💡 Comments that describe SQLite or pre-X1 request kinds as live

- `src/models/epics.rs:14` — "the two decoders — SQLite's and the shared store's"
- `src/service/tasks/crud.rs:483` — "Left to the SQLite foreign key"
- `src/sync/rows.rs:21` — "the retired SQLite store's … order" (keep the ordering fact, drop the SQLite framing)
- `spacetime/module/src/dispatch_request.rs:37-38` — "M5 adds wrap-up and exit-session requests as further values"
- `spacetime/module/src/session_request.rs:25`, `:64-74` — describe only `wrap_up` and `exit_session`; X1 added `close`

**Fix:** Rewrite each to describe the current behaviour.

## Changes

| File | Change |
|------|--------|
| `docs/specs/store-split.allium` | Header: M7 landed; D11 rewritten; `HostReportsDispatched` worktree requirement; X5 lists the deleted leftovers; W2 unchanged |
| `spacetime/module/src/feed.rs`, `tasks_epics.rs`, `tests.rs` | Delete the four leftovers and their tests |
| `src/sync/memory_caller/feed.rs`, `src/sync/sdk_connector/callers.rs`, `src/spacetime/bindings/` | Remove callers; regenerate bindings |
| `spacetime/module/module.wasm` | Rebuilt |
| `src/mcp/handlers/tasks/wrap_up.rs` | Branch check before `check_wrap_up_ready` |
| `src/mcp/handlers/tests/tasks/wrap_up.rs` | New ordering test |
| `src/models/epics.rs`, `src/service/tasks/crud.rs`, `src/sync/rows.rs`, `spacetime/module/src/dispatch_request.rs`, `spacetime/module/src/session_request.rs` | Comment fixes |

## Verification

- [ ] `allium check docs/specs/store-split.allium` passes; an `allium:weed` pass on it reports no divergence for the items above
- [ ] `cargo test --no-fail-fast` green; `cd spacetime/module && cargo test` green; `bash ./scripts/check-spacetime-module.sh` green
- [ ] `grep -n 'LANDING\|is landing' docs/specs/store-split.allium` returns nothing
- [ ] `./scripts/check-doc-symbols.sh` green
