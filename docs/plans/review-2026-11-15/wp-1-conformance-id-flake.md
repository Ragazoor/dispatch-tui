# Conformance tests: stop comparing generated ids across stores

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** `cargo test` stops failing at random because the real store's auto-inc ids jump while the in-memory twin's do not, and a check keeps the class from coming back.

## Context

Finding S1 from `docs/plans/2026-11-15-codebase-review.md` §3.1. `main`'s verify command failed on `d00aab72` with `memory_caller_conformance::schedules::memory_caller_keeps_schedules_like_the_real_module` — `left: TaskId(4096)`, `right: TaskId(2)` — and passed 4 of 4 runs alone. e6253a0a fixed the same class for work-package ids on 2026-11-11 ("The real store's auto-inc ids can jump (e.g. to 4096) mid-run") and left every other id kind. Every agent's verify can go red for a reason it did not cause, so do this package first.

## Findings

### 🟡 Medium — S1: conformance tests assert the real and twin stores hand out the same generated id (`tests/memory_caller_conformance/schedules.rs` `create` closure; same assertion in `dispatch_requests.rs`, `close_and_managed_epics.rs`, `pr_state.rs`, `session_requests.rs`; `scenario.rs` epic/task/learning ids; `learning_placement.rs`)

**Issue:** `assert_eq!(r, m, "both sides hand out the same task id")` and the `generated … id` assertions; later steps then use one id for both sides (`same_task(id)` reads `rows_real.task(id)` and `rows_mem.task(id)`). When the real store's sequence jumps, the assertion fails, or a later lookup on one side finds nothing.

**Fix:** follow e6253a0a's shape: keep a per-side id (a small `Pair { real, mem }` or a map from the real id to the twin id), compare row *shapes* (without ids, or with ids translated through the map), and check that a returned id is the newest row on its own side. Do it in every file listed. Then graduate: a shared creation helper in `tests/memory_caller_conformance/` that returns both ids, and a grep check (in `scripts/` run by `gates.sh`, with a self-test) that rejects `assert_eq!` between a real and a twin *generated* id (`"same … id"`, `"generated … id"`). Mutation-test the check: re-add one such assertion and confirm it fails.

## Changes

| File | Change |
|------|--------|
| `tests/memory_caller_conformance/{schedules,dispatch_requests,close_and_managed_epics,pr_state,session_requests,scenario,learning_placement}.rs` | per-side ids; compare shapes or translate ids |
| `tests/memory_caller_conformance/` (shared module) | creation helper returning both ids |
| `scripts/check-*.sh` (new) + `scripts/gates.sh` | reject cross-store generated-id equality; self-test |
| `docs/testing.md` | one line: conformance tests never compare generated ids across stores |

## Verification

- [ ] `./scripts/verify.sh` passes
- [ ] `cargo test --test memory_caller_conformance` passes three runs in a row under the full suite's load
- [ ] The new check fails when one removed assertion is re-added
