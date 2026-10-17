# Feeds, Scripts and CI

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Feed scripts report clean results correctly and never truncate, feed timeouts leave no orphans, and releases only ship green commits.

## Context

This work package addresses findings 21–23, 25, 39 and 42 of the 2026-10-17
code review (`docs/plans/2026-10-17-codebase-review.md`). Specs:
`docs/specs/feeds.allium`, `docs/specs/feed-scripts.allium`. Shipped feed
scripts are installed by `src/setup/plugins.rs`; check whether a script
change needs a provenance/manifest update.

## Findings

### 🟡 CVE feed truncated at 100 alerts under Reconcile (`scripts/fetch-cve.sh:58`)

**Issue:** `per_page=100` with no pagination; the epic syncs in Reconcile
mode, so alerts past 100 lose their tasks and worktrees. The script's own
NOTE admits it.

**Fix:** `gh api --paginate` (merge pages with `jq -s 'add'`), or fail /
signal degraded so the guard forces an additive sync.

### 🟡 Log-warnings feed fails on a clean log (`scripts/fetch-log-warnings.sh`)

**Issue:** Under `set -euo pipefail`, `grep -aE … | awk | jq` exits 1 when
grep finds nothing, with empty output. Every poll logs a non-zero exit.

**Fix:** `{ grep -aE … -- "$LOG_FILE" || [ $? -eq 1 ]; } | awk … | jq …`.

### 🟡 Feed timeout leaves orphaned children (`src/feed/exec.rs:93`)

**Issue:** `kill_on_drop` kills only `sh -c`; `gh | jq` children survive each
timeout. `.output()` buffers unbounded stdout. *Plausible.*

**Fix:** Spawn with `.process_group(0)`, kill the group on timeout; cap stdout.

### 🟡 Release builds do not wait for CI (`.github/workflows/release-please.yml`)

**Issue:** The build job runs on `release_created` and only checks
`dispatch --version`; nothing waits for ci.yml.

**Fix:** Gate on CI success for the SHA (`workflow_run`, or run
`cargo test --locked` in the release workflow).

### 🟢 Low findings

- `src/main.rs:490` (`cmd_verify_feed`) — no timeout (runtime uses
  `FEED_COMMAND_TIMEOUT`); 0 items is exit 1 although runtime treats a clean
  empty emission as valid. Use the timeout; warn and exit 0 on empty.
- `.github/workflows/ci.yml` — actions pinned by tag while release-please
  pins by SHA; `cargo install cargo-tarpaulin` unpinned. SHA-pin and
  `--locked --version`.

## Changes

| File | Change |
|------|--------|
| `scripts/fetch-cve.sh` | Pagination |
| `scripts/fetch-log-warnings.sh` | grep exit 1 is success |
| `src/feed/exec.rs` | Process group kill; stdout cap |
| `src/main.rs` | `cmd_verify_feed` timeout and empty handling |
| `.github/workflows/release-please.yml`, `ci.yml` | CI gate; pinning |

## Verification

- [ ] Tests first: `tests/feed_scripts.rs` case for an empty log; exec test where a grandchild outlives the timeout (assert it is gone)
- [ ] Gate scripts (`scripts/check-*.sh`) pass; `tests/ci_gates.rs` still agrees with the workflow
- [ ] `cargo test --no-fail-fast` and the task's verify command pass
