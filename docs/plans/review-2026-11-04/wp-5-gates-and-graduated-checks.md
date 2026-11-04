# Gates and graduated checks

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make every commit that reaches `main` pass the same gate the pre-push hook runs, and close the remaining holes in the graduated checks.

## Context

Findings C2, G5, G3 and G4 from `docs/plans/2026-11-04-codebase-review.md` §3.1 and §3.5. C1 (clippy failing on `main`) was fixed in task #98864; C2 is why it happened. A graduated check must be mutation-tested: re-introduce the bug it targets and confirm it fails.

## Findings

### 🟡 Medium — C2: wrap-up rebases put commits on `main` that no lint gate has seen (repo verify command, `.githooks/pre-push`)

**Issue:** the pre-push hook runs `cargo fmt`, clippy and every `scripts/check-*.sh`, but a rebase-mode wrap-up lands commits on `main` without a push. The verify command is `cargo fmt --check && cargo test && ./scripts/check-doc-paths.sh`: no clippy, no other checks. Second occurrence: 2026-10-14 (`check-no-test-sleep.sh`, `check-rustdoc-links.sh` failed on committed `main`) and 2026-11-04 (clippy). CLAUDE.md's prose rule did not hold.

**Fix:** add `scripts/verify.sh` that runs what the hook runs, in check mode (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, every `check-*.sh` that takes no argument — note `check-commit-msg.sh` needs a file) plus `cargo test --no-fail-fast`. Make `.githooks/pre-push` call the shared part so the two cannot drift. Set it as the repo's verify command with `set_verify_command` — **ask the user before changing the verify command**, since every agent runs it. Replace the CLAUDE.md prose rule ("Run the hook's `check-*.sh` scripts before committing…") with a pointer to the script. Keep the script runnable without rustup (`check-toolchain.sh` fails on a distro toolchain; decide whether verify skips or fails it, and say so).

### 🟡 Medium — G5: the two demanded tests exist only in part

**Issue:** the loss-of-sight test exists twice, separately (`spacetime/module-tests/tests/feed_authority.rs`, `src/sync/memory_caller/tests/feed_authority.rs`), not as one conformance test, though `workspaces.allium: ClaimsFollowSight` says "one conformance test runs each of them". The ignored-column test for every table behind a view does not exist: `src/sync/tests/view_updates.rs` covers tasks, epics, agents and poll owners only.

**Fix:** move the loss-of-sight case into `tests/memory_caller_conformance/` so module and twin run the same steps (or correct the spec wording); extend `pins_view_updates!` (or add a table-driven test) that, for every view-backed table, sends an update changing only a column the decoder drops and asserts the row survives — learnings' `embedding` included. Mutation-test: revert `rows.rs::retire_agent` to a decoded compare and confirm the new test fails.

### 🟢 Low — G3: the move-path guard still has blind spots (`tests/memory_caller_conformance/move_paths.rs::epic_id_writers`)

**Issue:** this run's mutation test passed (removing `clear_unseen_for_task` from `move_task_row` fails the guard), but it misses: a one-line `Task { epic_id, ..t }` literal, a `pub(super) fn`/`async fn` (attributed to the previous function), module files in subdirectories (`read_dir` is not recursive), and a site added to an existing table row without being exercised.

**Fix:** match `epic_id` anywhere in a `Task {…}` span; accept any visibility/`async` prefix; recurse; map each `MOVE_PATHS` entry to exactly one site. Mutation-test each new case.

### 🟢 Low — G4: `check-single-write-service.sh` still misses loops, helpers and rebinding

**Issue:** writes in a loop count once; a helper that writes, called twice, is not flagged; `let store = &self.db;` rebinding is not seen; a `pub(in …) fn` header is not recognised; only `src/service/` is scanned.

**Fix:** count a write inside a `for`/`while`/`loop` body as many; follow `self.<helper>()` one level; recognise every `fn` header form; or state the limits in the script header and accept them. Add probe fixtures that the script must fail on.

## Changes

| File | Change |
|------|--------|
| `scripts/verify.sh` (new), `.githooks/pre-push` | C2 shared gate |
| repo verify command (via `set_verify_command`, after asking) | C2 |
| `CLAUDE.md`, `docs/testing.md` | C2 pointer replaces prose rule |
| `tests/memory_caller_conformance/` | G5 loss-of-sight, G3 guard |
| `src/sync/tests/view_updates.rs` | G5 dropped-column test |
| `docs/specs/workspaces.allium` | G5 wording if needed |
| `scripts/check-single-write-service.sh` | G4 |

## Verification

- [ ] `scripts/verify.sh` green on the branch; fails when a clippy warning is introduced (mutation)
- [ ] `cargo test --no-fail-fast` green; `cargo clippy --all-targets -- -D warnings` clean
- [ ] Each graduated check fails on its re-introduced bug
