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
- JSON dump and restore covering every shared table.
- Seeding reducer: burn the sequence past the highest id in the snapshot, *then*
  load the rows with their explicit ids.
- `dispatch spacetime dump|restore` CLI subcommands.

**Done when** a real board dumps, restores into a fresh instance, and new tasks
get fresh ids.

**Landed 2026-09-17.** Two corrections to the design came out of building it,
both verified against a live SpacetimeDB 2.10.1 instance and recorded in
`docs/specs/spacetime-seed.allium` and the "SpacetimeDB" section of
`docs/reference.md`:

1. **The burn runs before the load, not after.** Burning a loaded table asks the
   store to generate ids the rows already hold; the insert violates the primary
   key and the reducer aborts.
2. **Block pre-allocation does not shorten the burn.** It is one insert and one
   delete per id — still fast (4096 ids in 29 ms), but for a different reason.

The transport is the `spacetime` CLI through `ProcessRunner`, not the Rust SDK
(decided 2026-09-17). Phase 4 links the SDK, where subscriptions need it.
Phase 1 extends the module crate in `spacetime/module/` rather than creating
one.

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

**Landed 2026-09-17.** Three notes, none of which change the shape above:

1. **The spec went to `agent-health.allium`, not `dispatch.allium`.** That
   file's scope line already reads "Claude Code hook handlers" and every
   `Hook*` rule lives there; `dispatch.allium` is scoped to worktrees, claims
   and launch. The new `HookDelivery` surface carries the four guarantees.
2. **One endpoint, not one per hook kind.** `POST /hook` takes a tagged
   `HookRequest` (`src/hooks/wire.rs`). The hook process does all the parsing,
   so an unrecognised argument fails there, before any delivery is attempted —
   which is what keeps "invalid argv" and "board is down" distinguishable.
3. **`src/cli/` had no hook paths to rewrite.** It holds `agent_tree`,
   `agent_diff`, `statusline` and `caller_headers` only; that plan bullet was
   a no-op.

Two things an `allium weed` pass turned up after the first cut, both fixed
here rather than deferred:

4. **The board's address had to be carried to the sessions it launches.** A
   hook finds its board through `--port`/`DISPATCH_PORT`, so a board started
   with `--port` was up and serving while every one of its agents' hooks was
   dropped as unreachable. The settings file dispatch already writes for every
   spawned session now carries `DISPATCH_PORT`, the way the MCP entry already
   carried the port in its url (`src/setup/statusline.rs`).
5. **`dispatch pr-gate` was migrated too.** It is the other PreToolUse hook
   dispatch installs and it still opened the database, so the guarantee would
   have been false as written. It is the one hook that *answers* the tool call
   rather than observing it, so it needed its own rule for an unreachable
   board: it **fails open** — reports, does not block, and leaves the flag
   unshown so the reminder is deferred rather than lost. See `PrLearningsGate`
   in `docs/specs/pr-workflow.allium`.

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

**Landed 2026-09-18.** Three findings, the first of which changes the schema
for every phase after this one:

1. **SpacetimeDB SQL cannot filter on an optional column**, so the subscription
   could not select on `Task.owner` or `Task.epic_id` — the only two things it
   selects on. An `Option<T>` is a SATS sum type and the SQL language has no
   literal and no operator for one. Absence in the module is now a sentinel
   (`""`, `0`) across 28 columns; `sort_order` is the deliberate exception. A
   column type change is NOT automigratable, so this was free now and costs a
   dump-rebuild-restore after Phase 5 seeds a store. See "Why almost nothing
   here is `Option`" in `spacetime/module/README.md`.
2. **The cold start costs nothing**, because the board draws before it
   connects. The round trip itself is 4–44 ms on loopback against an empty
   database. See "Cold start, measured" in the design doc for what that number
   does and does not cover.
3. **The identity conflict is fatal.** A store answering with a different
   person than the one stored stops the board rather than adopting it:
   adopting silently moves every user-board task to a stranger, and a board
   showing none of them looks exactly like a board with nothing on it.

Two pre-existing bugs were fixed in passing: every in-memory database shared
one host id (the schema template replays migration v97's mint, and the backup
API copies rows), which made any two-machine test vacuous; and
`tests/spacetime_module.rs` set `CARGO_TARGET_DIR` process-wide around a
publish, so two concurrent tests swapped build directories mid-build.

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

**Landed 2026-09-19.** Four notes, the third of which is a gate on turning any
of this on.

1. **`todos` gained an `owner`.** A subscription is a `WHERE` clause and the
   table had nothing to put in one — no epic to belong to, no owner to name — so
   the only query a shared store could answer for it was "all of them", i.e.
   every colleague's checklist on every board. Stamped once at creation,
   backfilled by migration v99 to the install's stored identity, and null on an
   install that never connected. `repo_paths` and `repo_base_branches` are
   subscribed to UNFILTERED, deliberately: a path and a branch name describe the
   work rather than the person.
2. **The read source is a seam (`sync::BoardReads`), not a swapped store.**
   `TaskReadStore` spans both halves of the store seam and only the shared half
   can come from a subscription, so the board holds a second, narrow handle for
   the reads that put cards on screen. The runtime picks the backing at
   bootstrap from `DISPATCH_SPACETIME_SERVER`; unset — every board today — is
   the unchanged single-machine install, not a degraded one.
3. **NOT SEEDED, AND NOT SAFE TO SEED YET.** Reads moved; writes did not. A
   board pointed at a store now reads the store and writes SQLite, so anything
   created on it is invisible the moment it is made. That is a coherent
   intermediate state only because nothing is pointed at a store. Phase 6 is
   what makes the variable safe to set, and the seed belongs with it rather than
   here — seeding now would produce a server whose only client corrupts its own
   view of it. The Phase 0 tooling for the seed is unchanged and ready.
4. **One pre-existing bug fell out of the "renders identically" comparison.**
   `list_repo_paths` ordered by `last_used DESC` alone; that column has
   whole-second resolution, so paths saved in one second tie and SQLite broke
   the tie by rowid in practice and not by promise. Both sides now say
   `id ASC`. An unspecified order is not something two stores can agree on.

Two open questions were opened in `sync.allium` rather than answered here:
whether the periodic refresh is still worth keeping now that rows arrive
unasked, and what a store-less board reads once Phase 8 drops the local shared
tables. The second is a real decision and should not be made by the phase that
happens to delete the tables. A third went into `todo.allium`: nothing fills a
null `Todo.owner` once an install first connects, so a todo created before that
moment is permanently invisible rather than temporarily so.

**Two gates on Phase 6, both opened as tasks rather than closed here.** Neither
is new work this phase created; both are things Phase 5 made load-bearing.

- **#4904 — the connection indicator is specified and unimplemented.** Nothing
  in `src/tui/` draws it. That was merely aspirational until reads moved; it is
  now the thing that makes the no-fallback bargain honest, because a board whose
  store is down and a board with nothing on it look identical without it.
- **#4905 — nothing can subscribe to an epic.** The store methods exist with no
  caller and no keybinding, so `subscribed_epics` is always empty and a
  configured board would draw only the operator's epic-less own tasks and zero
  epics. This is a gate on Phase 6 being *usable*, not just a missing feature.

An `allium weed` pass over the two specs this session touched is what surfaced
both, along with four real code bugs in this phase's own work — chief among them
a `ConnectionDropped` rule with no production producer at all. Run it before
declaring a phase done; the spec and the code disagreeing is exactly what it is
for.

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
