# CLAUDE.md Updates

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make CLAUDE.md shorter and add the facts agents hit first.

## Context

This work package addresses findings from the code review in `docs/plans/review-2026-10-05/report.md`.

## Findings

### 💡 Live-data fact missing (`CLAUDE.md`)

**Issue:** Only `docs/reference.md` says live data is in SpacetimeDB and `tasks.db` is not authoritative.

**Fix:** Add one sentence to `CLAUDE.md`.

### 💡 Dense paragraphs (`CLAUDE.md`)

**Issue:** 19.8 KB. The "`main` moves while you work" block (about 35 lines) carries rationale and history.

**Fix:** Move it to `docs/` and keep a short pointer plus the exact command pair. Mind `check-doc-paths.sh` and `check-doc-symbols.sh`.

### 💡 Undocumented symlink and coverage caveats

**Fix:** Note that `AGENTS.md` is a symlink to `CLAUDE.md`. Put the tarpaulin/`spacetime`-on-`PATH` caveat next to the coverage command.

## Changes

| File | Change |
|------|--------|
| `CLAUDE.md` | Live-data sentence, symlink note, coverage caveat, shorter "main moves" block |
| `docs/testing.md` or `docs/conventions.md` | Receive the moved block |

## Verification

- [ ] Run existing tests — all pass
- [ ] `./scripts/check-doc-paths.sh`, `check-doc-symbols.sh`, `check-doc-headings.sh` pass
