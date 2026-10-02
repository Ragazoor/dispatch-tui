# Docs Accuracy

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove stale text from CLAUDE.md and docs, and make CLAUDE.md short and imperative.

## Context

Findings from `docs/plans/review-2026-10-02/report.md`, section 6 and section 1.

## Findings

### 💡 Stale "Store seam" paragraph (`CLAUDE.md`, `docs/conventions.md`)

**Issue:** CLAUDE.md describes `SharedDomainStore`/`LocalStore`. Neither trait exists; `src/db/mod.rs:1004` says the seam is gone (task #4916). Re-check "Layout-cache coherence" and "board's read source" too.

**Fix:** Rewrite or delete those paragraphs and the "The store seam" section in `docs/conventions.md`. Check specs for implied two-backend text.

### 💡 CLAUDE.md too dense (`CLAUDE.md`)

**Issue:** ~32 KB; paragraphs of 700-1700 characters; changelog prose and task numbers ("no longer unchecked", "See task #4909").

**Fix:** Move tarpaulin/spacetime caveats, CI job description and spacetime module details to `docs/testing.md`. Rewrite the rest as short bullets with no task numbers. Add a short Testing pointer and a one-line specs index.

## Changes

| File | Change |
|------|--------|
| `CLAUDE.md` | Trim, fix stale seam text, add Testing and Specs pointers |
| `docs/conventions.md` | Fix "The store seam" section |
| `docs/testing.md` | Receive moved caveats |

## Verification

- [ ] `./scripts/check-doc-paths.sh`, `./scripts/check-doc-symbols.sh`, `./scripts/check-doc-headings.sh` pass
- [ ] `cargo test` passes (some tests read CLAUDE.md)
