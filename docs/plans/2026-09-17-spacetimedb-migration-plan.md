# Implementation plan: migrate dispatch to SpacetimeDB

**Design:** [docs/superpowers/specs/2026-09-17-spacetimedb-migration-design.md](../superpowers/specs/2026-09-17-spacetimedb-migration-design.md)
**Task:** #4855, epic #325
**Date:** 2026-09-17

## Goal

SpacetimeDB becomes the authoritative store for tasks, epics, todos, watchers,
live agent state and the host registry. SQLite keeps only the knowledge base,
its embeddings and local UI preferences. No shared table has a second copy.

## How to use this plan

Each phase is a work package sized for one dispatched agent. Phases 0 to 3
change no user-visible behaviour and land before the server exists.

Every phase follows the repo's order: **spec, then tests, then code.** The spec
step is not optional — `docs/specs/*.allium` is the source of truth, and a
behaviour change that skips it is not aligned. Each phase names the spec it
touches and the tests that must fail before any implementation.

Verify with the repo's verify command, read from `get_task`.

---

## Phase 0 — Dump, restore and seed

Builds the escape hatch before anything depends on it.

**Spec:** new `docs/specs/spacetime-seed.allium`. Dump completeness, id
preservation, the sequence-burn obligation, restore idempotency.

**Tests first**
1. A dump of a populated board round-trips: restore into an empty database
   yields byte-identical rows, ids included.
2. A task created immediately after a restore does **not** collide with the
   highest restored id. This is the test that catches the `Auto Inc` trap.
3. Restore is idempotent — running it twice does not duplicate rows.
4. A dump taken while rows change is internally consistent.

**Then**
- JSON dump and restore over the client SDK, covering every shared table.
- Seeding reducer: load rows with explicit ids, then burn the sequence by
  inserting and deleting id-0 rows until the counter passes the highest id.
- `dispatch spacetime dump|restore` CLI subcommands.

**Done when** a real board dumps, restores into a fresh instance, and new tasks
get fresh ids.

---

## Phase 1 — Module and schema

**Spec:** `docs/specs/core.allium` gains `Task.owner`; new `Host` and
`Subscription` entities.

**Tests first**
1. Module schema matches the current SQLite schema field for field, per table.
2. Publishing the module twice automigrates cleanly.
3. `Task.owner` is required when `epic_id` is null, and rejected otherwise.

**Then**
- SpacetimeDB module crate with the 10 shared tables at version 1.
- `hosts` and `subscriptions` tables.
- No dispatch changes. The module stands alone.

**Watch:** column order is load-bearing. A column added anywhere but the end is
a forbidden migration later.

---

## Phase 2 — One connection per machine (the hook rewrite)

Valuable on its own, independent of SpacetimeDB. It removes the cross-process
SQLite races `docs/conventions.md` documents, including the counter desync in
task #3755.

**Spec:** `docs/specs/dispatch.allium` — hooks write through the board, not the
database. Name what happens when the board is down.

**Tests first**
1. Each `hook-*` subcommand posts to the board's HTTP server and mutates the
   same state it does today. One test per hook kind.
2. A hook with no board running exits non-zero without writing, and says so.
   A missing task stays a silent skip, as today.
3. No `hook-*` path opens the database. Assert it structurally, so a new hook
   cannot regress it.
4. Two hooks firing concurrently cannot desync a denormalised counter.

**Then**
- HTTP endpoints on the existing Axum router beside `/mcp`.
- Rewrite `cmd_hook`, `cmd_hook_subagent`, `cmd_hook_shell`,
  `cmd_hook_peer_message` to call them. Delete `open_hook_service`.
- Same for the CLI subcommands in `src/cli/`.

**Decision recorded:** a hook whose board is down drops its event. Today it
writes. This is a deliberate behaviour change and belongs in the spec.

---

## Phase 3 — Store seam

**Spec:** none. Pure refactor.

**Tests first**
1. The existing suite passes unchanged against a trait-object store.
2. A second, in-memory store implementation satisfies the same trait tests.

**Then**
- Put the 10 shared tables behind the existing `*Store` traits so the backend
  can swap. Respect the `TaskReadStore` mutation seal.
- No behaviour change. ~3.7k lines of `src/db/queries/` touched; `tasks.rs`
  alone is 1.2k, so split this phase if it does not fit one session.

---

## Phase 4 — Identity, subscriptions and the connection

**Spec:** `docs/specs/host.allium` gains the user identity and its relationship
to host id. New subscription rules.

**Tests first**
1. A user identity is minted on first connect and persists across restarts.
2. One user with two host ids owns both, and worktree gating still keys on host.
3. A user subscribes to their own user board and any number of epics; they
   cannot subscribe to another user's board.
4. A dropped connection reconnects, and the board shows disconnected state
   while it is down.
5. Board startup surfaces a clear error when the server is unreachable.

**Then**
- SpacetimeDB Identity, stored beside the host id in local settings.
- Subscription management and the board's connect/reconnect loop.
- **Measure cold-start time** and record it. The design accepts a round trip;
  this is where we find out what it costs.

---

## Phase 5 — Cut over reads

**Spec:** no behaviour change intended. Update any spec that names SQLite as
the read path.

**Tests first**
1. Every board view renders from subscription data identically to SQLite.
   Snapshot tests carry most of this.
2. A teammate's task appears without polling when they change it.
3. Rows outside your subscriptions never reach the board.

**Then**
- Board reads from the subscription.
- Seed the server from ragge's board using Phase 0.

---

## Phase 6 — Cut over writes and reducers

**Spec:** `docs/specs/epics.allium` — epic status is derived server-side.

**Tests first**
1. Every mutation becomes a reducer call and produces the same end state.
2. `recalculate_epic_status` gives one answer with subtasks across two hosts.
   This is the flapping the previous design identified.
3. Validators reject the same inputs as today, now server-side.
4. A write with the server down fails loudly and changes nothing.
5. Two hosts writing the same task converge.

**Then**
- Mutations become reducer calls.
- Move `recalculate_epic_status`, validators and feed upsert into reducers.
- Keep dispatch, worktree provisioning and release local.

---

## Phase 7 — Host-scope polling and feeds

**Spec:** `docs/specs/pr-workflow.allium` — `PollPrStatus` gains a host
precondition.

**Tests first**
1. Only the owning host polls a given PR. Two boards do not both advance the
   retry counters.
2. A review task with a PR but no worktree still gets polled, by exactly one
   host. This edge case is open in the previous design.
3. Feed runs write only under the running user's identity.

**Then**
- Host-scope `PollPrStatus` and the feed runner.

---

## Phase 8 — Retire dead tables

**Tests first**
1. A fresh database has no `my_prs`, `review_prs`, `bot_prs` or
   `security_alerts` table. Follow the shape of
   `a_fresh_db_has_no_tips_state_table`.
2. SQLite holds no shared table after migration.

**Then**
- Drop the four legacy PR tables. They are referenced only by
  `src/db/migrations.rs`; the Review and Security boards they served are
  already removed, per `feeds.allium`.
- Drop the SQLite tables now owned by SpacetimeDB.

---

## Sequencing

Phases 0 to 3 are independent of the server and can run in parallel with
standing it up. Phase 5 must follow 4. Phase 6 must follow 5. Phase 8 last.

Phase 3 is the largest and the least interesting. Split it if it does not fit
one session.

## Open questions carried from the design

1. `task_usage` and `usage_events` — shared or local? Defaulting to local.
2. Cross-host task watchers. Tmux is per-machine, so a watcher on another host
   silently finds nothing. Unsolved; not blocking any phase here.
3. Server hosting, operation and backup cadence.
4. Whether learning text later syncs with embeddings recomputed per machine.
