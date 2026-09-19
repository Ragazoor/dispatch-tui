//! Dispatch's shared domain, as a SpacetimeDB module.
//!
//! Spec: `docs/specs/spacetime-seed.allium`.
//!
//! Phase 1 formalises the schema Phase 0 sketched: the ten shared tables at
//! module version 1, plus `Task.owner` and the subscriber a `Subscription`
//! belongs to. See `README.md` for what is still deliberately absent.
//!
//! **Column order is load-bearing.** A column added anywhere but the end of a
//! table is a forbidden migration, recoverable only through the dump and
//! restore this module exists to serve. Every table below is in the same order
//! as its SQLite original, so a snapshot's rows line up field for field.

//! # Absence is a sentinel, not a null
//!
//! Most columns here that the domain treats as absent-able are NOT `Option`.
//! `""` means absent for a string, `0` for an id reference.
//!
//! **SpacetimeDB SQL cannot filter on an optional column.** An `Option<T>` is a
//! SATS sum type, and the SQL reference says the language "does not provide a
//! way to construct them, nore does it provide any scalar operators for them"
//! (<https://spacetimedb.com/docs/reference/sql/>). `WHERE owner = '...'` on an
//! optional column is refused with "cannot be parsed as type
//! `(some: String | none: ())`"; so are `IS NULL`, `!= none` and `some('x')`.
//!
//! A subscription IS a `WHERE` clause. An optional column is therefore one no
//! client can subscribe by — and subscribing is the entire mechanism that keeps
//! a colleague's private work off somebody else's machine
//! (`sync.allium: SendsOnlyWhatWasSubscribedTo`).
//!
//! Only `tasks.owner` and `tasks.epic_id` are filtered on today. The rest is
//! future-proofing, done now because **changing a column's type is not
//! automigratable**: free while no server exists, a manual migration
//! afterwards.
//!
//! Each sentinel is unreachable as a real value by construction. `0` because
//! `#[auto_inc]` treats it as "no id supplied", so real ids start at 1. `""`
//! because paths, timestamps, urls, tags and identities have no meaningful
//! empty value.
//!
//! **`sort_order` is the deliberate exception**, on both `tasks` and `epics`.
//! Zero is a real sort order this codebase writes, and null means something
//! else again — the board orders by `COALESCE(sort_order, id)`, so null says
//! "fall back to the id". A sentinel would silently reorder cards, and nothing
//! would ever subscribe by sort order.
//!
//! The authoritative list, with the reasoning per column, is
//! `SharedTable::sentinel_columns` in `src/spacetime/snapshot.rs`. It is what
//! the dump/restore conversion and the schema parity test both read, and it and
//! this file are checked against each other by
//! `src/spacetime/tests/module_schema.rs`.

use spacetimedb::{ReducerContext, Table};

/// The shared schema this module holds, mirroring SQLite's `user_version` at
/// the point the module was cut.
///
/// A restore refuses a snapshot that does not name this number. The situation
/// the escape hatch is reached for is a migration the store would not perform,
/// which means the schema is precisely what changed — restoring old rows into a
/// new schema without saying so would produce exactly the quiet corruption the
/// rebuild was meant to escape. Phase 1 owns bumping it.
pub const SCHEMA_VERSION: i64 = 97;

/// This module's OWN schema version, which is not SQLite's.
///
/// The two were one number in Phase 0, when the module was a transcription of
/// the SQLite schema and mirroring `user_version` described both. They part
/// company here: `Task.owner` and `Subscription.subscriber` change the module's
/// shape and no SQLite migration corresponds to either, so a single number
/// would have to either claim a `user_version` that does not exist or stop
/// describing the module. It does neither. [`SCHEMA_VERSION`] keeps answering
/// "which SQLite schema do these rows come from?", which is the question a
/// restore asks; this answers "which module shape is holding them?".
pub const MODULE_SCHEMA_VERSION: i64 = 1;

/// One row, holding [`SCHEMA_VERSION`], so a client can read it over SQL
/// without the module having to expose a reducer that returns a value.
///
/// Not a shared table and deliberately absent from the snapshot: it describes
/// the store rather than the domain, and a snapshot that carried it would
/// overwrite the very number it is being checked against.
#[spacetimedb::table(accessor = schema_version, public)]
#[derive(Clone, Debug)]
pub struct SchemaVersion {
    #[primary_key]
    pub id: i64,
    pub version: i64,
    /// [`MODULE_SCHEMA_VERSION`]. Appended rather than replacing `version`:
    /// appending is the one schema change SpacetimeDB will automigrate, and a
    /// restore still needs the SQLite number beside it.
    ///
    /// Always written by the running module from its own constant, never taken
    /// from a caller — a module cannot be wrong about its own shape, and a
    /// caller can.
    ///
    /// The default is 0, not 1: an existing row that predates this column came
    /// from a module that had no version, and 0 says exactly that. Writing 1
    /// would claim the row had already been re-stamped by a version-1 module,
    /// which is the one thing the publish step still has to do.
    #[default(0)]
    pub module_version: i64,
}

/// Stamp the schema version on a brand-new database.
#[spacetimedb::reducer(init)]
pub fn init(ctx: &ReducerContext) {
    ctx.db.schema_version().insert(SchemaVersion {
        id: 1,
        version: SCHEMA_VERSION,
        module_version: MODULE_SCHEMA_VERSION,
    });
}

/// Re-stamp the schema version after an automigration.
///
/// A publish over an existing database does not re-run `init`, so without this
/// the number would keep describing the schema the database was *created* with.
/// Called by the publish step, not by a restore.
#[spacetimedb::reducer]
pub fn set_schema_version(ctx: &ReducerContext, version: i64) {
    let row = SchemaVersion {
        id: 1,
        version,
        module_version: MODULE_SCHEMA_VERSION,
    };
    if ctx.db.schema_version().id().find(1).is_some() {
        ctx.db.schema_version().id().update(row);
    } else {
        ctx.db.schema_version().insert(row);
    }
}

// ---------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------

#[spacetimedb::table(accessor = tasks, public)]
#[derive(Clone, Debug)]
pub struct Task {
    #[primary_key]
    #[auto_inc]
    pub id: i64,
    pub title: String,
    pub description: String,
    pub repo_path: String,
    pub status: String,
    #[default("")]
    pub worktree: String,
    #[default("")]
    pub tmux_window: String,
    #[default("")]
    pub plan_path: String,
    #[default(0)]
    pub epic_id: i64,
    pub sub_status: String,
    #[default("")]
    pub tag: String,
    pub sort_order: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
    pub base_branch: String,
    #[default("")]
    pub external_id: String,
    pub labels: String,
    #[default("")]
    pub last_pre_tool_use_at: String,
    #[default("")]
    pub last_notification_at: String,
    #[default("")]
    pub wrap_up_mode: String,
    #[default("")]
    pub url: String,
    #[default("")]
    pub url_type: String,
    #[default("")]
    pub pr_learnings_gate_shown_at: String,
    pub auto_run_plan: bool,
    pub live_subagents: i64,
    pub stop_pending: bool,
    #[default("")]
    pub stop_pending_at: String,
    pub live_shells: i64,
    #[default("")]
    pub oldest_live_shell_started_at: String,
    #[default("")]
    pub last_peer_message_sent_at: String,
    #[default("")]
    pub last_peer_message_received_at: String,
    pub phoenix: bool,
    /// The machine holding this task's worktree. Null means no machine holds
    /// one — coupled to `worktree`, not to `status`.
    #[default("")]
    pub host: String,
    /// The person whose user board this task sits on, set exactly when the task
    /// has no epic. Rationale and both refusal arms:
    /// `core.allium: OwnerTracksUserBoardTask`. Enforced by [`write_task`].
    ///
    /// **`""` is the absent owner, not a person.** See "Absence is a sentinel,
    /// not a null" in this file's header for why this column is not an
    /// `Option`. The default is absent even though an epic-less task with no
    /// owner does not satisfy the invariant, and that is deliberate: a
    /// migration cannot invent a person. Rows already in the store stay absent
    /// until the seeding client backfills the seeding user onto them
    /// (`spacetime-seed.allium: BackfillTaskOwner`). No new row can widen the
    /// gap, because every writer goes through [`write_task`].
    #[default("")]
    pub owner: String,
    /// When this task last entered done, or `""`. The Done column's ordering
    /// key, read newest-first (`board-layout.allium`, "Done Column Ordering").
    /// Empty until the task first finishes, and deliberately kept when it moves
    /// back out. A sentinel rather than an `Option` for the reason every other
    /// absent-able column here is — see the module header.
    ///
    /// Last, AFTER the module-only `owner`, even though SQLite has it right
    /// after `host`. SpacetimeDB permits a column to be appended and refuses
    /// one inserted anywhere else, and the module has already published
    /// `owner`, so appending is the only legal place — column order in the two
    /// schemas cannot stay identical once a module-only column exists. The
    /// parity test compares the SHARED columns in order and checks the
    /// module-only ones by presence; see `src/spacetime/tests/module_schema.rs`.
    #[default("")]
    pub completed_at: String,
}

#[spacetimedb::table(accessor = epics, public)]
#[derive(Clone, Debug)]
pub struct Epic {
    #[primary_key]
    #[auto_inc]
    pub id: i64,
    pub title: String,
    pub description: String,
    pub status: String,
    #[default("")]
    pub plan_path: String,
    pub sort_order: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
    pub auto_dispatch: bool,
    #[default(0)]
    pub parent_epic_id: i64,
    #[default("")]
    pub feed_command: String,
    #[default(0)]
    pub feed_interval_secs: i64,
    pub group_by_repo: bool,
    pub feed_role: String,
    pub origin: String,
    pub feed_append_only: bool,
    /// The twin of [`Task::completed_at`], on the same terms — a `""`
    /// sentinel, not an `Option`. Also written without a status transition, by
    /// a manual reorder of this epic's card in the Done column.
    #[default("")]
    pub completed_at: String,
}

#[spacetimedb::table(accessor = todos, public)]
#[derive(Clone, Debug)]
pub struct Todo {
    #[primary_key]
    #[auto_inc]
    pub id: i64,
    pub title: String,
    pub done: bool,
    pub sort_order: i64,
    pub created_at: String,
    #[default(0)]
    pub task_id: i64,
    #[default(0)]
    pub epic_id: i64,
    #[default(0)]
    pub parent_id: i64,
    /// The person whose checklist this is (`todo.allium: Todo.owner`).
    ///
    /// The subscription selects on it (`WHERE owner = <me>`), which is the
    /// whole reason it is a required `String` with `""` for absence rather than
    /// an `Option` — see "Why almost nothing here is `Option`" in the README.
    /// `""` is a todo created before its install ever connected; no
    /// subscription returns it.
    #[default("")]
    pub owner: String,
}

#[spacetimedb::table(accessor = task_watchers, public)]
#[derive(Clone, Debug)]
pub struct TaskWatcher {
    #[primary_key]
    #[auto_inc]
    pub id: i64,
    pub watcher_task_id: i64,
    pub target_task_id: i64,
    pub created_at: String,
}

#[spacetimedb::table(accessor = task_shells, public)]
#[derive(Clone, Debug)]
pub struct TaskShell {
    pub task_id: i64,
    pub shell_id: String,
    pub session_id: String,
    pub started_at: String,
}

#[spacetimedb::table(accessor = task_subagents, public)]
#[derive(Clone, Debug)]
pub struct TaskSubagent {
    pub task_id: i64,
    pub agent_id: String,
    pub session_id: String,
    pub started_at: String,
}

#[spacetimedb::table(accessor = repo_paths, public)]
#[derive(Clone, Debug)]
pub struct RepoPath {
    #[primary_key]
    #[auto_inc]
    pub id: i64,
    pub path: String,
    pub last_used: String,
    #[default("")]
    pub verify_command: String,
}

#[spacetimedb::table(accessor = repo_base_branches, public)]
#[derive(Clone, Debug)]
pub struct RepoBaseBranch {
    #[primary_key]
    #[auto_inc]
    pub id: i64,
    pub repo_path: String,
    pub branch: String,
    pub last_used: String,
}

/// The host registry: one row per machine, not one row in total.
///
/// New in this migration. SQLite kept only this install's own identity, in
/// `settings`, because a local board had no way to meet another machine. A
/// shared board does, and a task's `host` is meaningless unless the machine it
/// names can be looked up.
///
/// `core.allium: ExactlyOneLocalHost` is what constrains it now: exactly one
/// row is the machine reading it, and every other row is somebody else's. The
/// invariant was `Hosts.count = 1` until Phase 4, which is a statement this
/// table makes false the moment two machines share a store.
#[spacetimedb::table(accessor = hosts, public)]
#[derive(Clone, Debug)]
pub struct Host {
    #[primary_key]
    pub id: String,
    #[default("")]
    pub label: String,
    /// The `UserIdentity` this machine belongs to (`core.allium: Host.owner`).
    ///
    /// The field that makes "one person, several machines" expressible: a
    /// laptop and a desktop are two rows here carrying the same owner. It is
    /// NOT a substitute for the id — every locality gate compares machine
    /// against machine, because no amount of shared ownership moves a worktree
    /// off the disk it is on.
    ///
    /// Appended last, which is the only position SpacetimeDB will automigrate
    /// into, and carrying a default because an appended column without one is
    /// refused outright.
    ///
    /// `""` means no owner, and that is a real and lasting state rather than a
    /// startup window: a host minted offline on first run has none yet, and an
    /// install that never reaches a store never will. See "Absence is a
    /// sentinel, not a null" in this file's header.
    #[default("")]
    pub owner: String,
}

/// One person's standing interest in one epic (`core.allium: Subscription`).
///
/// Still empty until Phase 4 gives it a writer; present from the start so a
/// snapshot taken today is complete by the same definition as one taken then.
///
/// Subscribing to your own user board is deliberately not a row here. You can
/// only ever subscribe to your own, so the row would exist for everyone always
/// and carry nothing; `Task.owner` resolves it instead.
#[spacetimedb::table(accessor = subscriptions, public)]
#[derive(Clone, Debug)]
pub struct Subscription {
    /// `<subscriber>/<epic_id>`, so re-subscribing overwrites rather than
    /// duplicates.
    ///
    /// A derived key rather than a generated one, because the uniqueness that
    /// matters is over the PAIR and SpacetimeDB indexes single columns. A
    /// generated id would let the same person subscribe to the same epic twice
    /// — not visibly wrong, just quietly double whatever a reader counts — and
    /// would drag this table into the sequence burn for no gain. See
    /// `core.allium: SubscriptionIsUniquePerSubscriberAndEpic`.
    #[primary_key]
    pub id: String,
    pub epic_id: i64,
    /// The `UserIdentity` that holds this subscription. Appended last, which is
    /// the only position SpacetimeDB will automigrate into.
    ///
    /// The empty-string default is unreachable rather than meaningful: nothing
    /// has ever written a subscription, so there is no existing row for it to
    /// apply to. `seed_subscriptions` rejects it outright.
    #[default("")]
    pub subscriber: String,
}

// ---------------------------------------------------------------------------
// The sequence burn
// ---------------------------------------------------------------------------

/// Leave `table`'s id counter strictly past `ceiling`.
///
/// `#[auto_inc]` assigns a value only when the column is zero, so restoring
/// rows with explicit ids leaves the counter exactly where it started — at 1 —
/// and the next created task collides with the oldest restored one. Nothing
/// sets a counter, so the only way to move it is to make the store generate
/// values and throw them away.
///
/// # This must run BEFORE the rows are loaded
///
/// The throwaway inserts ask for ids 1, 2, 3 and so on, which are precisely the
/// ids a restore is about to write. Called against a table that already holds
/// them, the first insert violates the primary key and the whole reducer
/// aborts. There is no way to advance the counter past a row without generating
/// that row's id, so burning a loaded table is not slow, it is impossible.
///
/// Against an empty table it is neither: one insert and one delete per id, once,
/// inside one transaction. The counter's block pre-allocation is reserved
/// storage, not a shortcut past generating each value, so it does not shorten
/// this loop — a board whose highest task id is twenty thousand burns twenty
/// thousand of them. That is affordable because it happens at seed or recovery
/// time and never again.
///
/// Gaps in the id space are expected and harmless: nothing in the domain reads
/// an id as a count or a position.
#[spacetimedb::reducer]
pub fn burn_id_sequence(ctx: &ReducerContext, table: String, ceiling: i64) -> Result<(), String> {
    // One arm per generating table, each identical but for the accessor and the
    // blank row. Written as a macro so a seventh generating table is one line
    // rather than six, and so the six cannot drift apart — this is the one code
    // path whose job is to not lose anything.
    //
    // Tasks are the one table with a write seam (`write_task`) and this does not
    // use it, on purpose. The seam upserts and returns a Result; the burn needs
    // the generated id back, and inserts a row it deletes in the next statement,
    // so the row never becomes state any invariant is about. Giving the macro a
    // task-shaped special case would cost the uniformity that is its whole
    // point. What keeps it honest instead is `blank_task`'s SCRATCH_OWNER, which
    // `the_blank_task_the_burn_throws_away_satisfies_the_invariant` pins.
    macro_rules! burn_table {
        ($accessor:ident, $blank:expr) => {
            burn(ceiling, || {
                let id = ctx.db.$accessor().insert($blank).id;
                ctx.db.$accessor().id().delete(id);
                id
            })
        };
    }

    match table.as_str() {
        "tasks" => burn_table!(tasks, blank_task()),
        "epics" => burn_table!(epics, blank_epic()),
        "todos" => burn_table!(todos, blank_todo()),
        "task_watchers" => burn_table!(task_watchers, blank_watcher()),
        "repo_paths" => burn_table!(repo_paths, blank_repo_path()),
        "repo_base_branches" => burn_table!(repo_base_branches, blank_repo_base_branch()),
        // `task_shells`, `task_subagents`, `hosts` and `subscriptions` generate
        // no ids, so there is nothing to burn. Accepted rather than rejected so
        // a caller can loop over every shared table without a special case.
        "task_shells" | "task_subagents" | "hosts" | "subscriptions" => {}
        other => return Err(format!("unknown table {other}")),
    }
    Ok(())
}

/// Insert a blank task asking the store for an id, and leave it there.
///
/// The verification tool for a restore, and the only way to answer the question
/// that matters: **does the next task created collide with a restored one?**
/// Asserting that the burn loop ran proves the code calls itself; asserting on
/// the id this hands back proves the property. The caller reads the id back out
/// of `tasks` and deletes the row.
#[spacetimedb::reducer]
pub fn probe_generated_task_id(ctx: &ReducerContext) -> Result<(), String> {
    write_task(
        ctx,
        Task {
            title: "probe".into(),
            ..blank_task()
        },
    )
}

// ---------------------------------------------------------------------------
// Timestamps
// ---------------------------------------------------------------------------

/// SQLite's timestamp spelling, which both stores write.
const SQLITE_TIMESTAMP: &str = "%Y-%m-%d %H:%M:%S%.3f";

/// Format an instant the way the SQLite side does.
///
/// Takes micros rather than a [`Timestamp`] so it is testable without a store;
/// [`now`] is the one-line adapter the reducers use.
///
/// An instant outside chrono's representable range formats as the empty string
/// — the module's spelling of absent. It cannot happen with a real store clock,
/// and the alternative is a panic inside a reducer, which aborts the whole
/// transaction over a timestamp.
fn format_timestamp_micros(micros: i64) -> String {
    match chrono::DateTime::from_timestamp_micros(micros) {
        Some(dt) => dt.format(SQLITE_TIMESTAMP).to_string(),
        None => String::new(),
    }
}

/// The store's clock, in the store's timestamp format.
///
/// `ctx.timestamp` rather than any host clock: it is the same instant for every
/// row a reducer writes, which is what makes a multi-row mutation carry one
/// consistent `updated_at` instead of several that happen to be close.
fn now(ctx: &ReducerContext) -> String {
    format_timestamp_micros(ctx.timestamp.to_micros_since_unix_epoch())
}

// ---------------------------------------------------------------------------
// Epic status derivation
// ---------------------------------------------------------------------------

/// The statuses an epic or a task can hold, in the store's spelling.
///
/// Listed rather than parsed loosely, so an unrecognised one is a REFUSAL
/// rather than a value that falls through to a plausible branch. See
/// `derive_epic_status`'s unknown-status arm for why that matters here in
/// particular.
const ARCHIVED: &str = "archived";
const DONE: &str = "done";
const BACKLOG: &str = "backlog";
const KNOWN_STATUSES: [&str; 5] = [BACKLOG, "running", "review", DONE, ARCHIVED];

/// Derive an epic's status from its children's, or `None` for "leave it alone".
///
/// The whole of `epics.allium: EpicStatusRecalculation`'s derivation, as a pure
/// function over the child statuses. It is pure on purpose: this is the one
/// piece of logic the migration moved server-side, and the argument for moving
/// it is about WHICH children are visible rather than about how the answer is
/// computed — so the computation is testable without a store, and the
/// visibility is what the reducer below supplies.
///
/// `children` carries every child's status, tasks and sub-epics alike, INCLUDING
/// archived ones; filtering them out is this function's job rather than the
/// caller's, so the "archived children are not children" rule has one home.
///
/// `None` means no write. That is distinct from writing the same value back: a
/// write stamps `updated_at`, and on the forward arm `completed_at` too.
fn derive_epic_status(current: &str, children: &[String]) -> Option<&'static str> {
    // FIRST, and deliberately. An archived epic is terminal for this
    // derivation, so an all-done child set must not flip it back to done.
    if current == ARCHIVED {
        return None;
    }

    // An unknown status anywhere is a refusal, not a guess. The realistic
    // producer is a board running a newer binary than this module, and both
    // guesses are wrong in a way nobody can see: treating it as done finishes
    // an epic that is not finished, and treating it as unfinished holds one
    // open forever. Declining to write leaves the epic where it is and leaves
    // the next recalculation — by then perhaps against an updated module — free
    // to get it right.
    if children.iter().any(|s| !KNOWN_STATUSES.contains(&s.as_str())) {
        return None;
    }

    let active: Vec<&str> = children
        .iter()
        .map(String::as_str)
        .filter(|s| *s != ARCHIVED)
        .collect();

    // No active children is NOT all-done. A freshly created epic has none and
    // must not be born done.
    if active.is_empty() {
        return None;
    }
    if active.iter().all(|s| *s == DONE) {
        return (current != DONE).then_some(DONE);
    }
    // The regression: a done epic with an unfinished child is not done.
    if current == DONE {
        return Some(BACKLOG);
    }
    None
}

/// Whether moving from `prior` to `next` stamps a fresh completion.
///
/// The module's copy of `models::completed_at_for_status_transition`, on the
/// same two rules: only the transition INTO done stamps one, and a regression
/// out of done leaves the old stamp standing because `completed_at` records the
/// last completion rather than the current state.
fn stamps_completion(prior: &str, next: &str) -> bool {
    prior != DONE && next == DONE
}

/// The only way a task reaches the store.
///
/// Every writer goes through here so the ownership rule is unavoidable rather
/// than merely available. A free `validate_task_ownership` makes the rule
/// *callable*; this makes it the only path, which is what stops Phase 6's
/// writer from being asked by a doc comment to remember it.
///
/// It is also what makes [`SCRATCH_OWNER`] load-bearing instead of decorative:
/// the burn's throwaway rows and the probe row are epic-less, so without an
/// owner they are rows this function refuses.
///
/// Upserts by id, which is what makes a re-seed a no-op. An id of zero means
/// "generate one", so it can only be an insert.
fn write_task(ctx: &ReducerContext, row: Task) -> Result<(), String> {
    validate_task_ownership(row.epic_id, &row.owner)
        .map_err(|why| format!("task {}: {why}", row.id))?;
    if row.id != 0 && ctx.db.tasks().id().find(row.id).is_some() {
        ctx.db.tasks().id().update(row);
    } else {
        ctx.db.tasks().insert(row);
    }
    Ok(())
}

/// Generate and discard until the counter is strictly past `ceiling`.
///
/// `generate` returns the id it was handed, so the loop can stop on the value
/// rather than on a count — a counter that started above zero, or one that
/// skipped a block, both end this loop correctly.
fn burn(ceiling: i64, mut generate: impl FnMut() -> i64) {
    // Strictly greater, not equal: a counter sitting exactly on the ceiling
    // hands that id out next, which is the same collision one iteration later.
    while generate() < ceiling {}
}

fn blank_task() -> Task {
    Task {
        id: 0,
        title: String::new(),
        description: String::new(),
        repo_path: String::new(),
        status: "backlog".into(),
        worktree: String::new(),
        tmux_window: String::new(),
        plan_path: String::new(),
        epic_id: 0,
        sub_status: "none".into(),
        tag: String::new(),
        sort_order: None,
        created_at: String::new(),
        updated_at: String::new(),
        base_branch: "main".into(),
        external_id: String::new(),
        labels: "[]".into(),
        last_pre_tool_use_at: String::new(),
        last_notification_at: String::new(),
        wrap_up_mode: String::new(),
        url: String::new(),
        url_type: String::new(),
        pr_learnings_gate_shown_at: String::new(),
        auto_run_plan: false,
        live_subagents: 0,
        stop_pending: false,
        stop_pending_at: String::new(),
        live_shells: 0,
        oldest_live_shell_started_at: String::new(),
        last_peer_message_sent_at: String::new(),
        last_peer_message_received_at: String::new(),
        phoenix: false,
        host: String::new(),
        // A blank has no epic, so the invariant requires an owner. This one is
        // never a real person: the row exists to be generated and thrown away
        // by `burn_id_sequence`, and the only copy that outlives a call is the
        // `probe_generated_task_id` row an operator deletes by hand.
        owner: SCRATCH_OWNER.into(),
        completed_at: String::new(),
    }
}

/// The owner on a throwaway row. Not a `UserIdentity` and not shaped like one,
/// so a scratch row that escapes into a real board is obvious rather than
/// plausible.
pub const SCRATCH_OWNER: &str = "module-scratch";

fn blank_epic() -> Epic {
    Epic {
        id: 0,
        title: String::new(),
        description: String::new(),
        status: "backlog".into(),
        plan_path: String::new(),
        sort_order: None,
        created_at: String::new(),
        updated_at: String::new(),
        auto_dispatch: false,
        parent_epic_id: 0,
        feed_command: String::new(),
        feed_interval_secs: 0,
        group_by_repo: false,
        feed_role: "none".into(),
        origin: "manual".into(),
        feed_append_only: false,
        completed_at: String::new(),
    }
}

fn blank_todo() -> Todo {
    Todo {
        id: 0,
        title: String::new(),
        done: false,
        sort_order: 0,
        created_at: String::new(),
        task_id: 0,
        epic_id: 0,
        parent_id: 0,
        owner: String::new(),
    }
}

fn blank_watcher() -> TaskWatcher {
    TaskWatcher {
        id: 0,
        watcher_task_id: 0,
        target_task_id: 0,
        created_at: String::new(),
    }
}

fn blank_repo_path() -> RepoPath {
    RepoPath {
        id: 0,
        path: String::new(),
        last_used: String::new(),
        verify_command: String::new(),
    }
}

fn blank_repo_base_branch() -> RepoBaseBranch {
    RepoBaseBranch {
        id: 0,
        repo_path: String::new(),
        branch: String::new(),
        last_used: String::new(),
    }
}

// ---------------------------------------------------------------------------
// Seeding
// ---------------------------------------------------------------------------

// One reducer per table, each taking a BATCH.
//
// Per table rather than one generic reducer, because the module has to name the
// row type either way and a match over a JSON blob would move the column list
// from the compiler's problem to ours — on the one code path whose whole job is
// to not lose a column.
//
// Batched rather than per row, because every call is a round trip and a real
// board has thousands of rows. The caller chunks; see `SpacetimeCliStore` for
// the size it picks and why.
//
// Every one of these is an UPSERT: a row whose key is present is overwritten
// with this version of it, a row whose key is absent is inserted. That is what
// makes a second restore a no-op and a restore interrupted halfway safe to run
// again.

/// `core.allium: OwnerTracksUserBoardTask`, as a predicate.
///
/// Both directions refuse: an epic-less task with no owner, and an epic task
/// that carries one. The spec says why; the short version is that two answers
/// to "whose board is this?" are worse than none.
///
/// A free function rather than reducer-inline code so it can be tested without
/// a live `ReducerContext`. [`write_task`] is what makes it unavoidable.
/// Both arguments are in the STORE's vocabulary, not the domain's: `0` is no
/// epic and `""` is no owner. See "Absence is a sentinel, not a null" in this
/// file's header. Whitespace counts as absent too — a blank owner answers
/// nothing while passing any test for presence, which is the gap this closes.
pub fn validate_task_ownership(epic_id: i64, owner: &str) -> Result<(), String> {
    let has_epic = epic_id != 0;
    let has_owner = !owner.trim().is_empty();
    match (has_epic, has_owner) {
        (false, false) => {
            Err("a task with no epic sits on a user board and needs an owner".into())
        }
        (true, true) => Err(format!(
            "task is in epic {epic_id} and must not also carry the owner {owner:?}"
        )),
        _ => Ok(()),
    }
}

/// Write tasks with the ids they already have.
///
/// **A snapshot taken before `owner` existed is refused here, by design.** Every
/// epic-less row in it fails the check, loudly and before anything is written.
/// Backfilling the seeding user onto those rows is the seeding client's job,
/// named in the migration design as one of the two seed-time backfills — not
/// something this reducer should guess at, because the answer is *which person*
/// and the module has no way to know.
#[spacetimedb::reducer]
pub fn seed_tasks(ctx: &ReducerContext, rows: Vec<Task>) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_tasks needs each task's real id, not a generated one".into());
        }
        write_task(ctx, row)?;
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_epics(ctx: &ReducerContext, rows: Vec<Epic>) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_epics needs each epic's real id".into());
        }
        if ctx.db.epics().id().find(row.id).is_some() {
            ctx.db.epics().id().update(row);
        } else {
            ctx.db.epics().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_todos(ctx: &ReducerContext, rows: Vec<Todo>) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_todos needs each todo's real id".into());
        }
        if ctx.db.todos().id().find(row.id).is_some() {
            ctx.db.todos().id().update(row);
        } else {
            ctx.db.todos().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_task_watchers(ctx: &ReducerContext, rows: Vec<TaskWatcher>) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_task_watchers needs each watcher's real id".into());
        }
        if ctx.db.task_watchers().id().find(row.id).is_some() {
            ctx.db.task_watchers().id().update(row);
        } else {
            ctx.db.task_watchers().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_repo_paths(ctx: &ReducerContext, rows: Vec<RepoPath>) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_repo_paths needs each row's real id".into());
        }
        if ctx.db.repo_paths().id().find(row.id).is_some() {
            ctx.db.repo_paths().id().update(row);
        } else {
            ctx.db.repo_paths().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_repo_base_branches(
    ctx: &ReducerContext,
    rows: Vec<RepoBaseBranch>,
) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_repo_base_branches needs each row's real id".into());
        }
        if ctx.db.repo_base_branches().id().find(row.id).is_some() {
            ctx.db.repo_base_branches().id().update(row);
        } else {
            ctx.db.repo_base_branches().insert(row);
        }
    }
    Ok(())
}

// Tables with no generated id. Identity comes from the row's own columns, and
// there is no index to look it up by, so a re-seed scans and removes the old
// row before writing the new one. Linear per row and acceptable: these tables
// hold live agent state, which is small and short-lived, not history.

#[spacetimedb::reducer]
pub fn seed_task_shells(ctx: &ReducerContext, rows: Vec<TaskShell>) -> Result<(), String> {
    for row in rows {
        if let Some(existing) = ctx
            .db
            .task_shells()
            .iter()
            .find(|e| e.task_id == row.task_id && e.shell_id == row.shell_id)
        {
            ctx.db.task_shells().delete(existing);
        }
        ctx.db.task_shells().insert(row);
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_task_subagents(ctx: &ReducerContext, rows: Vec<TaskSubagent>) -> Result<(), String> {
    for row in rows {
        if let Some(existing) = ctx
            .db
            .task_subagents()
            .iter()
            .find(|e| e.task_id == row.task_id && e.agent_id == row.agent_id)
        {
            ctx.db.task_subagents().delete(existing);
        }
        ctx.db.task_subagents().insert(row);
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_hosts(ctx: &ReducerContext, rows: Vec<Host>) -> Result<(), String> {
    for row in rows {
        if row.id.is_empty() {
            return Err("a host needs its minted id".into());
        }
        if ctx.db.hosts().id().find(row.id.clone()).is_some() {
            ctx.db.hosts().id().update(row);
        } else {
            ctx.db.hosts().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_subscriptions(ctx: &ReducerContext, rows: Vec<Subscription>) -> Result<(), String> {
    for row in rows {
        if row.id.is_empty() {
            return Err("a subscription needs an id".into());
        }
        if row.subscriber.trim().is_empty() {
            return Err(format!("subscription {} needs a subscriber", row.id));
        }
        if ctx.db.subscriptions().id().find(row.id.clone()).is_some() {
            ctx.db.subscriptions().id().update(row);
        } else {
            ctx.db.subscriptions().insert(row);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Mutations (Phase 6)
// ---------------------------------------------------------------------------
//
// EVERY SHARED WRITE ENTERS HERE. `sync.allium: BoardWritesThroughTheStore` is
// what makes that true from the client's side; this section is the other half.
// The seed reducers above are not an exception — they are the restore path, and
// `spacetime-seed.allium` scopes them to it.
//
// WHY A REDUCER PER MUTATION rather than one "apply this row" entry point. Two
// reasons, and the second is the load-bearing one:
//
//   1. A reducer is a transaction. Whatever a mutation has to keep true — an
//      epic's derived status, a claim that only one host may win — is kept true
//      inside the same transaction as the write, so no client ever observes the
//      half-way state. `sync.allium: EveryMutationIsAtomicAndAnswered`.
//   2. A full-row write is a full-row CLOBBER. Two hosts editing different
//      fields of the same task would each send a complete row built from what
//      they last saw, and the later one would undo the earlier one's change to
//      a field it never touched. The patch reducers below take one `Option` per
//      field, where `None` means "leave it alone" — so two hosts touching
//      different fields converge on both changes rather than on one.
//
// `Option` IS FINE IN A REDUCER ARGUMENT. The module header's "absence is a
// sentinel" rule is about COLUMNS, because a subscription is a `WHERE` clause
// and SQL cannot filter an optional column. An argument is never filtered on,
// so the patch structs below use `Option` for the thing it is actually good at:
// distinguishing "set this to absent" (`Some("")`) from "do not touch it"
// (`None`).

/// One field of a patch: `None` leaves it alone.
///
/// A type alias rather than a bare `Option`, so the intent reads at every use
/// site. The inner value is already the column's own type, absence sentinel
/// included — `Some(String::new())` clears a string column.
type Patch<T> = Option<T>;

/// The fields of a task a patch may change.
///
/// `id` is not among them: it names the row rather than describing it. Neither
/// is `created_at`, which records something that already happened.
#[derive(spacetimedb::SpacetimeType, Clone, Debug, Default)]
pub struct TaskPatch {
    pub title: Patch<String>,
    pub description: Patch<String>,
    pub repo_path: Patch<String>,
    pub status: Patch<String>,
    pub worktree: Patch<String>,
    pub tmux_window: Patch<String>,
    pub plan_path: Patch<String>,
    pub epic_id: Patch<i64>,
    pub sub_status: Patch<String>,
    pub tag: Patch<String>,
    /// Doubly optional, and the one place the module's sentinel rule does not
    /// reach: `sort_order` is a genuinely nullable column (see the header), so
    /// "do not touch" and "set to null" are two different `None`s and need two
    /// levels to tell apart.
    pub sort_order: Patch<Option<i64>>,
    pub base_branch: Patch<String>,
    pub external_id: Patch<String>,
    pub labels: Patch<String>,
    pub last_pre_tool_use_at: Patch<String>,
    pub last_notification_at: Patch<String>,
    pub wrap_up_mode: Patch<String>,
    pub url: Patch<String>,
    pub url_type: Patch<String>,
    pub pr_learnings_gate_shown_at: Patch<String>,
    pub auto_run_plan: Patch<bool>,
    pub live_subagents: Patch<i64>,
    pub stop_pending: Patch<bool>,
    pub stop_pending_at: Patch<String>,
    pub live_shells: Patch<i64>,
    pub oldest_live_shell_started_at: Patch<String>,
    pub last_peer_message_sent_at: Patch<String>,
    pub last_peer_message_received_at: Patch<String>,
    pub phoenix: Patch<bool>,
    pub host: Patch<String>,
    pub owner: Patch<String>,
    pub completed_at: Patch<String>,
}

/// Apply a patch to a row in place.
///
/// Written as a macro over the field list so a new column is one line here
/// rather than three, and so no column can be silently left unpatchable — the
/// compiler checks each name against both structs.
macro_rules! apply_patch {
    ($row:ident, $patch:ident, $($field:ident),+ $(,)?) => {
        $(if let Some(value) = $patch.$field { $row.$field = value; })+
    };
}

/// The fields of an epic a patch may change.
#[derive(spacetimedb::SpacetimeType, Clone, Debug, Default)]
pub struct EpicPatch {
    pub title: Patch<String>,
    pub description: Patch<String>,
    pub status: Patch<String>,
    pub plan_path: Patch<String>,
    pub sort_order: Patch<Option<i64>>,
    pub auto_dispatch: Patch<bool>,
    pub parent_epic_id: Patch<i64>,
    pub feed_command: Patch<String>,
    pub feed_interval_secs: Patch<i64>,
    pub group_by_repo: Patch<bool>,
    pub feed_role: Patch<String>,
    pub origin: Patch<String>,
    pub feed_append_only: Patch<bool>,
    pub completed_at: Patch<String>,
}

/// The fields of a todo a patch may change.
#[derive(spacetimedb::SpacetimeType, Clone, Debug, Default)]
pub struct TodoPatch {
    pub title: Patch<String>,
    pub done: Patch<bool>,
    pub sort_order: Patch<i64>,
    pub task_id: Patch<i64>,
    pub epic_id: Patch<i64>,
    pub parent_id: Patch<i64>,
    pub owner: Patch<String>,
}

// -- Tasks ------------------------------------------------------------------

/// Create a task, and recalculate the epic it lands in.
///
/// The row arrives with `id = 0`, which is how `#[auto_inc]` is asked to
/// generate one. The caller learns the generated id by watching the row arrive
/// on its subscription, because a reducer returns no value — see
/// `sync.allium: EveryMutationIsAtomicAndAnswered`.
#[spacetimedb::reducer]
pub fn create_task(ctx: &ReducerContext, row: Task) -> Result<(), String> {
    let epic_id = row.epic_id;
    write_task(
        ctx,
        Task {
            // Never trust an incoming id on a create. A caller that supplied
            // one would be choosing an id the store has not reserved, and the
            // next generated id would collide with it.
            id: 0,
            ..row
        },
    )?;
    recalculate_epic_chain(ctx, epic_id);
    Ok(())
}

/// Change some fields of a task, and recalculate whatever epics it touched.
///
/// A missing task is a silent no-op rather than an error. The realistic
/// producer is a hook or a watcher firing against a task somebody deleted
/// meanwhile, and that is a race rather than a mistake — the same bargain the
/// SQLite side already makes.
#[spacetimedb::reducer]
pub fn patch_task(ctx: &ReducerContext, id: i64, patch: TaskPatch) -> Result<(), String> {
    let Some(mut row) = ctx.db.tasks().id().find(id) else {
        return Ok(());
    };
    let was_in_epic = row.epic_id;
    let prior_status = row.status.clone();
    // Read before `apply_patch!` consumes the patch's fields.
    let caller_named_a_completion = patch.completed_at.is_some();
    let moved_to_epic = patch.epic_id;

    apply_patch!(
        row,
        patch,
        title,
        description,
        repo_path,
        status,
        worktree,
        tmux_window,
        plan_path,
        epic_id,
        sub_status,
        tag,
        sort_order,
        base_branch,
        external_id,
        labels,
        last_pre_tool_use_at,
        last_notification_at,
        wrap_up_mode,
        url,
        url_type,
        pr_learnings_gate_shown_at,
        auto_run_plan,
        live_subagents,
        stop_pending,
        stop_pending_at,
        live_shells,
        oldest_live_shell_started_at,
        last_peer_message_sent_at,
        last_peer_message_received_at,
        phoenix,
        host,
        owner,
        completed_at,
    );

    // The completion stamp is the store's, not the caller's, unless the caller
    // named one explicitly. A client that computed it from its own clock would
    // make the Done column's ordering depend on whose laptop was fast.
    if !caller_named_a_completion && stamps_completion(&prior_status, &row.status) {
        row.completed_at = now(ctx);
    }
    row.updated_at = now(ctx);

    write_task(ctx, row)?;

    // BOTH epics, and in that order. A task moved between epics leaves one
    // possibly complete and makes the other possibly incomplete; recalculating
    // only the destination would leave the source stuck open.
    recalculate_epic_chain(ctx, was_in_epic);
    if let Some(now_in_epic) = moved_to_epic {
        if now_in_epic != was_in_epic {
            recalculate_epic_chain(ctx, now_in_epic);
        }
    }
    Ok(())
}

/// Delete a task and everything that only referred to it.
///
/// The watcher rows go with it in the same transaction. A watch pointing at a
/// task that no longer exists is not a row anybody can act on, and leaving it
/// would make "who is watching me?" answerable with a ghost.
#[spacetimedb::reducer]
pub fn delete_task(ctx: &ReducerContext, id: i64) -> Result<(), String> {
    let Some(row) = ctx.db.tasks().id().find(id) else {
        return Ok(());
    };
    let epic_id = row.epic_id;
    ctx.db.tasks().id().delete(id);
    delete_agent_state_for(ctx, id);
    for watch in ctx
        .db
        .task_watchers()
        .iter()
        .filter(|w| w.watcher_task_id == id || w.target_task_id == id)
        .collect::<Vec<_>>()
    {
        ctx.db.task_watchers().id().delete(watch.id);
    }
    recalculate_epic_chain(ctx, epic_id);
    Ok(())
}

/// Drop the live-session rows a task owns.
///
/// Shared by deletion and by the session-clearing paths. Keyed by `task_id`
/// rather than by the row's own identity because neither table has one: they
/// are a set of live things, not entities with a life of their own.
fn delete_agent_state_for(ctx: &ReducerContext, task_id: i64) {
    for row in ctx
        .db
        .task_shells()
        .iter()
        .filter(|r| r.task_id == task_id)
        .collect::<Vec<_>>()
    {
        ctx.db.task_shells().delete(row);
    }
    for row in ctx
        .db
        .task_subagents()
        .iter()
        .filter(|r| r.task_id == task_id)
        .collect::<Vec<_>>()
    {
        ctx.db.task_subagents().delete(row);
    }
}

// -- Epics ------------------------------------------------------------------

/// Create an epic. Backlog, by `epics.allium: CreateEpic`.
#[spacetimedb::reducer]
pub fn create_epic(ctx: &ReducerContext, row: Epic) -> Result<(), String> {
    let parent = row.parent_epic_id;
    ctx.db.epics().insert(Epic { id: 0, ..row });
    // The new epic is a child of its parent and a parent has no children rule
    // it can break by gaining a backlog one — but the parent may have been
    // done, in which case it is not any more.
    recalculate_epic_chain(ctx, parent);
    Ok(())
}

/// Change some fields of an epic, then recalculate it and its ancestors.
///
/// The recalculation runs AFTER the patch, so a manual status move is what the
/// derivation sees as "current" — which is how `epics.allium`'s "manual
/// placements are preserved" rule and its two auto-transitions coexist.
#[spacetimedb::reducer]
pub fn patch_epic(ctx: &ReducerContext, id: i64, patch: EpicPatch) -> Result<(), String> {
    let Some(mut row) = ctx.db.epics().id().find(id) else {
        return Ok(());
    };
    let prior_status = row.status.clone();
    let was_child_of = row.parent_epic_id;
    let caller_named_a_completion = patch.completed_at.is_some();

    apply_patch!(
        row,
        patch,
        title,
        description,
        status,
        plan_path,
        sort_order,
        auto_dispatch,
        parent_epic_id,
        feed_command,
        feed_interval_secs,
        group_by_repo,
        feed_role,
        origin,
        feed_append_only,
        completed_at,
    );
    if !caller_named_a_completion && stamps_completion(&prior_status, &row.status) {
        row.completed_at = now(ctx);
    }
    row.updated_at = now(ctx);
    ctx.db.epics().id().update(row);

    recalculate_epic_chain(ctx, id);
    if was_child_of != 0 {
        recalculate_epic_chain(ctx, was_child_of);
    }
    Ok(())
}

/// Delete an epic, its sub-epics and their tasks.
///
/// Recursive, and the recursion is the point: an epic's subtree is not
/// reachable any other way once its root is gone, so leaving it would strand
/// every descendant on a board that cannot draw them.
#[spacetimedb::reducer]
pub fn delete_epic(ctx: &ReducerContext, id: i64) -> Result<(), String> {
    let Some(row) = ctx.db.epics().id().find(id) else {
        return Ok(());
    };
    let parent = row.parent_epic_id;
    delete_epic_subtree(ctx, id, 0);
    recalculate_epic_chain(ctx, parent);
    Ok(())
}

/// How deep a delete or a recalculation will walk before it gives up.
///
/// A cycle in `parent_epic_id` is unreachable through any writer here, so this
/// is not a correctness mechanism — it is the thing that turns a corrupt store
/// into a refused reducer rather than a wasm stack overflow, which takes the
/// whole module down rather than one call.
const MAX_EPIC_DEPTH: usize = 64;

fn delete_epic_subtree(ctx: &ReducerContext, id: i64, depth: usize) {
    if depth > MAX_EPIC_DEPTH {
        return;
    }
    for child in ctx
        .db
        .epics()
        .iter()
        .filter(|e| e.parent_epic_id == id)
        .collect::<Vec<_>>()
    {
        delete_epic_subtree(ctx, child.id, depth + 1);
    }
    for task in ctx
        .db
        .tasks()
        .iter()
        .filter(|t| t.epic_id == id)
        .collect::<Vec<_>>()
    {
        ctx.db.tasks().id().delete(task.id);
        delete_agent_state_for(ctx, task.id);
    }
    ctx.db.epics().id().delete(id);
}

/// Recalculate an epic's status and every ancestor's, from the WHOLE child set.
///
/// THE ONE PIECE OF LOGIC THE MIGRATION MOVED SERVER-SIDE, and the reason is
/// visible in the first two lines of the body: it reads every task and every
/// sub-epic whose parent is this epic, without reference to who owns them or
/// which board subscribes to them. No client can do that. A client deriving the
/// status from its own subscription derives it from a subset, two clients hold
/// different subsets, and the epic flips between their two correct-looking
/// answers for as long as both are up. See
/// `epics.allium: EpicStatusRecalculation` and
/// `sync.allium: TheStoreEnforcesTheSharedRules`.
///
/// Called by the mutations above rather than exposed as the only entry point,
/// so the recalculation is part of the same transaction as the change that
/// provoked it. An epic whose status lagged its children by one round trip
/// would be a board showing a column that is briefly wrong, on every machine.
#[spacetimedb::reducer]
pub fn recalculate_epic_status(ctx: &ReducerContext, epic_id: i64) -> Result<(), String> {
    recalculate_epic_chain(ctx, epic_id);
    Ok(())
}

fn recalculate_epic_chain(ctx: &ReducerContext, epic_id: i64) {
    // Zero is the absent epic, not epic zero. A task on somebody's user board
    // reaches here with it and there is nothing to recalculate.
    let mut next = epic_id;
    for _ in 0..MAX_EPIC_DEPTH {
        if next == 0 {
            return;
        }
        let Some(epic) = ctx.db.epics().id().find(next) else {
            return;
        };

        let children: Vec<String> = ctx
            .db
            .tasks()
            .iter()
            .filter(|t| t.epic_id == next)
            .map(|t| t.status)
            .chain(
                ctx.db
                    .epics()
                    .iter()
                    .filter(|e| e.parent_epic_id == next)
                    .map(|e| e.status),
            )
            .collect();

        if let Some(target) = derive_epic_status(&epic.status, &children) {
            let completed_at = if stamps_completion(&epic.status, target) {
                now(ctx)
            } else {
                // Left exactly as it was. The regression out of done does not
                // clear it: `completed_at` records the last completion, and
                // reopening work does not unmake one.
                epic.completed_at.clone()
            };
            ctx.db.epics().id().update(Epic {
                status: target.to_string(),
                completed_at,
                updated_at: now(ctx),
                ..epic.clone()
            });
        }

        // Upward, unconditionally rather than only when this epic changed. A
        // parent's derivation reads every child's CURRENT status, so an
        // unchanged child can still be the one that completes the parent —
        // the child that changed may be several levels down.
        next = epic.parent_epic_id;
    }
}

// -- Todos ------------------------------------------------------------------

#[spacetimedb::reducer]
pub fn create_todo(ctx: &ReducerContext, row: Todo) -> Result<(), String> {
    ctx.db.todos().insert(Todo { id: 0, ..row });
    Ok(())
}

#[spacetimedb::reducer]
pub fn patch_todo(ctx: &ReducerContext, id: i64, patch: TodoPatch) -> Result<(), String> {
    let Some(mut row) = ctx.db.todos().id().find(id) else {
        return Ok(());
    };
    apply_patch!(row, patch, title, done, sort_order, task_id, epic_id, parent_id, owner);
    ctx.db.todos().id().update(row);
    Ok(())
}

/// Delete a todo and everything nested under it.
///
/// A child todo whose parent is gone is unreachable on the board — nothing
/// draws an orphan — so leaving it would be leaving a row nobody can see or
/// remove.
#[spacetimedb::reducer]
pub fn delete_todo(ctx: &ReducerContext, id: i64) -> Result<(), String> {
    delete_todo_subtree(ctx, id, 0);
    Ok(())
}

/// Clear the finished todos on ONE person's list.
///
/// Scoped by owner, and it has to be: an unscoped sweep on a shared store would
/// clear every colleague's completed checklist from whichever board happened to
/// press the key.
#[spacetimedb::reducer]
pub fn delete_done_todos(ctx: &ReducerContext, owner: String) -> Result<(), String> {
    for row in ctx
        .db
        .todos()
        .iter()
        .filter(|t| t.done && t.owner == owner)
        .collect::<Vec<_>>()
    {
        delete_todo_subtree(ctx, row.id, 0);
    }
    Ok(())
}

fn delete_todo_subtree(ctx: &ReducerContext, id: i64, depth: usize) {
    if depth > MAX_EPIC_DEPTH {
        return;
    }
    for child in ctx
        .db
        .todos()
        .iter()
        .filter(|t| t.parent_id == id)
        .collect::<Vec<_>>()
    {
        delete_todo_subtree(ctx, child.id, depth + 1);
    }
    ctx.db.todos().id().delete(id);
}

// -- Repo configuration -----------------------------------------------------

/// Record a repo path, or touch the one already there.
///
/// Upsert by path rather than by id, because the path is what the domain calls
/// the same repo. Two rows for one path would give the verify command two
/// answers.
#[spacetimedb::reducer]
pub fn save_repo_path(ctx: &ReducerContext, path: String, last_used: String) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("repo path is empty".into());
    }
    match ctx.db.repo_paths().iter().find(|r| r.path == path) {
        Some(existing) => ctx.db.repo_paths().id().update(RepoPath {
            last_used,
            ..existing
        }),
        None => ctx.db.repo_paths().insert(RepoPath {
            id: 0,
            path,
            last_used,
            verify_command: String::new(),
        }),
    };
    Ok(())
}

#[spacetimedb::reducer]
pub fn delete_repo_path(ctx: &ReducerContext, path: String) -> Result<(), String> {
    for row in ctx
        .db
        .repo_paths()
        .iter()
        .filter(|r| r.path == path)
        .collect::<Vec<_>>()
    {
        ctx.db.repo_paths().id().delete(row.id);
    }
    Ok(())
}

/// Set or clear a repo's verify command.
///
/// `""` clears it. Newlines are refused here rather than at the caller, because
/// this is the one place every caller passes through — the command is run as a
/// single shell line, and a second line would run unannounced.
#[spacetimedb::reducer]
pub fn set_verify_command(
    ctx: &ReducerContext,
    path: String,
    command: String,
) -> Result<(), String> {
    if command.contains('\n') || command.contains('\r') {
        return Err("verify command must be a single line; chain steps with && or ;".into());
    }
    let Some(existing) = ctx.db.repo_paths().iter().find(|r| r.path == path) else {
        return Err(format!("no repo path {path}"));
    };
    ctx.db.repo_paths().id().update(RepoPath {
        verify_command: command,
        ..existing
    });
    Ok(())
}

#[spacetimedb::reducer]
pub fn record_base_branch(
    ctx: &ReducerContext,
    repo_path: String,
    branch: String,
    last_used: String,
) -> Result<(), String> {
    match ctx
        .db
        .repo_base_branches()
        .iter()
        .find(|r| r.repo_path == repo_path && r.branch == branch)
    {
        Some(existing) => ctx.db.repo_base_branches().id().update(RepoBaseBranch {
            last_used,
            ..existing
        }),
        None => ctx.db.repo_base_branches().insert(RepoBaseBranch {
            id: 0,
            repo_path,
            branch,
            last_used,
        }),
    };
    Ok(())
}

// -- Hosts and subscriptions ------------------------------------------------

/// Register or rename this machine in the host registry.
///
/// Upsert by host id. A machine that reconnects is the same machine, and a
/// second row for it would make `core.allium: ExactlyOneLocalHost` false from
/// every other board's point of view.
#[spacetimedb::reducer]
pub fn register_host(
    ctx: &ReducerContext,
    id: String,
    label: String,
    owner: String,
) -> Result<(), String> {
    if id.trim().is_empty() {
        return Err("host id is empty".into());
    }
    match ctx.db.hosts().iter().find(|h| h.id == id) {
        Some(existing) => ctx.db.hosts().id().update(Host {
            label,
            owner,
            ..existing
        }),
        None => ctx.db.hosts().insert(Host { id, label, owner }),
    };
    Ok(())
}

/// Follow an epic. Idempotent, by `sync.allium: SubscribeToEpic`.
#[spacetimedb::reducer]
pub fn subscribe_to_epic(
    ctx: &ReducerContext,
    subscriber: String,
    epic_id: i64,
) -> Result<(), String> {
    if subscriber.trim().is_empty() {
        return Err("cannot subscribe without an identity".into());
    }
    if ctx.db.epics().id().find(epic_id).is_none() {
        return Err(format!("no epic {epic_id}"));
    }
    let id = subscription_id(&subscriber, epic_id);
    // DO NOTHING on a repeat, not an update: every column of this row is part
    // of its own key, so there is nothing a second subscribe could refresh.
    if ctx.db.subscriptions().id().find(&id).is_none() {
        ctx.db.subscriptions().insert(Subscription {
            id,
            subscriber,
            epic_id,
        });
    }
    Ok(())
}

/// The derived key of a subscription row.
///
/// Must agree character for character with the SQLite side's
/// `db::queries::settings::subscription_id`, or the same person subscribing on
/// two backings produces two rows that are the same subscription.
fn subscription_id(subscriber: &str, epic_id: i64) -> String {
    format!("{subscriber}/{epic_id}")
}

/// Stop following an epic.
///
/// Unfollowing something unfollowed is a refusal rather than a no-op, the
/// asymmetry `sync.allium: UnsubscribeFromEpic` argues for: subscribing twice
/// expresses what the caller wanted, unsubscribing from nothing means they were
/// wrong about the state they were in.
#[spacetimedb::reducer]
pub fn unsubscribe_from_epic(
    ctx: &ReducerContext,
    subscriber: String,
    epic_id: i64,
) -> Result<(), String> {
    let id = subscription_id(&subscriber, epic_id);
    if ctx.db.subscriptions().id().find(&id).is_none() {
        return Err(format!("not subscribed to epic {epic_id}"));
    }
    ctx.db.subscriptions().id().delete(&id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The store's spelling of "absent", named so the assertions below read as
    /// the domain statements they are rather than as bare literals. See
    /// "Absence is a sentinel, not a null" in this file's header.
    const NO_EPIC: i64 = 0;
    const NO_OWNER: &str = "";

    /// A task with no epic sits on somebody's user board, and the row has to
    /// say whose. Nothing else in it can answer.
    #[test]
    fn an_epicless_task_needs_an_owner() {
        assert!(validate_task_ownership(NO_EPIC, NO_OWNER).is_err());
        assert!(validate_task_ownership(NO_EPIC, "user-1").is_ok());
    }

    /// The other arm, and the one that is easy to leave out: an owner on a task
    /// that already has an epic is REJECTED, not ignored. Two answers to
    /// "whose board is this?" place the same card on two boards, and neither
    /// reader looks wrong locally.
    #[test]
    fn an_epic_task_must_not_carry_an_owner() {
        assert!(validate_task_ownership(7, "user-1").is_err());
        assert!(validate_task_ownership(7, NO_OWNER).is_ok());
    }

    /// Whitespace is not an identity either.
    ///
    /// The empty string is the sentinel, so the first assertion is really about
    /// the sentinel doing its job. The second is the one that earns its keep: a
    /// space passes every test for presence — non-empty, non-sentinel — while
    /// answering nothing.
    #[test]
    fn an_empty_owner_is_not_an_owner() {
        assert!(validate_task_ownership(NO_EPIC, "").is_err());
        assert!(validate_task_ownership(NO_EPIC, "   ").is_err());
    }

    /// The message names which arm failed. A seeding run that trips this is
    /// reading a snapshot taken before the field existed, and "invalid task"
    /// would leave the operator no way to tell which of the two arms to fix.
    #[test]
    fn the_refusal_says_which_arm_failed() {
        let missing = validate_task_ownership(NO_EPIC, NO_OWNER).unwrap_err();
        assert!(missing.contains("no epic"), "{missing}");

        let surplus = validate_task_ownership(7, "user-1").unwrap_err();
        assert!(surplus.contains("epic"), "{surplus}");
        assert_ne!(missing, surplus);
    }

    // -----------------------------------------------------------------
    // Epic status derivation (epics.allium: EpicStatusRecalculation)
    // -----------------------------------------------------------------

    fn children(statuses: &[&str]) -> Vec<String> {
        statuses.iter().map(|s| (*s).to_string()).collect()
    }

    /// The forward auto-transition. Every active child done means the epic is
    /// done, and this is the only way an epic reaches done without somebody
    /// moving it.
    #[test]
    fn all_active_children_done_makes_the_epic_done() {
        assert_eq!(
            derive_epic_status("backlog", &children(&["done", "done"])),
            Some("done")
        );
    }

    /// The backward one. A done epic that gains an unfinished child is not
    /// done, and saying so is what keeps the column honest when work reopens.
    #[test]
    fn a_done_epic_with_an_unfinished_child_regresses_to_backlog() {
        assert_eq!(
            derive_epic_status("done", &children(&["done", "running"])),
            Some("backlog")
        );
    }

    /// Neither transition fires, so nothing is written. The distinction between
    /// "no change" and "write the same value" is not cosmetic here: a write
    /// would stamp `updated_at` and, on the done arm, `completed_at`.
    #[test]
    fn a_running_child_leaves_a_backlog_epic_alone() {
        assert_eq!(derive_epic_status("backlog", &children(&["running"])), None);
    }

    /// Manual placements survive. `running` and `review` are never derived, so
    /// an epic a person moved there stays there until they move it back.
    #[test]
    fn a_manual_placement_survives_a_recalculation() {
        assert_eq!(derive_epic_status("review", &children(&["running"])), None);
        assert_eq!(derive_epic_status("running", &children(&["backlog"])), None);
    }

    /// ...but the forward transition still overrides one. A review epic whose
    /// children all finished is finished.
    #[test]
    fn all_done_overrides_even_a_manual_placement() {
        assert_eq!(
            derive_epic_status("review", &children(&["done"])),
            Some("done")
        );
    }

    /// Archived is terminal, and it is checked FIRST. Without the ordering an
    /// archived epic whose children are all done would flip back to done the
    /// next time anything touched it.
    #[test]
    fn an_archived_epic_never_moves() {
        assert_eq!(derive_epic_status("archived", &children(&["done"])), None);
        assert_eq!(derive_epic_status("archived", &children(&[])), None);
    }

    /// No active children is NOT "all children done". An epic with nothing in
    /// it keeps whatever status it has — a freshly created one is backlog and
    /// must not be born done.
    #[test]
    fn an_epic_with_no_active_children_keeps_its_status() {
        assert_eq!(derive_epic_status("backlog", &children(&[])), None);
        assert_eq!(derive_epic_status("running", &children(&[])), None);
    }

    /// Archived children do not count, on either side. One live child among
    /// archived ones decides alone, and archived ones cannot complete an epic.
    #[test]
    fn archived_children_are_not_children() {
        assert_eq!(
            derive_epic_status("backlog", &children(&["done", "archived"])),
            Some("done")
        );
        assert_eq!(
            derive_epic_status("backlog", &children(&["archived"])),
            None
        );
    }

    /// THE TWO-HOST CASE, as a property of the function rather than of a
    /// deployment. Each board sees a subset; the store sees the union. The
    /// subsets disagree and the union is right, which is the whole argument for
    /// deriving this at the store (sync.allium: TheStoreEnforcesTheSharedRules).
    #[test]
    fn two_partial_views_disagree_and_the_whole_view_decides() {
        let host_a_sees = children(&["done"]);
        let host_b_sees = children(&["running"]);
        let the_store_sees = children(&["done", "running"]);

        assert_eq!(derive_epic_status("backlog", &host_a_sees), Some("done"));
        assert_eq!(derive_epic_status("backlog", &host_b_sees), None);
        assert_eq!(derive_epic_status("backlog", &the_store_sees), None);

        // And the same disagreement from the other side: a done epic that host
        // A would leave alone and host B would regress.
        assert_eq!(derive_epic_status("done", &host_a_sees), None);
        assert_eq!(derive_epic_status("done", &host_b_sees), Some("backlog"));
        assert_eq!(derive_epic_status("done", &the_store_sees), Some("backlog"));
    }

    /// An unknown status is not a silent "not done". A board running a newer
    /// binary can write a status this module does not know, and treating it as
    /// unfinished would hold an epic open forever with nothing saying why.
    #[test]
    fn an_unknown_child_status_is_refused_rather_than_guessed() {
        assert_eq!(
            derive_epic_status("backlog", &children(&["done", "quantum"])),
            None
        );
    }

    // -----------------------------------------------------------------
    // Timestamps
    // -----------------------------------------------------------------

    /// The store's instant, in SQLite's spelling.
    ///
    /// Every timestamp column in this module is TEXT in the format the SQLite
    /// side writes, because the two stores' rows have to compare equal — the
    /// schema parity test and the seed round-trip both depend on it. A format
    /// that drifted by one character would pass every test that reads a
    /// timestamp back through the same parser and fail the one that compares
    /// stores.
    #[test]
    fn a_timestamp_is_formatted_the_way_sqlite_writes_one() {
        // 2026-09-19T12:34:56.789Z
        assert_eq!(
            format_timestamp_micros(1_789_821_296_789_000),
            "2026-09-19 12:34:56.789"
        );
    }

    /// Milliseconds, always three of them, padded. A truncating formatter that
    /// dropped a trailing zero would sort wrong as text, which is exactly how
    /// the Done column reads these.
    #[test]
    fn the_milliseconds_are_always_three_digits() {
        assert_eq!(
            format_timestamp_micros(1_789_821_296_700_000),
            "2026-09-19 12:34:56.700"
        );
        assert_eq!(
            format_timestamp_micros(1_789_821_296_000_000),
            "2026-09-19 12:34:56.000"
        );
    }

    /// Sub-millisecond precision is dropped rather than rounded, matching the
    /// SQLite side's `trunc_subsecs(3)`.
    #[test]
    fn sub_millisecond_precision_is_truncated() {
        assert_eq!(
            format_timestamp_micros(1_789_821_296_789_999),
            "2026-09-19 12:34:56.789"
        );
    }

    // -----------------------------------------------------------------
    // completed_at
    // -----------------------------------------------------------------

    /// Only the transition INTO done stamps it, mirroring tasks.allium's
    /// ConfirmDone.
    #[test]
    fn only_entering_done_stamps_a_completion() {
        assert!(stamps_completion("backlog", "done"));
        assert!(!stamps_completion("done", "backlog"));
        assert!(!stamps_completion("backlog", "running"));
    }

    /// The regression does not clear it. `completed_at` records the last
    /// completion and a reopening does not unmake one.
    #[test]
    fn a_regression_does_not_clear_the_completion() {
        assert!(!stamps_completion("done", "backlog"));
    }

    /// A blank task is the row `burn_id_sequence` throws away and the row
    /// `probe_generated_task_id` writes. It must satisfy the invariant or
    /// neither works: a blank has no epic, so it needs an owner. This is what
    /// makes SCRATCH_OWNER load-bearing rather than decorative.
    #[test]
    fn the_blank_task_the_burn_throws_away_satisfies_the_invariant() {
        let blank = blank_task();
        assert!(validate_task_ownership(blank.epic_id, &blank.owner).is_ok());
    }
}
