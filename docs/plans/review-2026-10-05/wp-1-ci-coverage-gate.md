# CI Coverage Gate and Conformance Job

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the coverage gate measure hand-written code and run the spacetime conformance tests in CI.

## Context

This work package addresses findings from the code review in `docs/plans/review-2026-10-05/report.md`.

## Findings

### ⚠️ Generated bindings drag coverage down (`.github/workflows/ci.yml:185`)

**Issue:** `src/spacetime/bindings/` (216 generated files, 2,777 lines) is about 7% covered and counted. Tarpaulin reports 76.79%; hand-written code is 88.87%.

**Fix:** Exclude it with `--exclude-files 'src/spacetime/bindings/*'`. Measure the new figure and set the floor deliberately (comment its calibration, as the existing comment does).

### ⚠️ Floor slack is 0.87 points (`.github/workflows/ci.yml:185`)

**Issue:** Floor 88 against 88.87% measured. One new 100-line untested file fails CI.

**Fix:** Recalibrate after the exclusion; keep a margin.

### ⚠️ Conformance tests skip in CI (`tests/spacetime_module.rs`, `tests/memory_caller_conformance.rs`)

**Issue:** They skip when `spacetime` is absent, which is the Coverage job. Drift between the module and its in-memory twin is only guarded where the CLI is installed.

**Fix:** Add a CI job that installs the `spacetime` CLI and runs both files. Keep the Coverage job without it (tarpaulin breaks `spacetime publish`).

## Changes

| File | Change |
|------|--------|
| `.github/workflows/ci.yml` | Add `--exclude-files`, new floor with comment, new conformance job |
| `docs/testing.md` | Document the exclusion and the conformance job; keep the tarpaulin/`PATH` caveat |

## Verification

- [ ] Run existing tests — all pass
- [ ] `cargo tarpaulin --engine llvm --exclude-files 'src/spacetime/bindings/*'` (with `spacetime` off `PATH`) reports about 88.9%
- [ ] `./scripts/check-doc-paths.sh` passes
