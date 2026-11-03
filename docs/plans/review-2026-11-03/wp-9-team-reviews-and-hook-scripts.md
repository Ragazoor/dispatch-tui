# Team Reviews and Hook Scripts

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Upgrading does not silently empty Team Reviews, a failed search does not delete tasks, and the peer-message hook runs on macOS's bash 3.2.

## Context

Findings U2 and U5 from `docs/plans/2026-11-03-codebase-review.md`. Specs: `docs/specs/feeds.allium` (Team Reviews, degraded-emission guards), `docs/specs/agent-health.allium` (`UndeliveredPeerMessage`). Shipped scripts live in `scripts/` and `plugin/`; edit those, never installed copies.

## Findings

### 🟡 Medium — U2: upgrading empties Team Reviews, and a failed search deletes tasks (`scripts/fetch-reviews.sh::search_prs`, `src/setup/plugins.rs::SHIPPED_SCRIPT_CONFIGS`)

**Issue:** 071ba533's script auto-updates, `teams.conf` ships `TEAMS=()`, and the repo-scoped `review-requested:@me` search is gone. The first cycle after upgrading emits no `team-request` items and reconcile deletes every Team Reviews task; nothing tells the user to edit `teams.conf`. Separately, `search_prs` prints `[]` and returns 0 when `gh search` fails (pre-existing), and each team adds a search against the 30/min pool, so a rate-limited cycle deletes and later re-creates tasks, re-firing their new-task marks.

**Fix:** when `TEAMS` is empty, print a one-line hint to stderr (reaches the feed log) and, if the spec allows, surface it once on the board. Make a failed `gh search` fail the whole emission (non-zero exit) so the degraded guards stop the reconcile. Test with a stub `gh` that fails.

### 🟡 Medium on macOS — U5: the peer-message hook aborts on bash 3.2 (`plugin/hooks/scripts/task-status-hook`)

**Issue:** `"${MSG_FLAGS[@]}"` with `MSG_FLAGS=()` under `set -u` is an "unbound variable" error on bash ≤ 4.3. `aarch64-apple-darwin` is a release target and macOS `/bin/bash` is 3.2, so every delivered `SendMessage` observation exits 1. (`fetch-reviews.sh` uses `local -n`, which also needs bash 4.3; check whether that script runs on macOS.)

**Fix:** `${MSG_FLAGS[@]+"${MSG_FLAGS[@]}"}`. Also pass `--body="$MSG_BODY"` so a body starting with `-` is not read as a flag. Consider a check that runs shipped hook scripts' empty-array expansions under `bash --posix` or shellcheck.

## Changes

| File | Change |
|------|--------|
| `scripts/fetch-reviews.sh` | Empty-teams hint; failed search fails the emission |
| `plugin/hooks/scripts/task-status-hook` | Safe empty-array expansion; `--body=` |
| `docs/specs/feeds.allium` | Empty-teams hint if surfaced |

## Verification

- [ ] New tests fail before the change and pass after
- [ ] `cargo test --no-fail-fast` green
- [ ] `shellcheck` on both scripts clean (if installed)
