# Spike: per-caller views for read enforcement (task #98520, epic #69657)

Date: 2026-10-28. SpacetimeDB 2.12.0 (module and Rust SDK pinned `=2.12.0`).
Source of the throwaway module and client: `2026-10-28-per-caller-views-spike/`
(`.rs.txt` so nothing builds them by accident).

## Verdict

The decision in `workspaces.allium` stands: private tables plus per-caller
`#[view]`s enforce reads, and the board can consume them. Five constraints
change how the follow-up tasks must build it (below, "What the follow-up tasks
must do"). None of them reverses the decision.

## Answers to the task's questions

### 1. Can a client subscribe to a view with a WHERE? Yes.

`SELECT * FROM covered_epics WHERE id = 3` and
`SELECT * FROM visible_epics WHERE workspace = 2` both applied, and delivered
only the matching rows. One-off SQL takes the same `WHERE`. So `own_creations`
and `covered_epics` can each be a view the board subscribes to, with or without
a filter.

Limits found:

- A view cannot be joined with a private table in SQL (`JOIN task` was refused
  as "no such table"). "Tasks of my covered epics" must be its own view; the
  client cannot compose it.
- Views and plain tables cannot be told apart by the SQL string, so
  `queries.rs` keeps the same shape: a list of `SELECT * FROM <view> [WHERE …]`.
- Not tested: a join between two views.

### 2. How are updates pushed, and what do they cost?

**Push.** The server materialises each view per caller (`ViewInstanceArgs::Sender`),
re-runs it after a commit that touched something it read, and diffs the result
against the last materialisation. The client gets only the diff.

- A row update arrives as **INSERT(new) then DELETE(old), same id, same
  millisecond**. There is no update event.
- A write the caller's view does not read arrives as nothing at all.
- Two overlapping subscriptions (`covered_tasks` and `covered_tasks WHERE
  epic_id = 1`) deliver each row once.
- A new follow, a new task and a new task in a private epic of the caller all
  arrived live (about 1 s apart in the test, the pace of my calls).

**Invalidation** (`spacetimedb-datastore-2.12.0/src/locking_tx_datastore/mut_tx.rs`,
`TableReadSet`). A view records what it read:

- a point index read (`.id().find(x)`, `.col().filter(single_value)`) records
  the exact key. Only a write to a row with that key re-runs the view;
- a range scan or a full scan is recorded as a full-table scan. Any write to
  that table re-runs the view.

A re-run is the whole view function in wasm, not an incremental update.

**Cost, measured** (local release server, unoptimised wasm because `wasm-opt`
is not installed, 205 epics, 4,007 tasks, depth 4, 40 users):

| Case | Time |
|---|---|
| Initial subscribe, 4,000-row view | 10-25 ms |
| Bump a task in a 6-row view (one other view also reads it) | p50 1.7 ms, p95 2.5 ms |
| Write that no view reads | about the CLI floor (21-27 ms per `spacetime call`) |
| Bump a task read by 1 / 10 / 40 users' 4,000-row views | 37 / 166 / 298 ms per call (CLI ~27 ms included) |

Cost model: **one write costs (followers of the written epic) × (size of each
follower's covered view)**, roughly 7-8 ms per 4,000-row view. It grows with
followers, not with total users. Two things to watch:

- A view that scans (such as an index range or a full table) re-runs on every
  write to that table. Keep every read a point read (see constraint 1).
- `visible_epics` reads `epic.workspace = N` as one point. A write to any epic
  in a workspace re-runs every member's `visible_epics`. Fine at tens of members;
  it is the first thing to measure again if workspaces get large.

I did not test a row count beyond 4,000 tasks or more than 40 concurrent
callers. The numbers say "linear in followers × view size", not more.

### 3. Can the Rust SDK and our generated bindings consume them? Yes, with one gap.

`spacetime generate --lang rust` emits a `<view>_table.rs` for each view with
`on_insert` / `on_delete` and `count` / `iter`, and compiles against
`spacetimedb-sdk =2.12.0`. The private tables get **only** their row *types*
(`epic_type.rs`, `task_type.rs`); no table accessor, so nothing can subscribe to
them by mistake. A subscription to `SELECT * FROM epic` as a non-owner is
refused with "no such table … may be marked private".

**The gap: view tables have no primary key in the bindings, so no `on_update`.**
`src/sync/sdk_connector/wiring.rs` (lines 34, 131, 281) upserts on `on_update`
and `wiring.rs:281` widens the subtree on a parent change. With views, an update
is INSERT(new) then DELETE(old). A by-id store that runs `delete(id)` on the
second event **erases the row it just updated**. The wiring must either apply
deletes first, or delete only when the stored row equals the old row. This
applies to every table that moves behind a view.

### 4. Does one-off SQL by a non-owner see private tables? No.

Non-owner and non-creator alike get `no such table`: bob reading `epic`, and
alice (the row's own creator) reading `task`. The publishing identity reads
them. That is the trust boundary the spec already names. Subscriptions to a
private table fail the same way.

### 5. Can a view walk the epic tree, or must `private_holder` be denormalised? It can walk.

Both shapes work and return identical rows for alice, bob and a 40-user
workspace member:

- **Down** (`covered_epics`): start from the caller's subscriptions, follow
  `parent_id` index lookups, stop at an epic the caller cannot see. Alice (holds
  a private epic) gets its public child; bob does not. This is
  `covered_epics` at any depth, as `sync.allium` requires.
- **Up** (`visible_epics_walk`): for each epic in the caller's workspaces, walk
  `parent_id` to the nearest private ancestor with `.id().find()`. No
  `private_holder` column.

The denormalised column (`visible_epics`) is simpler to read but the spec lets
visibility change (`SetEpicVisibility`) and lets epics move
(`EpicStaysInItsWorkspace` allows re-parenting inside a workspace). Either one
changes the holder for the whole subtree, so the column needs a cascade in two
reducers, mirrored in `memory_caller` and pair-tested. **Recommendation: derive
it by the upward walk and add no column.** I could not separate the two costs
cleanly (the server caches the materialised view, so a repeat query measured
the CLI), so the saving is argued from the design, not measured. If a later
measurement shows the walk is the hot spot, add the column then; it is an
appended column with a default, which the migration rule already allows.

## What the follow-up tasks must do

1. **Every column a view reads must have a `btree` index, and reads must be
   point reads.** A view handle has no `.iter()` (compile error: "method `iter`
   not found for `epic__ViewHandle`"). A range read works but counts as a full
   scan for invalidation.
2. **One view per question the board asks.** No client-side join of a view with
   a private table. Expect `own_creations` (epics, tasks, packages),
   `covered_epics`, `covered_tasks`, and a visible-by-workspace view per
   private table. Rows about a task (agents, requests, marks, watchers, usage)
   each need a view that checks the task's visibility, per the spec.
3. **Fix the board's apply path for PK-less views** (insert/delete pairs, see
   answer 3) before any table moves behind a view. This is the most likely
   source of a silent bug; give it a conformance test.
4. **Derive the private holder by walking up; do not add `private_holder`.**
   Re-measure if a workspace has hundreds of members.
5. **Budget the cost**: one write re-runs every follower's view. Keep covered
   views to what the board draws; do not put big payload columns in a view row.

## Untested, so not claimed

- A join between two views.
- Whether a view reflects a row before the creating reducer's reply arrives. The
  board's reducer-completion read-back (`generated_id`, relies on `own_creations`)
  needs this. Both happen after commit, but I did not time them against each
  other.
- More than 40 callers or 4,000 tasks.
- Views over `Option`/`Vec` returns with arguments. I used no-argument views
  only.

## What I got wrong during the spike

My first `spacetime publish` used the CLI's default server, which is the real
store on `127.0.0.1:3000`, and created a database called `viewspike` there. I
checked it was mine (two tables, `epic` and `task`, from the throwaway module)
and deleted it with `spacetime delete`; a describe afterward returned 404. No
other database on that store was touched. Every later command passed
`-s http://127.0.0.1:3188` and a separate `--config-path` per identity. The
test server ran with its own data directory under the worktree and was stopped
afterward.
