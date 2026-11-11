# Architecture

Key patterns that aren't obvious from reading the code:

- **Message → Command**: `App::update()` processes input messages and returns `Command`s (side effects). Keep rendering pure, effects in commands.
- **Message routing (co-located)**: see [Message routing](#message-routing-co-located) below.
- **Inline-mutation convention**: Input handlers in `input.rs` directly mutate `self.input.mode`, cursor positions, and other UI-only state, returning `vec![]` (no commands). This is intentional — not an Elm Architecture violation. The rule: if a state change has no side effects (no DB write, no process spawn, no network call), mutate inline and return empty. If it needs a side effect, return a `Command`. If a UI handler in `src/tui/input.rs` returns `vec![]` after mutating `self.input.mode`, cursor positions, or selected indices, that is intentional — do not change it to a `Message`.
- **ProcessRunner trait**: Abstraction over git/tmux shell commands. Tests use `MockProcessRunner` — never shell out in tests. `TaskService::new(db, runner)` takes the runner as a **required** argument so this can't be forgotten; the real runner is only reachable through the explicitly named `TaskService::new_with_real_runner(db)`, used by the CLI paths in `src/main.rs`. (`TaskService` genuinely shells out: `spawn_close_attached_pr` runs `gh`, and `wrap_up.rs` runs `git`.) When a test must supply a runner but expects no commands, pass `MockProcessRunner::unused()` — it panics on the first call, so an accidental shell-out fails loudly. `clock` stays an optional builder by contrast: `SystemClock` only reads the wall clock, so a default costs determinism, never a side effect.
- **TaskPatch builder**: Selective field updates for the database. `None` = don't change, `Some(None)` = set field to NULL.
- **MCP server**: Runs on port 3142 (configurable via `DISPATCH_PORT`). Agents call JSON-RPC methods in `src/mcp/handlers/` to update task status. Caller identity is established via `X-Caller-Task-Id` / `X-Caller-Kind` HTTP headers, parsed by the `extract_caller_identity` middleware (`src/mcp/middleware.rs`) and attached to the request as `Result<CallerIdentity, IdentityError>` — every handler that requires authorization extracts this extension rather than accepting an argument.
- **Integration tests**: Use `Store::open_in_memory()`, 
- **Command queue draining**: see [Command queue draining](#command-queue-draining) below.
- **Two full-board refresh paths**: `exec_refresh_from_db` (command queue, inline on the render thread) carries a revision guard (`BoardReads::revision`) because it fires speculatively every 5 ticks; `reload_board` (detached, via the `spawn_refresh_*` helpers and the shared-store row pump) has none because it is only reached after something already established that the board moved. The guard reflects *why* the refresh was requested, not the reads — see the doc comments on both before unifying them. **Both paths read the same set of tables**, through one function (`TuiRuntime::read_board`) that builds the messages, so they cannot drift: there used to be a narrower one covering only tasks and epics, justified by "a local write cannot have moved a repo path", which a shared store makes false.
- **Editor session invariant**: `TuiRuntime` holds an `editor_session: Arc<Mutex<Option<EditorSession>>>` (`src/runtime/mod.rs`). At most one pop-out editor can be open at a time — the runtime refuses to start a new one while the slot is occupied. The slot is `None` when idle.
- **Layout-cache coherence (self-healing)**: see [Layout-cache coherence](#layout-cache-coherence-self-healing) below.
- **Render dirty flag (fail-open)**: see [Render dirty flag](#render-dirty-flag-fail-open) below.

## Message routing (co-located)

`src/tui/dispatcher.rs` only routes the outer `Message` enum to its per-domain inner enum (`Message::Task(tm) => tm.route(app)`). The per-variant wiring lives **beside each `*Message` enum** in `src/tui/messages/<domain>.rs` as an inherent `route(self, app: &mut App) -> Vec<Command>` method (named `route`, not `dispatch`, to stay grep-distinct from the top-level `dispatcher::dispatch`). This makes `messages/*.rs` deliberately *not* a pure-data layer — each domain enum owns the wiring to `App`'s `handle_*` methods. Adding a TUI interaction is a single-file edit here (variant + its `route` arm) plus the `handle_*` implementation in `src/tui/update/<domain>.rs`; only a brand-new *domain* enum adds a line to `dispatcher.rs`. Every `route` arm delegates to a `handle_*` method (no inline `app.board` mutation, no direct `Command` construction) so the shape is uniform across all domains — see `src/tui/messages/split.rs` for the canonical example. Note the *command* side (`Command` → runtime effect) deliberately stays centralized in `src/runtime/commands.rs` rather than mirroring this co-location: `Command` handlers need `TuiRuntime`, so co-locating a `route` on `Command` would invert the `tui → runtime` dependency. The two routing layers use opposite conventions on purpose.

## Command queue draining

`execute_commands` (`src/runtime/event_loop.rs`) loads the initial `Vec<Command>` into a `VecDeque` and drains it iteratively. Most `commands::dispatch` arms return `vec![]`; returning additional commands to trigger a cascade is the exception. Any handler that does produce extra commands extends the queue with `queue.extend(extra)`, so a single message can cascade into multiple commands without recursive calls:

```rust
let mut queue = std::collections::VecDeque::from(cmds);
while let Some(command) = queue.pop_front() {
    let extra = commands::dispatch(command, app, rt);
    queue.extend(extra);
}
```

**Draining happens on the render thread's critical path.** `commands::dispatch` takes `&mut App` and holds it across every `.await`, and `execute_commands` is called inline from `run_loop` between the redraw and the next `next_loop_event`. Most of the ~38 `async fn exec_*` handlers await DB work inline (`src/runtime/tasks.rs`), and every mutation is a store round trip — so a slow write stalls key handling *and* rendering for its full duration. `MIN_FRAME_INTERVAL` and `frame_ready` are downstream of the drain loop and cannot help.

**A handler that shells out must not await it here.** The paragraph above is about DB work, and a subprocess is the worse case: nothing warns about it, and `SUBPROCESS_TIMEOUT` is 120s — two minutes of no frames and no key handling if the command blocks on an index lock, an NFS mount or an unreachable remote. `tokio::task::spawn_blocking` alone does **not** fix this: it keeps the work off a tokio worker, but `.await`ing the handle still parks the drain loop. Send the result back as a `Message` and return the `JoinHandle` un-awaited, as `exec_check_window` and `exec_detect_default_branch` do (`src/runtime/tasks.rs`). Where the handler backs an interaction guarantee about what the user may do *while* it runs, awaiting also makes that guarantee untestable — the user cannot type during an await, so the race it describes never happens and its guard is dead code that tests pass against.

The *notification-driven* refreshes deliberately avoid this: `spawn_refresh_from_db` / `spawn_refresh_task` / `spawn_refresh_epic` (`src/runtime/tasks.rs`) `tokio::spawn` their reads and feed results back as `Message`s. Moving a long-running `exec_*` handler off the critical path means following that pattern. Do it when you have a measured stall, not speculatively — the cost is that the handler's effect becomes observable a loop iteration later, which the `App` state machine has to tolerate.

## Layout-cache coherence (self-healing)

`App.layout` is a [`LayoutCache`](../src/tui/types/layout.rs) grouping six caches derived from `board.tasks`/`board.epics` — `epic_stats_cache`, `epic_placements_cache`, `children_map_cache`, `column_anchor_cache`, `epic_filter_cache`, `task_index` — plus their fingerprints, so the fields that must stay coherent with each other can only be invalidated as a unit (`LayoutCache::invalidate()`). Handlers that mutate the board should still call `App::invalidate_layout_cache()` (directly or via `sync_board_selection()`) as a perf optimization — it forces an immediate rebuild — but it is **not required for correctness**. `cached_epic_stats()` computes a cheap fingerprint (`compute_layout_fingerprint()`: a plain FNV-1a-style fold over id/status/epic-membership/sort_order/completed_at of every task and epic, plus the folded-section set and the board-wide filters — repo, the lens, both hide-lists, search query — deliberately cheaper than a cryptographic hash; the filters are in there because `epic_filter_cache` and `epic_placements_cache` are both derived through them, so a fingerprint blind to them would let a filter change serve a stale board) on every call, including the cache-hit fast path, and self-heals — discarding and rebuilding the five `HashMap`-backed caches — whenever the fingerprint no longer matches the one captured at the last rebuild. `task_index` (used by `find_task_mut`) uses the same pattern with its own lighter fingerprint, `compute_task_ids_fingerprint()` (task ids only, no epics/status/sort_order — `task_index` only maps id→Vec position), so a same-length wholesale replacement of `board.tasks` with a different id set is caught too, not just length changes. This means a handler that forgets to invalidate can no longer produce silently stale UI on any of the six caches; it only pays for one extra rebuild on the next read.

`epic_placements_cache` is the one field with a second, stricter guard. It answers *which columns an epic's card is drawn in* (`board-layout.allium`, "Epic Card Placement"), so a stale read does not merely misorder a column — it makes cards appear in the wrong ones or vanish. Its `&self` readers cannot call `cached_epic_stats()` to trigger the self-heal, so `BoardView::cached_placements()` checks the fingerprint itself and reports a miss rather than handing back a map built before the last change. Read it through that accessor, never as a bare field.

## Render dirty flag (fail-open)

`App::handle_key()` (`src/tui/input.rs`) unconditionally sets `self.dirty = true` after dispatching a key to its mode-specific handler; the render loop in `src/runtime/event_loop.rs` only redraws when `frame_ready` sees `dirty && elapsed_since_render >= MIN_FRAME_INTERVAL` (16ms). A prior version tried to skip redraws for true no-ops (e.g. `j` at the last row) by snapshotting a handful of fields — row, column, mode/view discriminant, caret, buffer length — before and after the handler ran, only marking dirty when one of them changed. That opt-in snapshot was fixed for missed handlers three separate times (popup cursor state, tree-view open/collapse state living in a `RefCell`, edit buffers — all invisible to the snapshot) before being replaced outright: any mutating handler that forgot to also call `self.dirty = true` produced a keystroke with no visible effect until an unrelated event (the next keypress, a tick, an MCP notification) happened to trigger a redraw, at which point the missed change and the new one both appeared at once. `Message`/`Mcp` events already set `dirty = true` unconditionally in `apply_loop_event`; `Key` events now follow the same fail-open rule instead of an opt-in one — the 16ms cap already bounds the cost of a redundant redraw, so there is no correctness or perf reason to special-case no-ops.

## What the services do

`src/service/` and `src/mcp/handlers/` hold no rule about the shared data. A service method does one of three things: it checks the shape of the request (an empty patch, the feed interval floor, a named epic existing); it does work that needs this machine (git, tmux, `claude`, `gh`, local files, embeddings); or it makes one reducer call. The module decides every state change: status stamps and clears, the phoenix respawn, the epic rollup, the close, the Conflict flag, PR state and managed-epic provisioning. An MCP handler translates a tool call into that service call and formats the answer. The learning create checks and the feed interval floor are request shape and stay at the boundary. See `docs/specs/store-split.allium` (S1, X1-X6) and `docs/invariants.md`.

## Error Handling

The codebase uses three error types at different layers:

- **`anyhow::Result`** — infrastructure and IO errors (file operations, shell commands, DB initialization). Used at the outer edges where errors propagate up to the caller.
- **`ServiceError`** (`Validation` / `NotFound` / `Internal`) — business logic errors in `src/service/mod.rs`. MCP handlers match on these to return appropriate JSON-RPC error codes.
- **Domain-specific errors** (`FinishError` in `src/dispatch/finish.rs`) — operations with distinct failure modes that callers need to handle differently (e.g., rebase conflicts vs. push failures).

Rule of thumb: use `ServiceError` for request validation and business rules, domain-specific errors when callers branch on the variant, and `anyhow` for everything else.

## Quick Dispatch

`Shift+D` creates and immediately dispatches a task without going through the task creation dialog. The flow:

1. The `Shift+D` key handler (`src/tui/input/normal.rs`) emits `TaskMessage::QuickDispatch { repo_path, epic_id }`, whose `route` arm produces `Command::QuickDispatch { draft: TaskDraft { title: DEFAULT_QUICK_TASK_TITLE, repo_path, .. }, epic_id }` (`src/tui/commands/task.rs::QuickDispatch`)
2. Runtime handles it in `exec_quick_dispatch()` (`src/runtime/tasks.rs`) — calls `create_task()` then writes a dispatch request with origin `quick` and mode `quick` (`store-split.allium: RequestDispatch`; `tasks.allium: CreateQuickTask`). This host's reactor (`src/runtime/dispatch_host.rs`) provisions it like any other request, and jumps to the new agent because the row's origin is `quick`
3. The created task gets title `"Quick task"` (`DEFAULT_QUICK_TASK_TITLE` in `src/models/tasks.rs`), no tag, no plan
4. If the board has multiple repo paths, `Shift+D` first enters `InputMode::QuickDispatch` (repo picker), then emits `InputMessage::SelectQuickDispatchRepo(idx)` (`src/tui/messages/input.rs`) to resolve the repo before dispatching

Quick dispatch is the same code path as normal dispatch — a request row, provisioned by this host's reactor — the difference is the task is created with defaults, skips the creation dialog entirely, and names the quick prompt variant.

**Command-level shortcut:** `Command::QuickDispatch` skips the `TaskMessage::Dispatch` → `Command::DispatchAgent` hop: `exec_quick_dispatch()` creates the task and writes its request in one step. The outcome comes back like every dispatch's — `TaskMessage::Dispatched` from the reactor, or the failed request row.

## Read enforcement: per-caller views

`docs/specs/workspaces.allium` says who may see what. This section says how the store enforces it.

<!-- allow-phantom-symbol: SpacetimeDB's own attribute, which this module deliberately does not use -->
**Views, not row filters.** SpacetimeDB 2.12 ships row-level filters (`client_visibility_filter`) behind an unstable flag, and its own source says they are not yet enforced. So every table that holds task, epic, package, learning, membership or setting data is private, and clients read it through per-caller views that apply the module's `visible_to` predicates. The store's publisher identity can still read private tables with one-off SQL. The operator, the trust boundary every sight check exempts, is the store's own identity (the database identity: scheduled calls); it reads the tables directly and is served by no view. The publisher is recognised separately, and only for the `seed_*` restore reducers (`spacetime-seed.allium: actor Operator`); it is not exempt from sight checks.

Four facts about views shape the build (proven on a throwaway module, `docs/plans/2026-10-28-per-caller-views-spike.md`):

1. A view reads only through indexes, so every column it filters on needs a btree index, and a range read re-runs the view on every write to that table. So every read in a view is a point read, and a write to an unrelated row never re-runs it.
2. A client cannot join a view with a private table, so each board ask is its own view.
3. A view's rows have no primary key in the generated bindings, so an update arrives as INSERT(new) then DELETE(old) with no `on_update`, and the board's apply path must not let the delete erase the updated row (`sync.allium: AViewUpdateNeverErasesTheRow`).
4. One write re-runs every follower's view, so cost is followers × covered-view size.

A view derives a private epic's holder by walking `parent_id` upward. No `private_holder` column is stored, so `SetEpicVisibility` and re-parenting need no subtree cascade.

**The views.** One per question the board asks, each answering for `ctx.sender()` and each built on the module's own `visible_to` predicates, never a copy (the module and the in-memory store run one function over two sets of tables). Rows are the table's own row type, so a card needs no second lookup.

- `own_epics`, `own_tasks`, `own_packages`: what the caller created, plus the epic-less tasks they own, kept only while `visible_to` still holds (a member who left a workspace loses their own items in it). `own_tasks` also serves a task run on a host the caller owns (one with a current agent, or an open dispatch or session request, on such a host) while the caller still sees it, so unfollowing the epic of a teammate's task one dispatched does not lose the task its agent and requests name.
- `covered_epics`: the epics the caller follows and every descendant, walked by the server at any depth, stopping at an epic the caller cannot see. `covered_tasks`: the tasks of the covered epics.
- `visible_epics`, `visible_tasks`, `visible_packages`: everything visible in a workspace the caller belongs to; a client narrows with `WHERE workspace_id = N` (`WHERE epic_id = N` for tasks).
- Rows about a task (`agents`, `dispatch_requests`, `session_requests`, `unseen_tasks`, `task_subagents`, `learning_retrievals`, `learning_verdicts`): one view each, serving the rows of the tasks the board holds (own tasks and covered tasks). Never wider than the task's own visibility. `unseen_tasks` and `bookmarks` are the caller's own, on tasks they still see. `task_watchers` has no writer, so its view follows the held-task rule like the rest.
- `poll_owners` follow their scope: an epic-scoped row is served while the epic is visible, a task-scoped row while the task is.
- `work_package_links`: one row per link, served while the package and the linked item are both visible.
- `visible_learnings`: the learnings `LearningVisibility` lets the caller see.
- `own_settings`: settings of the hosts the caller owns (D9). Host rows stay readable by all; they carry no task content.
- `visible_workspaces`, `visible_memberships`: the workspaces the caller belongs to and their membership rows (a super-admin sees every one). A view cannot scan a table, so every workspace row carries an indexed `listed` flag that is always true, and a super-admin's view reads every row through it.
- `unseen_items`: the super-admin's listing of private items nobody can see (`admin-surfaces.allium: UnseenItemListing`).

**Private tables with no view.** `todos`, `task_shells`, `retired_feed_items` and `filter_presets` are private and served by no view: no client reads them. `dispatch_request_timeouts` and `session_request_timeouts` are private. `handed_out_agent_names` stays public: names carry no task content and every board needs the whole set. `todos` and `task_shells` have no writer; a publish that drops a table with rows is refused, so dropping them waits for a check that every live store holds none.

**Usage events.** `usage_events` carries an indexed `recorded_by` column, stamped with the caller by `record_usage_event`. An event names no task, so the held-task rule cannot apply to it: an event is its recorder's alone, and a row recorded before the column existed is nobody's. Usage is served on demand, not through a view. A view is recomputed whole when its read set changes, and `record_usage_event` fires on every keybinding, hook and dispatch, so a view would re-run over a person's whole history (up to the 100,000-row cap) on every event. The `own_usage_summary` procedure runs when `query_usage` asks: it reads the caller's rows through the `recorded_by` index, applies the filters and returns grouped counts. The board holds no usage rows.

**Obligations on every change.** Every new reducer and check lives in `spacetime/module/src/` and is mirrored in `src/sync/memory_caller/` with a pair test in `tests/memory_caller_conformance.rs`. `RULES_VERSION` and `BOARD_RULES_VERSION` move together. New columns are appended last with a default, or the module will not automigrate.
