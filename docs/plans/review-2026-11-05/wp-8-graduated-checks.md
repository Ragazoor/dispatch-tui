# Graduated checks: close the mutation misses

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** The single-write checker and the move-path `epic_id` guard catch the cases this round's mutation tests found they miss, and plugin-script tests can no longer inherit an agent's environment.

## Context

Findings G6, G7 and G8 from `docs/plans/2026-11-05-codebase-review.md` §3.6, plus the C3 graduation in §5. Each fix adds a self-test (or test) case for the miss it closes, so the next review's mutation test has something to fail. The reviewer's probe files were in a scratchpad that is gone; rebuild probes from the descriptions below.

## Findings

### 🟡 Medium — G6: `check-single-write-service.sh` stops reading a file at its first `#[cfg(test)]` (`scripts/check-single-write-service.sh`, the awk rule `/^#\[cfg\(test\)\]/ { flush(); fn = ""; exit }`)

**Issue:** `src/service/mod.rs` has `#[cfg(test)] pub use …` at line 14, so nothing below it is checked; `src/service/api.rs` from line 641.

**Fix:** skip only the braced `mod tests { … }` block (track brace depth from a `#[cfg(test)]` followed by `mod`), not the rest of the file. Self-test: a probe with a mid-file `#[cfg(test)] pub use` followed by a two-write function must fail.

### 🟢 Low — G7: single-write checker misses not in its header

**Issue:** not followed: `let store = Arc::clone(&self.db);` rebinding. The comment stripper `sub(/\/\/.*/…)` removes everything after a `//` inside a string literal (`"http://…"`), including writes on that line. False positive: same-named helpers in two `impl` blocks of one file share a count. Already admitted in the header: helpers in another file, two levels deep, `Self::h(self)`, `join_all(...map(|t| self.db.w(t)))`, a `for` header wrapped before `{`, receivers on another field.

**Fix:** handle the first three (self-test cases for each); keep the header's list of accepted limits current.

### 🟢 Low — G8: move-path `epic_id` guard misses (`tests/memory_caller_conformance/move_paths.rs::epic_id_writers`)

**Issue:** caught now: one-line literals, `pub(super) async fn`, subdirectories. Missed: values starting with `Option`, `pub`, `i64` or `Patch` (`epic_id: Option::from(e)`, `epic_id: public_epic`) are filtered as declarations; `Self { epic_id, ..self }` in `impl Task`; `.epic_id.replace(`, `mem::replace(&mut t.epic_id, …)`, a `&mut t.epic_id` binding; a `"}"` in a string ends the span; `#[reducer] pub fn mv` on one line charges the previous function; a second write in a function already in the table is deduplicated away; twin-only move functions (`src/sync/memory_caller/` is not scanned).

**Fix:** judge a field write by what follows `epic_id:` (declarations are `epic_id: <Type>` inside a `struct`, so detect the struct span instead of a prefix list); scan `Self {` literals inside `impl Task`; treat `&mut …epic_id` and `.epic_id.replace(` as writes; strip string literals before brace counting; handle an attribute on the `fn` line; scan the twin. Unit tests for the scanner with each form.

### 🟢 Low — C3 graduation: one test helper for spawning plugin scripts (`src/setup/hooks/tests.rs::invoke_hook`, `tests/hook_scripts.rs`)

**Issue:** 9ee5cd16 added `DISPATCH_TASK_ID` to agent windows; `tests/hook_scripts.rs` strips it, `src/setup/hooks/tests.rs` did not, and `cargo test` failed inside every agent window (fixed in #98918 with `env_remove`). Two harnesses spawn the same scripts with different environment rules.

**Fix:** one shared helper that both use, which strips every `DISPATCH_*` variable (or starts from `env_clear()` with an explicit allow-list that keeps `PATH` and `HOME` — `env_clear()` alone breaks bash). A test that the helper's child sees no `DISPATCH_TASK_ID` even when the parent has one.

## Changes

| File | Change |
|------|--------|
| `scripts/check-single-write-service.sh` | `#[cfg(test)]` handling; G7 cases; header |
| its self-test | probe cases for each fix |
| `tests/memory_caller_conformance/move_paths.rs` | scanner fixes and unit tests |
| `src/setup/hooks/tests.rs`, `tests/hook_scripts.rs` (or a shared test util) | one spawn helper |

## Verification

- [ ] `./scripts/verify.sh` passes
- [ ] Mutation: re-add a two-write function below a `#[cfg(test)] pub use` in `src/service/mod.rs` — the checker fails
- [ ] Mutation: write `epic_id: Option::from(e)` in a module move path not in `MOVE_PATHS` — the guard fails
- [ ] Run the hook tests with `DISPATCH_TASK_ID=1` exported — they pass
