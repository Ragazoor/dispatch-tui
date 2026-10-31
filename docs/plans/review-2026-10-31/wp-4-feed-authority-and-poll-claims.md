# Feed Authority and Poll Claims

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Keep feeds running across workspace moves and member removal, and stop members from rewriting or archiving what belongs to others.

## Context

Findings V6–V9 from `docs/plans/2026-10-31-codebase-review.md`. Spec first (`docs/specs/workspaces.allium`, `feeds.allium`, `sync.allium`, `learnings.allium`). Mirror every module change in `src/sync/memory_caller/` with a conformance case; bump both rules versions together on a rule change.

## Findings

### 🟡 Medium — V6: moving an epic or removing a member strands feed poll claims (`spacetime/module/src/visibility.rs::move_epic_to_workspace`; `workspace.rs::drop_follows_in_workspace`)

**Issue:** both delete subscriptions directly and never release `poll_owners` rows, unlike `config.rs::unsubscribe_from_epic`. The poll-owner host whose owner lost sight gets "does not exist" from `ingest_feed`; no other host claims the epic. The feed stalls until `OverrideFeedOwner`.

**Fix:** release epic-scoped poll claims held by hosts whose owner loses sight, in both paths (spec: `MoveEpicToWorkspace`, `RemoveMember`, `UnsubscribeFromEpic`).

### 🟡 Medium — V7: `archive_stale_learnings` has no gate and no sight check (`spacetime/module/src/learning.rs::archive_stale_learnings`)

**Issue:** any client passing `cutoff = "9999"` archives every approved, un-upvoted learning in every workspace; a normal board's cutoff also archives other workspaces' learnings. Breaks `WritesRequireSight`.

**Fix:** filter by `learning_scope_seen` for the caller, or make it operator-only / scheduled.

### 🟡 Medium — V8: any member can change the shell command another person's host runs (`spacetime/module/src/tasks_epics.rs::patch_epic` `feed_command`; `feed.rs::ensure_role_epic`)

**Issue:** the epic's poll-owner host runs `feed_command` (`src/feed/mod.rs`); anyone who sees the epic may set it.

**Fix:** only the epic's creator, the poll owner's owner, or a workspace admin may change `feed_command`.

### 🟡 Medium (plausible) — V9: one shared PR Reviews tree per workspace, signalled to everyone (`spacetime/module/src/feed.rs::ensure_role_epic`; `unseen.rs::mark_task_unseen_for_viewers` since 300ce9cb)

**Issue:** the managed root is found by `(feed_role, parent 0, visible)`, not by creator. After migration everyone is in Default, so each board reuses the first person's "PR Reviews" root and rewrites its `feed_command`; the poll owner's PRs fill "My Reviews", and every member is signalled. The spec covers only the cross-workspace case.

**Fix:** key managed roots by creator, or make them private to their creator.

## Decisions

### Who may change `feed_command` (V8)

**Question:** which roles may change an epic's feed command?

**Options:** (a) creator + workspace admin — simple; the poll-owner's owner cannot fix their own host's command on someone else's epic. (b) (a) plus the poll owner's owner — covers that, more checks. (c) leave open, but a host refuses to run a command not set by its own owner — host-side, no module change, but the feed silently stops.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

### Managed PR Reviews roots (V9)

**Question:** per-creator public roots or private roots?

**Options:** per-creator public roots keep teammates able to see each other's review queues but clutter the board with one tree per person; private roots keep the board clean but hide them. Either needs a migration for live stores that already share one root.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/workspaces.allium`, `feeds.allium`, `sync.allium`, `learnings.allium` | Rules for claims, feed command authority, managed roots, archive scope |
| `spacetime/module/src/visibility.rs`, `workspace.rs` | Release poll claims |
| `spacetime/module/src/learning.rs` | Scope or gate archive |
| `spacetime/module/src/tasks_epics.rs`, `feed.rs` | Feed command authority; managed root keying |
| `src/sync/memory_caller/` | Mirror |
| `tests/memory_caller_conformance/` | One case per rule |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass
- [ ] Conformance case: remove a member who owns a poll claim; another host can claim it
- [ ] `cargo clippy --all-targets -- -D warnings` clean
