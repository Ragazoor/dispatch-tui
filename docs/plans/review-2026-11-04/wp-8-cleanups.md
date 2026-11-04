# Cleanups

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix the small findings from the 2026-11-04 review.

## Context

Findings M11, M13, M14, N4, N5, N7, N8, T4, T5 and T6 from `docs/plans/2026-11-04-codebase-review.md`. Behaviour changes start in the spec (`docs/specs/*.allium`), then tests, then code. Run `allium check` on any spec you edit.

## Findings

### 🟢 Low — M11: spec and TUI still offer a detach that always fails (`docs/specs/tasks.allium::MoveTaskToEpic`, `src/tui/update/move_task.rs`, `workspaces.allium::ReParentingKeepsTheHolder`)

**Issue:** caafe9ea (task #98852) made "only the operator detaches a task from its epic" the rule. `MoveTaskToEpic` still says a null epic detaches, and the picker still offers "— no parent —", which is always refused with "a task stays in its workspace". `ReParentingKeepsTheHolder`'s `else: owner` arm is unreachable for a task in an epic.

**Fix:** see Decisions; then update `MoveTaskToEpic`, hide the sentinel for a task in an epic, drop the unreachable arm.

### 🟢 Low — M13: the publisher claim runs only on a module change (`src/spacetime/managed_store.rs`)

**Issue:** startup publishes, and so claims, only when `module_action` returns `Publish`; a failed claim is a `warn` and the hash is recorded anyway. `spacetime-seed.allium` says the next start fixes it.

**Fix:** run `claim_store_owner` on every managed bring-up (it is idempotent and treats "already recorded" as success), or reword the spec.

### 🟢 Low — M14: doc slips in 94a6acdc

**Fix:** in `src/tui/update/feeds.rs`, move `handle_feed_not_authors_host`'s doc comment so `handle_feed_already_refreshing` keeps its own; in `feeds.allium::FeedCommandRunsOnSettersHost`, cite `decide_feed_poll_action`.

### 🟢 Low — N4: `setsid` children can pop a GUI askpass (`src/process.rs::non_interactive_env`, `src/feed/exec.rs::run_feed_command_bounded`) — plausible

**Issue:** without a tty and with `DISPLAY`/`WAYLAND_DISPLAY` set, OpenSSH uses askpass; with a helper installed, a bounded fetch with a passphrase key pops a dialog until the deadline kills it.

**Fix:** set `SSH_ASKPASS_REQUIRE=never` in `non_interactive_env` (child-only env; does not touch the user's ssh config) and for the feed command. Assert it in the env test.

### 🟢 Low — N5: an allow-listed partial write (`src/service/managed_feeds.rs::write_managed_feed_settings`)

**Issue:** allow-listed in `scripts/check-single-write-service.sh` as "four independent settings rows", but its own comment says the command and interval are coupled; a failure after the first write leaves them inconsistent.

**Fix:** see Decisions.

### 🟢 Low — N7: `build.jobs` detection misses the inline form (`src/dispatch/build_jobs.rs::cargo_config_sets_jobs`)

**Fix:** parse the config as TOML so `build = { jobs = 2 }` counts; add a test.

### 🟢 Low — N8: the first scope probe runs past a short deadline (`src/dispatch/request_host.rs::DeadlineRunner::agent_scope`, `::start_sccache_server`)

**Issue:** with a little time left, the first `prepare_agent_scope` (up to 3 × 10 s) runs on the unbounded runner; the sccache start is bounded at 5 s, not the time left.

**Fix:** bound both by the remaining deadline.

### 🟢 Low — T4: unplaced epics share a sort scale with placed ones (`src/tui/types/layout.rs::EpicTree::build`, `FlattenedGroupKeys::key_for`)

**Issue:** an epic on a parent cycle is never placed and falls back to `sort_key()`, which collides with placed ranks and interleaves groups. Needs corrupt data.

**Fix:** after the walk, rank unplaced epics after every placed one. Test with a self-parented epic.

### 🟢 Low — T5: scrolling up can hide a nested group's root header (`src/agent_tree/render/agents.rs::render_agents`)

**Fix:** walk back over consecutive header lines above the cursor to set `top_line`, keeping "cursor wins in a too-short pane"; update the spec's "the line above it" wording; snapshot test.

### 🟢 Low — T6: `handle_pane_view` is quadratic in epics (`src/mcp/handlers/hooks.rs::handle_pane_view`)

**Fix:** add `models::epics::epic_path_with(target, &by_id)` and use the `by_id` map built a few lines above.

## Decisions

### M11: confirm the detach rule

**Question:** caafe9ea made "only the operator may detach a task from its epic". Is that the intent?

**Options:** keep it (align spec and TUI as above); or allow the task's creator or an admin to detach too (a module rule change, a conformance case and a rules-version bump).

### N5: `write_managed_feed_settings`

**Question:** make it one reducer, or accept the risk?

**Options:** one `set_managed_feed_settings` reducer (module, twin, bindings, conformance case, rules bump); or keep four writes and reword the allow-list reason to state the risk honestly.

Ask the user with `AskUserQuestion` before implementing each of these findings. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/tasks.allium`, `workspaces.allium`, `feeds.allium`, `spacetime-seed.allium` | M11, M13, M14 |
| `src/tui/update/move_task.rs`, `src/tui/update/feeds.rs` | M11, M14 |
| `src/spacetime/managed_store.rs` | M13 |
| `src/process.rs`, `src/feed/exec.rs` | N4 |
| `src/service/managed_feeds.rs`, `scripts/check-single-write-service.sh` | N5 |
| `src/dispatch/build_jobs.rs`, `src/dispatch/request_host.rs` | N7, N8 |
| `src/tui/types/layout.rs`, `src/agent_tree/render/agents.rs` | T4, T5 |
| `src/models/epics.rs`, `src/mcp/handlers/hooks.rs` | T6 |

## Verification

- [ ] `cargo test --no-fail-fast` green; `cargo clippy --all-targets -- -D warnings` clean
- [ ] `allium check` on every edited spec
