# Implementation plan: Phase 7 — `PollOwner` and host-scoped polling

**Design:** [docs/superpowers/specs/2026-09-21-poll-ownership-design.md](../superpowers/specs/2026-09-21-poll-ownership-design.md)
**Task:** #4865, epic #325
**Date:** 2026-09-21

Read the design doc first — this plan assumes it. Short recap: a new shared
`PollOwner` entity (task | epic scope, a host, a claim timestamp) gives
`PollPrStatus` and `FeedTick` a single owner each for the cases that have no
natural one today (a host-less review task, and every feed epic — `Epic` has
no `host` field at all). Claims are permanent; a human can explicitly
override one. No TTL, no automatic failover — settled with the user across
several rounds of design discussion, recorded in the design doc.

Spec, then tests, then code, per this repo's convention. `allium:tend` first.

## Spec changes (`allium:tend`)

1. **`core.allium`**: new `entity PollOwner` (scope kind, scope_id, host,
   claimed_at) with `UniquePollOwnerPerScope`. No new config value — no TTL
   to configure.
2. **`pr-workflow.allium`**: `PollPrStatus` gains the host/claim precondition
   from the design doc. New rule `ClaimPollOwner` (or fold the claim into
   `PollPrStatus`'s own `ensures` — tend's call) for the task-scope claim.
   Reuse across `pr-workflow.allium` and `feeds.allium`, so the claim/override
   rules likely belong in `core.allium` next to the entity, with
   `PollPrStatus`/`FeedTick` each stating their own precondition against it.
3. **`feeds.allium`**: `FeedTick` gains the same precondition, scoped to the
   epic. `EditEpic` gains the take-over prompt when `feed_command` changes
   and an existing `PollOwner` names a different host. `UpsertFeedTasks`
   (or wherever `upsert_feed_item`'s behaviour is specced) gains: a newly
   created feed task stamps `created_by` from the running host's owner
   identity.
4. **`mcp-task-tools.allium`** (or wherever the MCP tool catalogue is
   specced): new `override_poll_owner` tool — scope, scope_id, unconditional
   reassignment to the caller's host.

Run `allium:weed` after, since this touches four files and it is easy to
leave one inconsistent with the others.

## Tests before code, in the order the design doc lists them

1. `PollPrStatus`, task **with** worktree — unchanged behaviour, but pin it
   with a test now that a second concept (`PollOwner`) exists nearby: only
   `task.host` polls.
2. `PollPrStatus`, task **without** worktree — exactly one host claims and
   polls.
3. Claim permanence — ownership never moves across many ticks from a second,
   non-owning host.
4. `FeedTick` — exactly one host runs a given feed epic's command per
   interval.
5. Feed-created tasks stamp `created_by` on insert, untouched on update.
6. Manual override — reassigns an existing claim; behaves like a fresh claim
   when none exists.

## Work, by layer

### 1. SpacetimeDB module (`spacetime/module/src/lib.rs`)

- New `poll_owners` table: `scope` (an enum or a `bool is_epic`-style flag —
  tend decides the Rust shape during spec write), `scope_id: i64`, `host:
  String`, `claimed_at: Timestamp`. Index on `(scope, scope_id)`.
- `claim_poll_owner(scope, scope_id)` reducer: find-or-create by the pair,
  no-op if a row already exists (same shape as Phase 6c's epic
  find-or-create, Decision 2 in that plan — no unique index needed, reducers
  serialize).
- `override_poll_owner(scope, scope_id)` reducer: same find-or-create target,
  but overwrites `host`/`claimed_at` unconditionally when a row exists.
- `upsert_feed_item`: stamp `created_by` on the `None` (insert) branch from
  `require_identity(ctx)` — mirror the epic-creator reducers' existing
  pattern (Phase 6c, Decision 3) rather than inventing a new one.
- Regenerate bindings: `./scripts/regenerate-spacetime-bindings.sh`.
- Extend `src/spacetime/tests/module_schema.rs` /
  `src/spacetime/tests/bindings_parity.rs` coverage for the new table if
  those tests enumerate tables (check before assuming — they may already be
  generic).

### 2. Sync layer (`src/sync/`)

- `rows.rs`: `PollOwnerRow { scope, scope_id, host }` (no `claimed_at` needed
  client-side unless a test wants it) plus storage and an accessor —
  `poll_owner(scope, scope_id) -> Option<PollOwnerRow>`. This is the reader
  the Phase 6c comment predicted ("Phase 6 adds the reader together with the
  test that needs it") — same idea, later phase.
- `decode.rs`: decode the new bindings table.
- `sdk_connector.rs`: subscribe `SELECT * FROM poll_owners` unconditionally
  (like `hosts`) — every host needs to see every claim to know whether it's
  the owner. Wire `on_insert`/`on_update`/`on_delete` into `rows.upsert_poll_owner`/
  `remove_poll_owner`.
- `writes.rs`: `SharedWriter` methods `claim_poll_owner`/`override_poll_owner`
  calling the two reducers. Not a `TaskServiceApi`/`EpicServiceApi` method —
  this isn't a task/epic mutation, so the mutation-boundary seal doesn't
  apply; model it the way `register_host` was wired in Phase 6c (separate
  plumbing, own small surface) unless a closer-fitting existing seam turns up
  during implementation.

### 3. Host-scoping the two tick paths

- `src/tui/update/agent.rs::tick_pr_poll`: for a task with `host = None`,
  check `poll_owner(Task, task.id)`; if absent, claim (fire a command
  invoking `SharedWriter::claim_poll_owner`) and proceed optimistically this
  tick; if present and not local, skip; if present and local, proceed. Task
  `host = Some(h)` keeps the existing direct comparison — no `PollOwner`
  involved.
- `src/feed/mod.rs` (`FeedRunner::tick` / wherever the per-epic interval gate
  lives): same three-way check, scoped to the epic, before spawning the feed
  command.
- On a single-machine install (no shared writer attached): skip the
  `PollOwner` check entirely in both paths — always eligible, matching
  today's behaviour and `is_locally_owned`'s existing no-op-on-one-machine
  precedent.

### 4. Manual override surfaces

- MCP tool `override_poll_owner(scope, scope_id)` — new handler in
  `src/mcp/handlers/`, wired into `mcp_tools!` (`src/mcp/handlers/dispatch.rs`).
  Available to both scopes from day one, per user direction ("agents are
  first class users").
- TUI: `EditEpic`'s existing editor-session apply path
  (`src/tui/update/epics.rs` or wherever `EditorSession(epic)` is processed)
  gains a post-apply check: if `feed_command` changed and
  `poll_owner(Epic, epic.id)` names a different host, enter a new
  `InputMode::ConfirmOverridePollOwner { epic_id, other_host_label }`
  (pattern-matched on `ConfirmDetachTmux`'s "confirm a follow-up action after
  the primary one already completed" shape, not `ConfirmTrustRepo`'s
  "gates the primary action" shape — the edit itself is not gated on this
  answer). `y` calls the override; `n`/Esc leaves ownership alone.

### 5. `Task.created_by` on feed-created rows

Already covered by module change #1 — this is server-side, so no client
change beyond what the sync layer already does for `created_by` generally
(it already decodes/round-trips the field; only the module wasn't stamping
it for feed inserts).

## Verify

Run the repo's verify command (read from `get_task`) before declaring this
done — this touches the spacetime module, so include
`./scripts/check-spacetime-module.sh` if the verify command doesn't already.
Confirm `HEAD..main` before wrapping up, per this repo's standing
instruction: other Phase 7-adjacent work may have landed on `main` since this
session started.
