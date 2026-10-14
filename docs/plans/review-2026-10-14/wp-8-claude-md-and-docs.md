# CLAUDE.md and docs

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** `CLAUDE.md` tells an agent the thin-TUI rule, points at `store-split.allium`, states one correct timing figure, and says the gate scripts can fail on committed `main`.

## Context

This work package addresses findings from the 2026-10-14 code review
(`docs/plans/review-2026-10-14/report.md`, §7, §8 M9). `CLAUDE.md` is loaded
into every agent's context; keep it slim and put detail in `docs/`.

## Findings

### 💡 Timing is stale (`CLAUDE.md:35`)

**Issue:** "The lib target runs in ~4s once built (measured 3.5s, 5582
tests)" — measured 2026-10-14: 29.9 s for 5,917 tests on a loaded machine;
`tests/spacetime_module.rs` alone is 58 s; a full `--no-fail-fast` run with
the live store is ~3.5 minutes. Knowledge-base entry #428 says 15–20 s.

**Fix:** State one measured figure with its conditions (idle machine, warm
cache, `spacetime` on `PATH`), or drop the number and keep the advice (run
in the foreground; pass an explicit `timeout` on the first run). Update
learning #428 to match.

### 💡 The thin-TUI split is not in the file (`CLAUDE.md:75`, `CLAUDE.md:144`)

**Issue:** Nothing says that data rules live in `spacetime/module/src/`,
that a rule change is mirrored in `src/sync/memory_caller/` and covered in
`tests/memory_caller_conformance.rs`, that `RULES_VERSION`
(`spacetime/module/src/rules.rs`) and `BOARD_RULES_VERSION`
(`src/sync/rules_gate.rs`) move together, or that
`docs/specs/store-split.allium` (the largest spec) exists. The spec list at
L144 omits it.

**Fix:** One paragraph under "Store Layering" with those four facts and a
pointer to the spec; add `store-split` to the spec list.

### 💡 Gate scripts can fail on committed `main` (`CLAUDE.md:49`)

**Issue:** The hook runs at push time; the file tells agents to run
`cargo fmt` before committing but not the `check-*.sh` scripts. Two of them
failed on `main` on 2026-10-14 (`check-no-test-sleep.sh`,
`check-rustdoc-links.sh`), from the two most recent commits.

**Fix:** Add one sentence: run the gate scripts (the list already in that
paragraph) before committing, since a wrap-up rebase lands on `main`
without pushing.

### 💡 Coverage job runs the live store (`CLAUDE.md:30-33`, `docs/testing.md:180-184`)

**Issue:** The coverage paragraph does not say the job now runs
`tests/spacetime_module.rs` and `tests/memory_caller_conformance.rs` under
tarpaulin, which is what timed it out on 2026-10-09, nor that the module
crate's lines must be excluded (work package 1 makes that change).

**Fix:** One sentence in `CLAUDE.md`'s local-coverage paragraph pointing at
the updated `docs/testing.md` section.

### 💡 Inline-test guidance (`docs/testing.md:117-129`)

**Fix:** Add a row/line: an inline `#[cfg(test)] mod` over ~200 lines moves
to a `tests/` submodule.

## Changes

| File | Change |
|------|--------|
| `CLAUDE.md` | L35 timing; L49 gate scripts before commit; L75 thin-TUI paragraph; L144 add `store-split`; coverage sentence |
| `docs/testing.md` | Inline-test size rule; coverage section cross-check with WP1's edits |
| Knowledge base | `record_learning`/update #428 with the measured figure and conditions |

## Verification

- [ ] `./scripts/check-doc-paths.sh`, `./scripts/check-doc-symbols.sh`, `./scripts/check-doc-headings.sh` green
- [ ] `CLAUDE.md` stays under ~150 lines
- [ ] `cargo test --no-fail-fast` green (docs-only, but the verify command requires it)
