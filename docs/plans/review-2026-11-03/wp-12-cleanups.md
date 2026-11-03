# Cleanups

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix the small findings from the 2026-11-03 review and bring stale spec text in line with the code.

## Context

Findings U6, K3, K4 and §3.7's stale-text items from `docs/plans/2026-11-03-codebase-review.md`. Behaviour changes start in the spec (`docs/specs/*.allium`), then tests, then code. Run `allium check` on any spec you edit.

## Findings

### 🟢 Low — U6: Space cannot remove a hide on a hidden-by-parent row (`src/tui/update/board_filter.rs::handle_toggle_hidden_epic`)

**Issue:** since 7f0227b4 the code branches on the row mark, and `[-]` (partly hidden) wins over the row's own hide, so Space always takes the insert branch. `ToggleHiddenEpic` says it removes when `epic in hidden_epics` (removing also clears its sub-epics' entries).

**Fix:** branch on `hidden.contains(&id)` first, then on the mark; add a test with a hidden child under a hidden parent.

### 🟢 Low — K3: bounded git still prompts in the default ssh setup, and the feed lacks BatchMode (`src/process.rs::ssh_config_args`, `::configured_ssh_command`; `src/feed/exec.rs::run_feed_command_bounded`)

**Issue:** fix for R1 (7db4940b) → with neither `GIT_SSH_COMMAND` nor `core.sshCommand` set, git runs ssh without BatchMode; a passphrase key hits SIGTTIN and reads as a timeout (P1 again). The feed gets BatchMode only when `GIT_SSH_COMMAND` is set (R5). Each bounded git call also spawns a `git config --get` with its own 10 s timeout outside the caller's deadline.

**Fix:** start bounded children (and the feed command) with `setsid` instead of `process_group(0)`: they keep their own group for the kill but have no controlling tty, so ssh fails at once and no user config is touched. Then drop or cache the `git config --get` lookup. Test with a fake `ssh` that tries to read `/dev/tty`.

### 🟢 Low — K4: a refused workspace create still moves the admin cursor (`src/tui/update/admin.rs::select_workspace_named`)

**Issue:** a duplicate-name create is refused, but the next snapshot highlights the existing workspace with that name.

**Fix:** clear the pending name on refusal; render or update test.

### 🟢 Low — stale spec text (§3.7)

**Issue and fix:**
- `docs/specs/sync.allium`, `SubscribeOnceIdentityIsSettled` guidance (~546–553): says usage comes through a view (5edab153 removed it) and that subscriptions are public with "known gap #98806" (2e681e34 fixed it). Rewrite. The same claims in the doc comments on `src/sync/rows.rs::Rows` (also "learnings subscribed unconditionally"): rewrite.
- `docs/specs/workspaces.allium`: ~1247 asks the question b5c9818a answered; ~1251 says packages carry no `created_by` (fb7b2c05 stamps them); ~986 "an epic with no creator is stamped" (all are now); ~1245 says moves onto a user board drop links (they keep them). Correct each. If the "Task Moves and Patch Rules" package changes the detach rule, leave ~1247 to it.
- Remove `src/service/epics.rs::require_epic_accepting_work` (no callers since 34aaba57) and its re-export.

## Changes

| File | Change |
|------|--------|
| `src/tui/update/board_filter.rs` | U6 |
| `src/process.rs`, `src/feed/exec.rs` | K3 `setsid` |
| `src/tui/update/admin.rs` | K4 |
| `docs/specs/sync.allium`, `docs/specs/workspaces.allium`, `src/sync/rows.rs` | Stale text |
| `src/service/epics.rs`, `src/service/mod.rs` | Remove dead function |

## Verification

- [ ] New tests fail before the change and pass after
- [ ] `allium check` on edited specs
- [ ] `cargo test --no-fail-fast` green
- [ ] `cargo clippy --all-targets -- -D warnings` clean; `./scripts/check-doc-symbols.sh` clean
