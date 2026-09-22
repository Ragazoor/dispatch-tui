# Design: `PollOwner` — a claim-based ownership model for recurring background polling

**Task:** #4865, epic #325
**Date:** 2026-09-21
**Supersedes:** the host-registry-election sketch in the migration plan's
Phase 7 section and in `2026-09-13-distributed-dispatch-design.md`'s
"PollPrStatus is a second writer" section.

## Problem

Two `Tick()`-driven rules write shared state without any host filter:

- `PollPrStatus` (`pr-workflow.allium`) polls GitHub for every `review` task
  with a PR url. On a shared board every teammate's board does this
  independently. The merge/close state converges, but
  `consecutive_permanent_failures`, `consecutive_transient_failures` and
  `next_poll_at` are per-attempt counters two hosts polling at their own
  cadence will race, and `PrPollGaveUp`'s one-time `UserInformed` can fire on
  several machines for the same give-up.
- `FeedTick` (`feeds.allium`) spawns a feed epic's command and upserts the
  result. `Epic` has no `host` field at all — there is no natural single
  owner the way a dispatched task's worktree gives one — so every host
  subscribed to a feed epic runs its command independently, every interval.

`Task.host` already answers "who owns this?" for a task with a worktree
(`core.allium: HostTracksWorktree`), and `Task::is_locally_owned` already
gates worktree/tmux operations on it. Neither helps here: an `Epic` never has
a host, and a host-less review task (see below) never gets one either. Reusing
`is_locally_owned` directly would be actively wrong — it treats "no host" as
"owned by everyone," which is the right default for worktree ops (safe
because of `DispatchClaimExclusive`) and the wrong default for polling (must
be exactly one host).

**Where a host-less review task comes from:** a feed epic upserting a task
straight into `status = review` with a PR url (an "open PRs" style feed).
`upsert_feed_item`'s new-task branch never touches `worktree`/`host`, so the
row has no host from the moment it is created.

## Rejected approach: implicit election over the ambient `Hosts` registry

My first pass proposed computing a deterministic winner (e.g. lowest
`Host.id`) over whichever hosts happen to be in the shared `Hosts` registry at
tick time. Rejected per user feedback: ownership would be tied to whichever
humans' TUIs happen to be running and to the current membership of a registry
that already exists for a different purpose (identity), rather than to an
explicit, inspectable assignment that is "modelled as such." It also has no
failover story — if the elected host goes offline, every other host
recomputes the same answer and still defers to it.

## Design: `PollOwner`, a permanent claim

A new shared entity, owned by `core.allium` (alongside `Host`, which it
depends on):

```
entity PollOwner {
    scope: PollScopeKind    -- task | epic
    scope_id: Integer       -- the Task or Epic id, per `scope`. No foreign
                             -- key, matching how TaskWatcher's watcher/target
                             -- ids are stored (core.allium/task-watchers.allium)
                             -- rather than referencing Task directly.
    host: Host
    claimed_at: Timestamp

    -- At most one owner per (scope, scope_id).
    invariant UniquePollOwnerPerScope { ... }
}
```

**No expiry, no renewal, no TTL.** Per user direction: this is a system of
collaborators, not adversarial or unreliable infrastructure, and a watchdog
lease is overkill for it. A claim is a one-time, permanent assignment: the
first host to see an eligible scope with no `PollOwner` row claims it, and it
keeps polling that scope for as long as the row exists. `claimed_at` is kept
purely as an audit timestamp (mirroring `Task.created_at`) — nothing reads it
to decide anything.

**Claim, evaluated once per qualifying scope per tick, before the rule that
consumes it:**

```
if no PollOwner exists for (scope, scope_id):
    PollOwner.created(scope: scope, scope_id: scope_id, host: local_host(), claimed_at: now)
-- else: a row already exists. If it names this host, this host polls. If it
-- names another host, this host does nothing for this scope, forever, unless
-- the row is removed (see "Known limitation" below).
```

This is the same idiom `DispatchClaimExclusive` already uses for worktree
claims: a single conditional write, safe by construction because SpacetimeDB
reducers execute one at a time — there is no concurrent second call to race
a check-then-act sequence against (see Phase 6c's Decision 2, which made the
identical observation for epic find-or-create).

**Consumers:**

- `PollPrStatus` gains: `task.host != null and task.host = local_host()`
  **or** `task.host = null and owns_poll_claim(task: task)`.
- `FeedTick` gains: `owns_poll_claim(epic: epic)`.

A task **with** a worktree keeps using `task.host` directly — cheaper, and
already race-free — the claim only covers the two cases that have no natural
owner: host-less review tasks and every feed epic.

## Storage and the reducer

`PollOwner` is genuinely shared, comparable state — it must live in the
SpacetimeDB module as a new table, with a `claim_poll_owner(scope, scope_id)`
reducer implementing the conditional insert above: find-or-create by
`(scope, scope_id)`, exactly like Phase 6c's epic find-or-create (Decision 2)
— look up by the domain key, insert only if absent, otherwise a no-op. No
uniqueness index is needed for the same reason: reducers execute one at a
time, so there is no concurrent second call to interleave with the
check-then-insert.

Per the established pattern (`#809`: a reducer returns no value), the TUI
side predicts its own outcome from the already-subscribed view before
deciding whether to *act* as owner this tick: if the subscribed view shows no
`PollOwner` row yet, proceed with the poll/feed-tick optimistically and call
the claim reducer; a genuine simultaneous claim by two hosts is resolved by
whichever reducer call SpacetimeDB serializes first; the loser's next tick
sees the winner's row and stands down from then on. A single doubled tick at
the moment of the very first claim is the same order of cost
`PollPrStatus`'s own existing "transient failure" tolerance already accepts —
not a correctness problem.

`PollOwner` needs no SQLite/`LocalBoardReads` counterpart: on a single-machine
install with no shared store, there is no claim to contend over, so
`PollPrStatus`/`FeedTick` skip the claim check entirely when
`shared_writer()`/subscription is absent — mirroring how `is_locally_owned`
is already a no-op on one machine.

## Feed-run identity stamping (test #3)

Independent of the claim: `upsert_feed_item`'s new-task branch (and the
equivalent path for any other feed-created row) stamps `Task.created_by =
local_host().owner` — once, on insert, never rewritten, matching
`created_by`'s existing "stamped once" semantics elsewhere. This records
*which host's feed run* actually produced a given row and is a real signal
regardless of the claim design above — with the claim, it also happens to now
unambiguously name the single host that was allowed to run that tick.

## Scope of this phase

Stays inside `dispatch tui`. `PollOwner` and its claim logic are gated on
`Tick()` exactly where `PollPrStatus`/`FeedTick` already are — no separate
daemon binary. A headless "feed process" that can hold a claim without a TUI
open is a later phase; nothing here forecloses it; the claim model is exactly
the seam such a process would plug into.

## Manual override

No TTL, but a stale claim is a real, expected case per user feedback: a feed
epic's owning host can go dark for a long time (laptop off, person on leave)
with no automatic failover. So a claim is permanent **until a human
explicitly overrides it** — anyone can take over any scope's ownership, but
only through a deliberate action, never as a side effect of an ordinary tick.

```
rule OverridePollOwner {
    when: OverridePollOwner(scope, scope_id)

    ensures:
        if exists PollOwner{scope, scope_id}:
            PollOwner.host = local_host()
            PollOwner.claimed_at = now
        else:
            PollOwner.created(scope: scope, scope_id: scope_id, host: local_host(), claimed_at: now)
}
```

The only difference from the passive claim rule is that this one overwrites
an *existing* owner unconditionally — same write, wider precondition.

**Trigger, epic scope:** not a dedicated keybinding — folded into `EditEpic`
(`feeds.allium`/`epics.allium`). When a human sets or changes an epic's
`feed_command` and a `PollOwner` row already names a *different* host, `EditEpic`
shows the same kind of y/n status-bar confirmation `ConfirmDone`/
`ConfirmTrustRepo` already use (`tasks.allium`): "This feed is currently
owned by <host label> — take over?" Declining leaves `feed_command` changed
but ownership untouched; confirming calls `OverridePollOwner`. No prompt at
all when there is no existing owner (a brand-new feed epic, or one that never
claimed) — that path is just an ordinary claim on the next tick, or when the
edit is to some other field. This piggybacks on the moment a human is
already looking at that epic's feed configuration, rather than inventing a
new one.

**Trigger, task scope, and both scopes generically:** an MCP tool
(`override_poll_owner` or similar — named during the spec pass), taking the
scope and id, callable for both epics and host-less PR-poll tasks. Recorded
as knowledge-base learning #817 (repo scope) this session: a new capability
is not TUI-only by default, since agents are first-class users of this
platform — a dispatched agent can call this tool the same way a human uses
the epic-edit confirmation, and it is the *only* trigger for the task scope,
which has no analogous "editing" moment to hook into.

## Known limitation, not solved here

Nothing detects that a claim has gone stale and prompts anyone to override
it — the human has to notice (e.g. a feed epic visibly not updating) and act.
That is the accepted tradeoff: simple, explicit, no watchdog, per user
direction that this is a system of collaborators rather than unreliable
infrastructure needing automated failover.

## Tests to write (TDD, before code)

1. `PollPrStatus`, task **with** worktree: only `task.host` polls; the other
   host's tick issues nothing and its counters never move. (Direct `host`
   check, no claim involved.)
2. `PollPrStatus`, task **without** worktree: exactly one host claims
   ownership and polls; the other host's tick issues nothing, on this tick
   and every tick after.
3. Claim permanence: the owning host keeps polling across many ticks even
   while a second host is also ticking the same task/epic every time —
   ownership never moves.
4. `FeedTick`: exactly one host runs a given feed epic's command per
   interval; the non-owning host's tick is a no-op for that epic, every time.
5. Feed-created tasks: `Task.created_by` is stamped with the running host's
   owner identity on insert, and left untouched on a later update (re-poll)
   of the same feed item.
6. Manual override: a scope already owned by host A is reassigned to host B
   after B's explicit override action, confirmed via the y/n prompt; B then
   polls it and A's tick becomes a no-op. An override with no existing owner
   behaves like a normal claim.
