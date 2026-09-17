# Distributed Dispatch Design

**Task:** #4812 — "Spacetime DB"
**Date:** 2026-09-13
**Status:** revised after adversarial review of both the design and the
SpacetimeDB evaluation. Several claims in the first draft were disproven by
the code and are corrected inline below. Foundations to be implemented this
session; everything else deferred to follow-up tasks.

## Problem

Dispatch is single-machine. Its state lives in one SQLite file
(`src/db/mod.rs`), and nothing in `src/` has any concept of *which* machine a
task belongs to — a grep for `hostname`, `machine_id` or `host_id` across the
crate returns nothing but an unrelated doc comment.

The task asked whether SpacetimeDB could change that: make dispatch a
distributed system where tasks sync between machines, and let feed scripts
write to the database directly instead of routing through dispatch's feed
runtime.

The desired end state, settled in conversation:

- Several people on a team, each on their own machine, share one board.
- Each person's agents run locally, in local worktrees and local tmux windows.
- For a task a teammate dispatched, you see **status plus output** — the card,
  its column, and the ability to read that agent's log or trajectory. You do
  **not** dispatch onto their machine or attach to their tmux.

## SpacetimeDB evaluation

This was the question the task actually asked, so the findings are recorded
here even though the answer is "not as the primary store". Verified against
`clockworklabs/SpacetimeDB@master`, whose `LICENSE.txt` declares the Licensed
Work as SpacetimeDB 2.10.1; latest release v2.10.0, 2026-09-04.

### What SpacetimeDB is

A central server process — not an embedded library, not peer-to-peer. Clients
open a WebSocket, subscribe with SQL, and receive a live local cache. Server
logic lives in the database: *reducers*, which are transactional and sandboxed,
and since v1.10.0 (2025-11-27) *procedures*, which trade those guarantees for
the ability to make outbound HTTP calls.

It is a healthy project: ~25k GitHub stars and commits landing daily.

### Why it cannot be dispatch's primary store

**1. The module cannot drive local execution.**

The first draft of this document claimed the module "cannot reach the outside
world". That is obsolete and was corrected in review. Procedures *can* make
outbound HTTP — `procedure_http_request` is a real host function in
`crates/bindings-sys/src/lib.rs:777`. The accurate statement is narrower and
still decisive:

- **Outbound HTTP to public addresses only.** `crates/core/src/host/instance_env.rs::is_blocked_ipv4`
  walks RFC 6890 and blocks loopback, `10/8`, `172.16/12`, `192.168/16`,
  `169.254/16`, `100.64/10` and the IPv6 equivalents. The DNS resolver filters
  resolved addresses too, so a hostname pointing at a LAN IP is also blocked,
  and redirects into a blocked range are rejected. Loopback is unblockable
  except by recompiling standalone with the test-only
  `allow_loopback_http_for_tests` cargo feature. A procedure therefore cannot
  reach an engineer's laptop, a LAN host, or a local agent.
- **Still no filesystem and no subprocess capability.** The ABI has no `open`,
  no `socket`, no `spawn`, and there is no native module type.
- Reducers remain fully isolated and cannot call a procedure directly, only via
  a schedule table. Procedures are still `unstable` in Rust, run outside a
  transaction, and their `with_tx` closures may be re-executed against
  different database states.

Nearly everything dispatch *does* is a subprocess on a local machine: `git
worktree add`, tmux window creation, spawning `claude`, running feed scripts,
`gh pr view`. None of it can move into the database, and the one escape hatch
that exists cannot reach the machines that would run it.

This also settles the "feed scripts write to the database directly" idea. A
feed script still needs a local process to *execute* it; only the write target
would change. Dispatch already runs an Axum server that could accept that write
today, so SpacetimeDB is not what unlocks it.

**2. No offline support in the Rust client.** [Issue #5481, "Offline-first
support"](https://github.com/clockworklabs/SpacetimeDB/issues/5481), filed
2026-07-04, is open, has no linked PR, and its single comment is from a
community member rather than a maintainer. There is no roadmap commitment.

The SDK source confirms it. `sdks/rust/src/client_cache.rs` describes itself as
"a **read-only** replica of a subset of a remote database". There is no
persistence layer; the only `std::fs::File` in the crate is an optional debug
log. Mutations are messages into a WebSocket channel, and every path returns
`Error::Disconnected` when it is down.

Worse than the first draft claimed: the Rust SDK has **no automatic reconnect**
either. The reconnect fixes in v2.9.0 and v2.10.0 are TypeScript-side. The Rust
disconnect path fires `on_disconnect` and ends the connection; you rebuild the
`DbConnection` yourself. The one thing that persists is the identity token in
`~/.spacetimedb_client_credentials` — auth survives a restart, no data does.

Dispatch's database today is a local file that opens instantly and never fails.
Trading that for a store that requires a reachable server is a regression for a
TUI used all day.

**3. Self-hosting does not recover offline — this is the strongest finding, not
the weakest.** `spacetime start` runs a local server, but SpacetimeDB
Standalone is single-node, and says so in its own source.
`crates/standalone/src/lib.rs:282`:

```rust
// standalone does not support replication.
let num_replicas = 1;
```

The publish path discards whatever replica count a client requests. There is no
replication crate among the 43 in `crates/`, and no hits for `raft`,
`consensus` or `leader_election`. The CLI has no `replicate`, `sync`, `export`
or `import` subcommand.

The replication smoketests that appear in the issue tracker were never tests of
the open-source build: they lived in the old Python suite, queried a
`spacetime-control` database with `node_v2` and `replication_state` tables that
exist nowhere in the open tree, and were run against `../private/docker-compose.yml`.
They were deleted in April 2026 and never ported to the Rust suite.

So the two available shapes are "one shared server, nobody works offline" or "a
local server per laptop, nobody shares" — and the second is what SQLite already
provides, with an added process. A DIY sync between a local and a shared
instance via procedures is theoretically possible but blocked in practice: per
finding 1, the shared server would have to be on a public IP, and you would be
hand-writing conflict resolution anyway.

**4. Migrations.** The first draft said "additive changes only", which is
imprecise. Several removals automigrate (unique constraints, indexes, primary
key annotations, empty tables), and some additions do not (adding a unique or
primary key constraint, adding a column without a default). The part that
matters is exact:

> Removing or modifying existing columns. This includes changing the type,
> canonical name, or order of columns.

— forbidden, requiring the incremental-migration pattern or
`spacetime publish --delete-data`.

`src/db/migrations.rs` is 2482 lines across 96 schema versions
(`LATEST_SCHEMA_VERSION` is 96), several of which rebuild tables in place
(`tasks_new`, `epics_new`, `learnings_new`). That style would not survive.

The incremental pattern's cost, from `clockworklabs/incr-migration-demo`: a
permanent second table duplicating every column, dual writes on every mutation
forever, and a lazy-migrate read path — with no documented step that ever
removes the old table. It compounds per migration. Caveat in SpacetimeDB's
favour: that demo is pinned to 1.0, was last pushed 2025-03-03, and its
motivating example (adding a column) now automigrates thanks to default values.
The pattern is still required for removals and type changes, just less often
than the demo implies.

Beyond those four, the mechanical cost is ~3.5k lines of query code
(`src/db/queries/`) rewritten, plus the single-writer / read-pool connection
model and the `TaskReadStore` mutation seal described in `docs/conventions.md`.

### Licence

Exact wording from `LICENSE.txt`, rather than the first draft's paraphrase:

> **Additional Use Grant:** You may make use of the Licensed Work provided your
> application or service uses the Licensed Work with **no more than one
> SpacetimeDB instance in production** and provided that you do not use the
> Licensed Work for a Database Service.

A "Database Service" explicitly excludes "your employees and contractors", so a
team of engineers running clients against one self-hosted server is clearly
within the grant. Three points the first draft flattened:

- **The limit counts production instances, not seats.** This matters directly:
  an architecture of "a local standalone per laptop syncing to a shared one" is
  N+1 instances, and "in production" is nowhere defined in the licence. Finding
  3 already rules that architecture out technically, but the grant should not be
  recorded as "fine for us" without noting which shape it was scoped to.
- **The Change Date is per-version**, currently 2031-09-08. Each release you
  adopt resets to roughly five years from its own publication; there is no
  converging horizon.
- **The Rust client SDK is BSL too.** `sdks/rust/LICENSE` is byte-identical to
  the root licence, so a dispatch binary linking `spacetimedb-sdk` contains
  Licensed Work. Within the grant, but the terms travel into the binary, which
  matters if dispatch ever ships outside the company. Only `docs/` is
  Apache-2.0.

### Where SpacetimeDB could still earn a place

As the **shared mirror**, not the primary store. Live subscriptions mean a
teammate's card moves on your board with no polling, and SpacetimeDB Identity
supplies per-user auth. Reducers are a reasonable home for sync-conflict rules,
since those are pure logic over table state.

But this is a transport decision, and it is the *last* decision, not the first
— see "Transport" below.

## The distributed design

### Ownership: a task is pinned to a host whenever it has a worktree

The first draft stated this as "the host is whoever holds the worktree", and
then drew a conclusion from it that the code disproves. Both the rule and the
conclusion are corrected here.

The rule. A task's machine-local state — worktree, tmux window, running agent,
`live_subagents`, `live_shells`, the hook timestamps — appears when the task is
dispatched and lives on exactly one machine. So:

- `Task.host` is **nullable**. Null means no machine holds a worktree for it.
- It is set when the worktree is provisioned, inside `TaskService::dispatch`
  (`src/service/tasks/dispatch.rs`).
- It is cleared when the worktree is released — `ArchiveTask`, `DeleteTask`,
  `RetryFresh`.

**The disproven conclusion: "the backlog has no owner".** That does not follow.
`MoveTaskBackward` (`docs/specs/tasks.allium:329`) nulls `tmux_window` but
deliberately preserves the worktree "for resume", pinned by
`move_backward_from_running_detaches_but_keeps_worktree` in
`src/tui/tests/navigation.rs:330`. A task can therefore sit in **backlog**
holding a live worktree, and under the rule above it keeps its host — correctly,
because that worktree is still on one specific disk.

The honest statement is that ownership tracks the worktree, not the column:

> A task with a worktree is pinned to the host that holds it, whatever column
> it is in. Only worktree-less tasks are unowned and collaboratively editable.

A shared, groomable backlog is still the common case, because most backlog
tasks have never been dispatched. But "backlog" is not the test; `worktree ==
null` is.

Note that `core.allium`'s `is_detached` derivation excludes `status = backlog`
from its condition, so the domain model has never had to name this state. The
design inherits that gap rather than resolving it, and resolving it is part of
the gating work below.

### The invariant enforces nothing on its own

The first draft claimed this ownership model "yields the conflict rule for
free". It does not, and that was the most serious error in it.

**`ResumeTask` has no host check.** `docs/specs/dispatch.allium:635` requires
only `status in {running, review, done}`, `worktree != null` and `tmux_window =
null`. It reattaches to the existing worktree path on disk and runs a local
`has_window()` check. On a shared board, a teammate's dispatch sees a detached
task, finds `worktree != null` — a path meaningless on their machine — their
tmux reports no window, and it launches an agent into a directory that does not
exist.

So every rule that acts on worktree or tmux state needs an explicit
`task.host == local_host_id` precondition: `ResumeTask`, `RetryResume`,
`RetryFresh`, and dispatch's reuse-worktree path. This is domain logic, not a
transport concern, and it belongs in the foundations. See F3 below.

### Dispatched work is not single-writer

Also disproven. The first draft claimed conflicts "essentially cannot arise"
because only the owning machine writes a dispatched task's row.

**`PollPrStatus` is a second writer, on every machine.**
`docs/specs/pr-workflow.allium:71`:

```
for task in core/Tasks
    where status = review and url != null and url.url_type = pr
      and pr_poll_due(task):
```

It fires on `Tick()`, with no host filter of any kind. Every teammate's board
independently polls GitHub for the same PR and writes the same row. The PR's
merged/open/closed state is convergent, so that part is harmless. The
per-attempt state is not: `consecutive_permanent_failures`,
`consecutive_transient_failures` and `next_poll_at` are counters that two hosts
polling at their own cadence will race, and `PrPollGaveUp`'s one-time
`UserInformed` side effect can fire on several machines.

PR polling therefore needs host-scoping of its own: poll on the host that holds
the worktree, or elect exactly one poller. A review task with a PR but no
worktree is an open edge case.

`FeedRunner`'s upsert is a weaker version of the same problem — also
host-agnostic, but its writes derive from one deterministic external feed, so
independent hosts largely converge rather than fight.

### Task ids: block allocation, not renumbering

Ids must be globally unique once boards are shared, and they must stay small
integers — a dispatch id is typed by hand constantly, so a UUID is not
acceptable as the primary handle.

**Renumbering on conflict was evaluated and rejected.** The textbook answer —
optimistic concurrency, then renumber the loser — fails because a task id is
not only a database key. `src/dispatch/worktree.rs::worktree_paths` builds the
worktree name directly from it:

```rust
let worktree_name = format!("{}-{slug}", task.id);
```

So id 4812 becomes a directory on disk (`.worktrees/4812-spacetime-db`), a git
branch of the same name that may already be pushed, a tmux window name, the
agent session name `task-4812`, foreign keys in seven tables (`learnings`,
`learning_retrievals`, `learning_verdicts`, `task_shells`, `task_subagents`,
`task_usage`, `todos`), and text already delivered to a running agent and
possibly a PR title. Renumbering a dispatched task means renaming a pushed
branch and a live agent's session.

Restricting renumbering to backlog tasks — freezing the id at dispatch — holds
until a task is dispatched *while offline*, which reintroduces the collision on
a task that already owns a branch.

**Chosen approach: Hi/Lo block allocation.** Prevent conflicts rather than
resolve them. A host reserves a block of ids (e.g. 100) from the shared server,
mints locally from that block with no network round-trip per task, and refills
while online. Ids are globally unique by construction; there is never a conflict
and never a renumber.

Accepted costs, one of which the first draft badly understated:

- **Default board ordering stops tracking creation time.** The first draft
  called this "gaps in the sequence". The real regression is that `id` is the
  implicit creation-order tiebreaker throughout:
  `ORDER BY COALESCE(sort_order, id) ASC, id ASC` at
  `src/db/queries/tasks.rs:239` and `:875`, and `sort_order.unwrap_or(id.0)` at
  `src/tui/mod.rs:1615,1621,1703,1704`, `src/tui/update/navigation.rs:188-197`
  and `src/tui/update/todos.rs:34,36`. This is the board's default order for
  every card the user has not manually reordered. Under blocks, a task from a
  low block sorts before a task from a high block regardless of when it was
  created, permanently, because `sort_order` stays null until a manual reorder.
  Either accept and document that, or make `sort_order` non-optional for
  cross-host lists. **Open.**
- A host must connect once to obtain its first block before it can create
  syncable tasks. A host that has never connected keeps working exactly as
  dispatch does today, purely locally.
- Abandoned blocks leak id space. Harmless at this scale.

**Open problem: merging N existing installs.** Block allocation answers "one
existing install plus N new ones joining". The actual scenario is several
engineers *already* running dispatch solo, each with an independent history
starting at id 1. Host A's task #42 and Host B's task #42 are unrelated and
cannot both enter one id space. Granting future blocks does nothing about
already-existing overlapping ranges. This has no answer yet and belongs to the
shared-server follow-up.

### Sync payload: dirty rows, not an event log

An event log is the right tool when intermediate states matter. In dispatch
they do not. A task that moves backlog → running → review while its owner is
offline only needs to reach teammates as `review`; replaying three events
produces the same result as pushing one row, with more machinery and an
ordering problem.

So: mark a row dirty when it changes, push its current state on reconnect, let
the owning host win. Mechanically a `dirty` flag and a `synced_at` column. This
fails safe — a dropped push simply syncs on the next one.

Revisit an event log only if an audit trail of who moved what, and when, becomes
a requirement. That is a separate feature with a separate justification.

### Epics

A task has one host; an epic spans people. `recalculate_epic_status`
(`src/service/tasks/crud.rs:479`, `src/service/epics.rs:498`, and via
`FeedRunner`) is invoked from every task and epic mutation path in the service
layer, and derives an epic's status from its subtasks. With subtasks spread
across hosts, each host computes a different partial rollup and the status
flaps.

The derivation therefore has to move to whatever holds the full picture — the
shared server. This is the one piece of domain logic that genuinely must live
server-side, and it is a constraint on the transport decision.

### Task watchers

`src/service/tasks/watchers.rs` nudges a *watcher* task's tmux pane when a
*watched* task's status changes. The status write happens wherever the watched
task's owning process runs; the nudge must land in a tmux pane that may be on a
different machine. Tmux is per-machine and there is no cross-host nudge path, so
under a shared board the notification silently finds nothing. Unsolved; grouped
with agent output below.

### Agent output

Reading a teammate's agent output means shipping logs and trajectory
(`src/mcp/trajectory.rs`) off their machine. Deliberately deferred: a separate
data path with its own size, retention and privacy questions, and nothing above
depends on it.

### Transport

Deliberately the last decision. Once ownership, host gating, id allocation and
dirty-row sync are settled, the shared side needs only to hold the
authoritative row set, allocate id blocks, recalculate epic status, and push
changes to subscribers.

Two candidates, to be decided in a follow-up task:

- **SpacetimeDB** — live subscriptions and identity come free; costs a new
  server dependency, a WASM module, a second schema kept in step with the SQLite
  one, and the migration constraints above.
- **Dispatch's existing Axum server** — no new dependency and one schema; costs
  writing the push channel and auth by hand.

Neither is blocked by the foundations below, which is the point of sequencing
them this way.

## Scope of this session: foundations

### F1. Host identity in settings

Generate an opaque host id on first run and persist it in the existing
`settings` table (`src/db/queries/settings.rs`), with a host label defaulting to
the machine's hostname and editable thereafter.

The hostname is deliberately not the identity: two machines can share one,
hostnames change, and a hostname discloses more than a shared board needs.
Separating identity from display keeps both jobs simple.

### F2. `tasks.host`

A nullable column, stamped when the worktree is provisioned and cleared when it
is released, carrying the ownership rule above. Surfaced on the `Task` model
(`src/models/tasks.rs`).

Two SQLite notes from the knowledge base, both load-bearing here:

- A partial unique index over a nullable column does **not** deduplicate rows
  where that column is NULL — SQLite treats every NULL as distinct (learning
  #517). Relevant the moment anyone reaches for a unique constraint involving
  `host` or a future sync key.
- `ADD COLUMN` does not re-resolve the table's triggers, but `DROP COLUMN` does,
  and a drop therefore breaks any stub-schema test fixture whose table lacks a
  column some trigger names (learning #496). F2 only adds, which is the safe
  side; a later agent reversing this should not reach for a drop casually.

### F3. Gate worktree operations on host

Add a `task.host == local_host_id` precondition to every rule that acts on
worktree or tmux state: `ResumeTask`, `RetryResume`, `RetryFresh`, and
dispatch's reuse-worktree path.

On a single machine this is a no-op — the host always matches. On two machines
it is what stops a teammate launching an agent into a directory that is not on
their disk. It is also what gives F2 a consumer in this session rather than
leaving it as inert schema.

### Dropped from this session: explicit id minting

The first draft proposed making dispatch choose task ids in application code,
as a seam for later block allocation, describing it as a pure refactor. Review
disproved that.

Today `insert_task_row` (`src/db/queries/tasks.rs:279`) lets SQLite assign the
id atomically inside one `INSERT`, under the write lock, with no window for a
collision. `docs/conventions.md:335` records that dispatch's "single writer" is
per-process, and that several dispatch processes run concurrently — every Claude
Code hook invokes its own CLI process against the same file. Computing the id in
Rust as `SELECT MAX(id)+1` and then inserting reopens exactly that cross-process
race, for the one column that was immune to it by construction.

The second insert site is also not a simple insert: `upsert_feed_tasks`
(`src/db/queries/tasks.rs:1136`) is an `INSERT ... ON CONFLICT DO UPDATE` in a
loop over N feed items in one transaction, so minting an id per item requires
knowing which rows are new before minting.

This is real work with real hazards and no benefit until blocks exist. It moves
to the block-allocation task, where it must be done as a single atomic statement
or with catch-and-retry on the primary key violation.

### Explicitly not in this session

No network, no shared server, no id blocks, no dirty flag, no sync, no
SpacetimeDB, no PR-poll host scoping, no agent-output shipping, no task-watcher
fix.

## Risks

**The foundations could be wasted if the team-board idea is dropped.** F1 is
inert and nearly free. F2 and F3 together are defensible on their own terms: F3
is a correctness guard the moment anyone runs dispatch against the same repo
from two machines, which is possible today.

**Block allocation changes visible board ordering**, not just id density — see
above. This needs a decision before blocks land, and user-facing documentation
when they do.

**Epic status recalculation must move server-side.** A transport that makes that
awkward damages the epic model. Recorded as a constraint on the transport
decision.

**Merging existing installs has no answer.** If the team's engineers each
already run dispatch, the shared-server task starts with an unsolved id-space
collision.

## Follow-up tasks

Not created yet; to be decided with the user after the foundations land.

1. Shared server: authoritative row set, auth, id-block allocation — including
   the merge problem and explicit id minting.
2. Decide the `sort_order` versus `id` ordering question that blocks introduce.
3. Host-scope `PollPrStatus` so only one machine polls a given PR.
4. Dirty-row tracking and reconnect push.
5. Epic status recalculation moved server-side.
6. Transport decision: SpacetimeDB versus the existing Axum server.
7. Agent log and trajectory shipping, and the cross-host task-watcher nudge.
8. Host renames sync once a shared host registry exists — the label is a shared
   display value, not a local preference. Decides the two consequences deferred
   with it: whether two hosts may hold the same label, and how a stale label
   reads to somebody else.
