# Memory double shares the module's rules

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** One implementation per rule: the in-memory test double calls the module's pure validators and ingest bodies instead of re-deriving them, and the native copies of rules that moved to the module are deleted.

## Context

This work package addresses findings from the 2026-10-14 code review
(`docs/plans/review-2026-10-14/report.md`, §2 A2/A3, §8 H1–H4).
`src/sync/memory_caller/` (4,723 lines) re-implements the SpacetimeDB module
(4,885 code lines) and reuses only eight helpers from it. The `allium:weed`
pass found the first real drift. `docs/specs/spacetime-memory-store.allium`
(`ReducerConformance`) already says the double should reuse the module's
ctx-free helpers; this package applies that to the rules that were added
natively since.

Spec first: if a rule's wording in `docs/specs/store-split.allium` or
`spacetime-seed.allium` has to change to make the shared function the single
source, change the spec, then the test, then the code.

## Findings

### ⚠️ The double accepts statuses the module refuses (`src/sync/memory_caller/tasks_epics.rs:20-37`, `:58-64`; `spacetime/module/src/tasks_epics.rs:21`, `:551`, `:582`; `spacetime/module/src/support.rs:65`)

**Issue:** The module calls `validate_status` in `write_task` and on both
epic writes (`spacetime-seed.allium: StoreRefusesUnknownStatus`). The
double's `write_task`/`write_epic` never check. `validate_status` is
`pub(crate)`, so the double cannot reuse it. `TaskPatch.status` is an
`Option<String>` in the bindings, so a test can patch to a status the module
would refuse and the double silently stores it.

**Fix:** Make `validate_status` `pub`, call it from the double's
`write_task`/`write_epic`, and add a conformance test in
`tests/memory_caller_conformance.rs` that patches a task and an epic to an
unknown status and expects the same refusal from both.

### ⚠️ Duplicate feed-ingest bodies (`src/sync/memory_caller/feed.rs:493` and `:344`; `spacetime/module/src/feed_ingest.rs:231` and `:97`)

**Issue:** `ingest_role_routed` is 128 lines in the double and 105 in the
module; `ingest_grouped` is 93 and 69. They are the two longest functions in
the board crate and the longest in the module, maintained twice, held
together by seven conformance tests.

**Fix:** In the module, lift the decision logic of both into ctx-free `pub`
functions over plain row structs (input: the existing tasks/epics the
routing needs; output: the list of inserts/patches/retirements to apply).
The reducer applies that list through `ctx`; the double applies it to its
`Vec`s. Keep behaviour identical — the existing conformance tests and
`src/feed/ingest/tests.rs` must pass unchanged.

### ⚠️ `expire_*` outcome differs between double and module (`src/sync/memory_caller/dispatch_requests.rs:288-293`, `src/sync/memory_caller/session_requests.rs:424-429`; `spacetime/module/src/dispatch_request.rs:701`, `spacetime/module/src/session_request.rs:1033-1038`)

**Issue:** On a row that has already finished, the module's
`expire_dispatch_request`/`expire_session_request` return `Ok(())`; the
double returns `Refused`. No client observes it today because the module's
version is scheduled, but it is the one rule where the two disagree.

**Fix:** Return `Ok` from the double on an already-finished row; pin with a
conformance test that expires a finished row through both.

### ⚠️ Native rule copies with no production caller (`src/models/tasks.rs:272`, `src/models/tasks.rs:549`)

**Issue:** `SubStatus::is_valid_for` has 31 test references and no
production caller — the live rule is the module's `validate_sub_status`,
which the double already reuses. `Task::is_wrappable` is documented as the
answer "every gate reads", and no production gate reads it; the module's
`validate_wrap_up_request` (`spacetime/module/src/session_request.rs:216`)
decides. Both are untested promises that the board and the module agree.

**Fix:** Delete both. Re-point their tests at the module's functions through
the double (the `is_valid_for` table tests become `validate_sub_status`
cases; the `is_wrappable` cases become `request_wrap_up` refusal cases). If a
TUI surface needs a local "is this affordance shown" answer, derive it from
`Task::wrap_up_block` (`src/models/tasks.rs:533`) and say so in
`docs/specs/tasks.allium`.

## Changes

| File | Change |
|------|--------|
| `spacetime/module/src/support.rs` | `validate_status` → `pub` |
| `spacetime/module/src/feed_ingest.rs` | Extract ctx-free `pub` decision functions for `ingest_role_routed` and `ingest_grouped`; reducers apply their output |
| `spacetime/module/src/dispatch_request.rs`, `session_request.rs` | No behaviour change; export any helper the double needs |
| `src/sync/memory_caller/tasks_epics.rs` | Call `validate_status` in `write_task`/`write_epic` |
| `src/sync/memory_caller/feed.rs` | Replace the native bodies with calls to the module's decision functions |
| `src/sync/memory_caller/dispatch_requests.rs`, `session_requests.rs` | `expire_*` returns `Ok` on a finished row |
| `src/models/tasks.rs` | Delete `SubStatus::is_valid_for` and `Task::is_wrappable`; move their tests |
| `tests/memory_caller_conformance.rs` | Three new conformance cases (unknown status, finished-row expiry, one ingest route) |
| `docs/specs/spacetime-memory-store.allium` | List the newly shared helpers under `ReducerConformance` |
| `scripts/build-managed-module.sh` | Run after the module change; commit the rebuilt `module.wasm` |

## Verification

- [ ] `cargo test --no-fail-fast` green (`spacetime` and `tmux` on `PATH`)
- [ ] `cd spacetime/module && cargo test` green; `bash ./scripts/check-spacetime-module.sh` green
- [ ] `tests/memory_caller_conformance.rs` has the three new cases and they pass against the live store
- [ ] `grep -rn 'is_valid_for\|is_wrappable' src spacetime` returns nothing
