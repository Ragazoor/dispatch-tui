# Gate scripts and CI

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the pre-push hook and the CI coverage job pass on `main` again, without weakening either gate.

## Context

This work package addresses findings from the 2026-10-14 code review
(`docs/plans/review-2026-10-14/report.md`, §1, §3 T1, §8 B1–B3). Six days of
commits have landed with no hook, no CI and no coverage number. Two hook
scripts fail on today's `main`, and the coverage job both times out and
(when it completes) scores under the floor.

Do not "fix" the sleep by rewriting the timer with `sleep_until` or
`timeout` — that evades the grep without resolving the convention. The
convention's premise ("production has no legitimate use") is what changed.

## Findings

### 🚨 Pre-push hook rejects a legitimate production timer (`src/runtime/event_loop.rs:56`, `scripts/check-no-test-sleep.sh:10`)

**Issue:** `next_loop_event` awaits `tokio::time::sleep(wait)` for the
frame-wait deadline (commit `6e48cfed`, 2026-10-13). Check 1 of
`check-no-test-sleep.sh` bans the call anywhere under `src/` on the stated
premise that production never needs it (`scripts/check-no-test-sleep.sh:10-11`,
`docs/conventions.md:647`, `docs/testing.md:161`). The hook therefore fails
on `main`.

**Fix:** Scope check 1 to test code using the same region rule check 2
already uses (test files plus top-level inline `#[cfg(test)] mod` blocks).
Keep it unconditional there — no `allow-test-sleep` marker for
`tokio::time::sleep` in tests. Update the script header, `docs/conventions.md`
("No `tokio::time::sleep` in tests": production use is now legitimate for
timers; name `next_loop_event` as the one site), `docs/testing.md:161`, and
extend `scripts/test-check-no-test-sleep.sh` with two cases: a production
`tokio::time::sleep(` passes; one inside an inline `#[cfg(test)] mod` fails.

### 🚨 Coverage job cannot pass (`.github/workflows/ci.yml:226-254`, `docs/testing.md:180-184`, `tests/common/spacetime_instance.rs`)

**Issue:** Run 37892931546 (2026-10-09) died with `cargo_tarpaulin: Failed to
run tests: Error: Timed out waiting for test response` inside
`tests/spacetime_module.rs` — the live-store tests now run under tarpaulin
(task #40968) and the job passes no `--timeout`. Run locally to completion
on 2026-10-14, the same command scores **85.74%** (21,698 / 25,307) against
`--fail-under 86`: the module crate became a `test-support` path dependency,
so tarpaulin instruments `spacetime/module/src/*` natively — 2,449 lines at
25.93%, seven files at 0% (`feed.rs`, `feed_ingest.rs`, `agent_state.rs`,
`seed.rs`, `grouping.rs`, `sequence.rs`, `lib.rs`) because the reducers run
as wasm inside the live store. The board code alone is 92.15%
(21,063 / 22,858).

**Fix:** Add `--exclude-files 'spacetime/module/*'` next to the bindings
exclusion, and `--timeout 600` (raise if the live targets still trip it).
Rewrite the floor comment (`ci.yml:231-250`) and the paragraph at
`docs/testing.md:180-184` to say both exclusions and why; re-measure and
record the figure and date beside the floor. Extend
`tests/ci_gates.rs::ci_coverage_job_enforces_a_coverage_floor` to assert the
module exclusion is present. If the timeout persists on CI, run
`tests/spacetime_module.rs` and `tests/memory_caller_conformance.rs` outside
tarpaulin and merge the reports — say which you did in the comment.

### 🚨 `main` unpushed; CI red on `origin` since 2026-09-10 (`.github/workflows/ci.yml`)

**Issue:** `origin/main` is 78 commits behind local `main` (last push
2026-10-08). Seven consecutive CI runs on `origin/main` failed. The 2026-10-09
Clippy failure (`nonminimal_bool` at `src/tui/update/agent.rs:514` and
`src/tui/update/lifecycle.rs:219`) is already fixed in local `main`.

**Fix:** Not a code change. Once the two items above are green locally and
the full pre-push hook passes, tell the operator `main` is ready to push and
that one green CI run on `origin/main` closes this finding. Do not push
`main` yourself — pushing `main` needs the operator's confirmation.

## Changes

| File | Change |
|------|--------|
| `scripts/check-no-test-sleep.sh` | Scope check 1 (`tokio::time::sleep(`) to test regions; update the header comment |
| `scripts/test-check-no-test-sleep.sh` | Two new cases: production sleep passes, inline-`cfg(test)` sleep fails |
| `docs/conventions.md` | "No `tokio::time::sleep` in tests": production timers are legitimate; name the one site |
| `docs/testing.md` | L161 (sleep rule) and L180-184 (coverage: both exclusions, timeout, new figure) |
| `.github/workflows/ci.yml` | `--exclude-files 'spacetime/module/*'`, `--timeout 600`, rewritten floor comment with the 2026-10-14 figures |
| `tests/ci_gates.rs` | Assert the module exclusion and the timeout are present |

## Verification

- [ ] `./scripts/check-no-test-sleep.sh` exits 0 on `main`; `bash ./scripts/test-check-no-test-sleep.sh` exits 0
- [ ] `./scripts/check-rustdoc-links.sh` exits 0 (the `Task::display_name` link was fixed by the review commit)
- [ ] Every step of `.githooks/pre-push` passes when run by hand
- [ ] `cargo tarpaulin --engine llvm --out stdout --exclude-files 'src/spacetime/bindings/*' --exclude-files 'spacetime/module/*' --timeout 600` finishes and scores ≥ 86; quote the figure in the floor comment
- [ ] `cargo test --no-fail-fast` green
