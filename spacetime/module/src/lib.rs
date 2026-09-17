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
    pub worktree: Option<String>,
    pub tmux_window: Option<String>,
    pub plan_path: Option<String>,
    pub epic_id: Option<i64>,
    pub sub_status: String,
    pub tag: Option<String>,
    pub sort_order: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
    pub base_branch: String,
    pub external_id: Option<String>,
    pub labels: String,
    pub last_pre_tool_use_at: Option<String>,
    pub last_notification_at: Option<String>,
    pub wrap_up_mode: Option<String>,
    pub url: Option<String>,
    pub url_type: Option<String>,
    pub pr_learnings_gate_shown_at: Option<String>,
    pub auto_run_plan: bool,
    pub live_subagents: i64,
    pub stop_pending: bool,
    pub stop_pending_at: Option<String>,
    pub live_shells: i64,
    pub oldest_live_shell_started_at: Option<String>,
    pub last_peer_message_sent_at: Option<String>,
    pub last_peer_message_received_at: Option<String>,
    pub phoenix: bool,
    /// The machine holding this task's worktree. Null means no machine holds
    /// one — coupled to `worktree`, not to `status`.
    pub host: Option<String>,
    /// The person whose user board this task sits on, set exactly when the task
    /// has no epic. Rationale and both refusal arms:
    /// `core.allium: OwnerTracksUserBoardTask`. Enforced by [`write_task`].
    ///
    /// The default is null even though an epic-less task with a null owner does
    /// not satisfy the invariant, and that is deliberate: a migration cannot
    /// invent a person. Rows already in the store keep a null until the seeding
    /// client backfills the seeding user onto them
    /// (`spacetime-seed.allium: BackfillTaskOwner`). No new row can widen the
    /// gap, because every writer goes through [`write_task`].
    #[default(None)]
    pub owner: Option<String>,
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
    pub plan_path: Option<String>,
    pub sort_order: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
    pub auto_dispatch: bool,
    pub parent_epic_id: Option<i64>,
    pub feed_command: Option<String>,
    pub feed_interval_secs: Option<i64>,
    pub group_by_repo: bool,
    pub feed_role: String,
    pub origin: String,
    pub feed_append_only: bool,
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
    pub task_id: Option<i64>,
    pub epic_id: Option<i64>,
    pub parent_id: Option<i64>,
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
    pub verify_command: Option<String>,
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
/// `core.allium: ExactlyOneHost` still says one row, and still means it: it is
/// a statement about the LOCAL store, which Phase 1 does not change. Relaxing
/// it to a registry belongs with the code that first writes a second row, in
/// Phase 4.
#[spacetimedb::table(accessor = hosts, public)]
#[derive(Clone, Debug)]
pub struct Host {
    #[primary_key]
    pub id: String,
    pub label: Option<String>,
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
    validate_task_ownership(row.epic_id, row.owner.as_deref())
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
        worktree: None,
        tmux_window: None,
        plan_path: None,
        epic_id: None,
        sub_status: "none".into(),
        tag: None,
        sort_order: None,
        created_at: String::new(),
        updated_at: String::new(),
        base_branch: "main".into(),
        external_id: None,
        labels: "[]".into(),
        last_pre_tool_use_at: None,
        last_notification_at: None,
        wrap_up_mode: None,
        url: None,
        url_type: None,
        pr_learnings_gate_shown_at: None,
        auto_run_plan: false,
        live_subagents: 0,
        stop_pending: false,
        stop_pending_at: None,
        live_shells: 0,
        oldest_live_shell_started_at: None,
        last_peer_message_sent_at: None,
        last_peer_message_received_at: None,
        phoenix: false,
        host: None,
        // A blank has no epic, so the invariant requires an owner. This one is
        // never a real person: the row exists to be generated and thrown away
        // by `burn_id_sequence`, and the only copy that outlives a call is the
        // `probe_generated_task_id` row an operator deletes by hand.
        owner: Some(SCRATCH_OWNER.into()),
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
        plan_path: None,
        sort_order: None,
        created_at: String::new(),
        updated_at: String::new(),
        auto_dispatch: false,
        parent_epic_id: None,
        feed_command: None,
        feed_interval_secs: None,
        group_by_repo: false,
        feed_role: "none".into(),
        origin: "manual".into(),
        feed_append_only: false,
    }
}

fn blank_todo() -> Todo {
    Todo {
        id: 0,
        title: String::new(),
        done: false,
        sort_order: 0,
        created_at: String::new(),
        task_id: None,
        epic_id: None,
        parent_id: None,
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
        verify_command: None,
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
pub fn validate_task_ownership(epic_id: Option<i64>, owner: Option<&str>) -> Result<(), String> {
    // Blank is not absent for the purposes of this check. An empty or
    // whitespace-only string satisfies "is not null" while answering nothing,
    // which a null check on its own cannot see.
    let owner = owner.filter(|o| !o.trim().is_empty());
    match (epic_id, owner) {
        (None, None) => Err("a task with no epic sits on a user board and needs an owner".into()),
        (Some(epic_id), Some(owner)) => Err(format!(
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A task with no epic sits on somebody's user board, and the row has to
    /// say whose. Nothing else in it can answer.
    #[test]
    fn an_epicless_task_needs_an_owner() {
        assert!(validate_task_ownership(None, None).is_err());
        assert!(validate_task_ownership(None, Some("user-1")).is_ok());
    }

    /// The other arm, and the one that is easy to leave out: an owner on a task
    /// that already has an epic is REJECTED, not ignored. Two answers to
    /// "whose board is this?" place the same card on two boards, and neither
    /// reader looks wrong locally.
    #[test]
    fn an_epic_task_must_not_carry_an_owner() {
        assert!(validate_task_ownership(Some(7), Some("user-1")).is_err());
        assert!(validate_task_ownership(Some(7), None).is_ok());
    }

    /// An empty string is not an identity. Accepting it would satisfy the
    /// "required" arm while answering nothing, which is the failure mode a
    /// null check on its own cannot see.
    #[test]
    fn an_empty_owner_is_not_an_owner() {
        assert!(validate_task_ownership(None, Some("")).is_err());
        assert!(validate_task_ownership(None, Some("   ")).is_err());
    }

    /// The message names which arm failed. A seeding run that trips this is
    /// reading a snapshot taken before the field existed, and "invalid task"
    /// would leave the operator no way to tell which of the two arms to fix.
    #[test]
    fn the_refusal_says_which_arm_failed() {
        let missing = validate_task_ownership(None, None).unwrap_err();
        assert!(missing.contains("no epic"), "{missing}");

        let surplus = validate_task_ownership(Some(7), Some("user-1")).unwrap_err();
        assert!(surplus.contains("epic"), "{surplus}");
        assert_ne!(missing, surplus);
    }

    /// A blank task is the row `burn_id_sequence` throws away and the row
    /// `probe_generated_task_id` writes. It must satisfy the invariant or
    /// neither works: a blank has no epic, so it needs an owner. This is what
    /// makes SCRATCH_OWNER load-bearing rather than decorative.
    #[test]
    fn the_blank_task_the_burn_throws_away_satisfies_the_invariant() {
        let blank = blank_task();
        assert!(validate_task_ownership(blank.epic_id, blank.owner.as_deref()).is_ok());
    }
}
