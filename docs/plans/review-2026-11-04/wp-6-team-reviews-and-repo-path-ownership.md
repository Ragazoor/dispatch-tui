# Team Reviews hint and repo-path ownership

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Let PR Reviews reconcile again under the default config, and settle who owns and may remove a repo path.

## Context

Findings N1, N3 and N2 from `docs/plans/2026-11-04-codebase-review.md` §3.4; regressions in the fixes for U2 (7d45d991) and K2/F2 (942e92c0, dd8e17a1). All three are decision items. Spec first: `docs/specs/feeds.allium`, `core.allium`, `cli.allium`. Shipped feed scripts auto-update (`src/setup/plugins.rs::SHIPPED_SCRIPT_CONFIGS`).

## Findings

### 🟡 Medium — N1: PR Reviews never removes a stale card under the default config (`scripts/fetch-reviews.sh::search_team_prs`, `scripts/teams.conf`, `src/feed/exec.rs::degraded_partial_emission`)

**Issue:** the shipped `teams.conf` has `TEAMS=()`, and the new hint "TEAMS is empty…" goes to stderr on every run. Any stderr with items > 0 makes the cycle additive (`DegradedNonEmptyEmission`); with 0 items it fails. Every user without teams gets a PR Reviews epic whose merged, approved and no-longer-requested cards are never removed; a manual refresh reports "degraded". `tests/feed_scripts.rs::fetch_reviews_hints_on_stderr_when_teams_conf_lists_no_teams` asserts the stderr without asking how the runner classifies it.

**Fix:** see Decisions. Add a test that runs the default config's emission through the runner's classifier and asserts a reconciling (non-degraded) cycle.

### 🟡 Medium — N3: a path shared by two machines resolves for feeds on only one (`spacetime/module/src/config.rs::save_repo_path`, `src/feed/cycle.rs::sync_emission`) — plausible

**Issue:** `repo_paths` has one row per path, and the first saver's host keeps it. Laptop and desktop both have `~/Code/work/api`; the desktop saved first; when the laptop polls, `org/api` items get no repo path. `core.allium` records "stays with the host that saved it first".

**Fix:** see Decisions.

### 🟢 Low — N2: a TUI delete of another host's or a legacy repo path silently does nothing (`src/runtime/tasks.rs::exec_delete_repo_path` → `spacetime/module/src/config.rs::delete_repo_path`)

**Issue:** the overlay lists every host's paths; the module filters on the caller's host and returns Ok, so the row comes back on reload. With K1 a legacy row whose path exists nowhere can never be removed.

**Fix:** refuse with a reason ("owned by host X"); plus see Decisions.

## Decisions

### N1: where the empty-teams hint goes

**Question:** how should the user learn `teams.conf` is empty without degrading the feed?

**Options:**
- Show it in the TUI/setup (e.g. once, when the PR Reviews epic is created or on `dispatch setup`). No runner change; the script stays silent.
- A recognised notice prefix on stderr (e.g. `notice:`) that `degraded_partial_emission` ignores. Keeps the hint where it is; adds a convention to the feed contract (`feeds.allium`).
- Drop the hint. Cheapest; the user may never learn about `teams.conf`.

### N3: one repo-path row per path, or per (path, host)

**Question:** should two machines each own the same path?

**Options:**
- Key `repo_paths` by (path, host). Correct for multi-machine users; schema change, migration, bindings regeneration, a rules-version bump, and every reader must pick its host's row.
- Keep one row; let the feed fall back to any row whose path exists on this disk. Small; the verify command and base branches stay the first saver's.
- Keep as is (first saver wins) and document the limit.

### N2: who may remove an unowned row

**Question:** in the TUI, should another host's or a legacy unowned row be removable, or hidden from the delete action?

**Options:** let the TUI delete unowned rows (and refuse other hosts' rows with a reason); or show only this host's rows as deletable.

Ask the user with `AskUserQuestion` before implementing each of these findings. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/feeds.allium`, `core.allium`, `cli.allium` | decisions recorded |
| `scripts/fetch-reviews.sh`, `src/feed/exec.rs` | N1 |
| `spacetime/module/src/config.rs` + twin `src/sync/memory_caller/config.rs` | N2 refusal, N3 |
| `src/runtime/tasks.rs`, `src/feed/cycle.rs` | N2 message, N3 |
| `tests/feed_scripts.rs`, conformance tests | N1 classifier test, N2/N3 cases |

## Verification

- [ ] `cargo test --no-fail-fast` green; `cargo clippy --all-targets -- -D warnings` clean
- [ ] The N1 classifier test fails before the fix
- [ ] If the schema changes: module rebuilt, bindings regenerated, rules versions bumped together
