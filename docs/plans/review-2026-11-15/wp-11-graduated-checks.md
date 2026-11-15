# Graduated checks: close the residues

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** the single-write checker, the plugin-script test helper and the Allium gate stop missing the cases found by mutation on 2026-11-15.

## Context

Findings G9 and G11 from `docs/plans/2026-11-15-codebase-review.md` §3.9. Each fix adds a self-test case for the miss it closes, and each is mutation-tested (re-introduce the miss, confirm the check fails).

## Findings

### 🟢 Low — G9: a one-line `#[cfg(test)] item;` hides the next item from the single-write checker (`scripts/check-single-write-service.sh`)

**Issue:** verified by mutation: `#[cfg(test)] use std::fmt as _probe;` followed by an `impl` whose function makes two `self.db` writes passes green. Static residues: `'{'`/`'}'` char literals counted as braces; multi-line and raw strings not blanked; `&*self.db` and `self.db.as_ref()` aliases; a helper in a second `impl` of the same type is uncharged.

**Fix:** end the skip at the attribute line's own `;` when the item is on that line; handle char literals and the two aliases; list what stays unhandled in the header. A self-test case per fix.

### 🟢 Low — G11: two looser checks (`tests/common/script_command.rs::plugin_script_command`; `scripts/check-allium.sh`)

**Issue:** the helper strips `DISPATCH_*` only; `GIT_*` from a hook still reach the scripts' `git` calls (the 11-05 review suggested `env_clear()` plus an allow-list). `check-allium.sh`'s `MIN_VERSION` is a floor, not a pin, and a stale baseline entry is only a note, so a warning that returns later is absorbed.

**Fix:** `env_clear()` keeping `PATH` and `HOME` (and whatever the scripts need, found by running the tests); pin the allium version (CLAUDE.md installs `3.6.1`); fail on stale baseline entries.

## Changes

| File | Change |
|------|--------|
| `scripts/check-single-write-service.sh` (+ self-test) | G9 |
| `tests/common/script_command.rs` | `env_clear()` with allow-list |
| `scripts/check-allium.sh`, its baseline | version pin; stale entries fail |

## Verification

- [ ] `./scripts/verify.sh` passes
- [ ] The G9 probe (one-line `#[cfg(test)]` before a two-write `impl` in `src/service/mod.rs`) now fails the checker; revert it after
