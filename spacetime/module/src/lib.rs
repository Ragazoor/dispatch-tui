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

/// The SQLite `user_version` this module was cut from.
///
/// **Nothing checks a restore against it any more, and nothing should.** It was
/// the number a restore compared a snapshot's `schema_version` against, and it
/// was wrong in both directions: hand-written and linked to no migration list,
/// so it went stale unnoticed; and a mirror of the *whole* database's counter,
/// so a migration touching only local tables invalidated every earlier backup
/// though no shared column had moved. A restore now compares the shared tables'
/// COLUMNS, which both sides can derive — see `spacetime-seed.allium`'s
/// `RefuseMismatchedSchema`.
///
/// What is left is provenance: the number is stamped into the database so an
/// operator can see which SQLite schema the rows were cut from. It survives
/// only because dropping a SpacetimeDB table is not an automigratable change,
/// and `tests/spacetime_module.rs` both requires the automigration to succeed
/// and uses this very row as its probe for "migrated rather than rebuilt".
/// Phase 1 owns the module's schema and can retire it with a fresh publish.
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

/// One row, holding [`SCHEMA_VERSION`], readable over SQL without the module
/// having to expose a reducer that returns a value.
///
/// Not a shared table and deliberately absent from the snapshot: it describes
/// the store rather than the domain.
///
/// **No client reads it any more.** A restore compares column sets instead
/// (`spacetime-seed.allium`: `RefuseMismatchedSchema`), so this is provenance
/// an operator can query, not a gate. See [`SCHEMA_VERSION`] for why it is
/// still here.
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
    /// Indexed: `recalculate_epic_chain` and `delete_epic_subtree` both ask
    /// "which tasks belong to this epic?" at every level of an ancestry walk.
    /// Unindexed that is a scan of every task in the STORE per level, which on
    /// one machine was a scan of one person's board and on a shared one is not.
    #[index(btree)]
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
    // DEAD COLUMNS, kept for the reason `docs/specs/spacetime-seed.allium`
    // gives: this store's migrations are append-only, and removing a column
    // is refused outright — the only way out is dump/rebuild/restore, which
    // is out of scope for retiring one feature. #4965 dropped the
    // shell-tracking feature these backed (see docs/specs/agent-health.allium
    // and core.allium's Task entity for why); no reducer writes them anymore
    // and nothing reads them. Left at whatever value automigration carried
    // them forward with.
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
    /// Who created this task; stamped once and never rewritten, regardless of
    /// epic membership. NOT `owner` — `owner` tracks board placement and is
    /// blanked when the task joins an epic; this survives that. Exists so
    /// `sync.allium`'s `own_creations` subscription can find a task its
    /// creator just made no matter which epic it landed in
    /// (`core.allium: Task.created_by`). Appended last for the same reason
    /// `owner` and `completed_at` are: appending is the only automigratable
    /// position. `""` for a row created before this column existed, and never
    /// backfilled.
    #[default("")]
    pub created_by: String,
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
    /// Indexed, for the same walk `Task::epic_id` serves: the sub-epic half of
    /// "which children does this epic have?".
    #[index(btree)]
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
    /// Who created this epic; stamped once and never rewritten. The epic twin
    /// of [`Task::created_by`] — an epic has no `owner` column at all, so this
    /// is the only way `sync.allium`'s `own_creations` subscription can find
    /// an epic its creator just made, before anyone (including them) follows
    /// it. `""` for a row created before this column existed, or created by an
    /// unidentified process (e.g. a feed's grouped sub-epics), and never
    /// backfilled.
    #[default("")]
    pub created_by: String,
}

/// DEAD TABLE, kept for the same reason `TaskShell` is: this store's
/// migrations are append-only and there is no supported way to drop a table
/// short of dump/rebuild/restore. #4970 removed the TODO overlay this backed;
/// only `seed_todos` still writes it, so a restore of an old snapshot lands.
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
    #[index(btree)]
    #[default(0)]
    pub parent_id: i64,
    #[index(btree)]
    #[default("")]
    pub owner: String,
}

#[spacetimedb::table(accessor = task_watchers, public)]
#[derive(Clone, Debug)]
pub struct TaskWatcher {
    #[primary_key]
    #[auto_inc]
    pub id: i64,
    /// Both sides indexed: deleting a task removes the watches pointing at it
    /// AND the ones it holds, which is two lookups rather than one scan.
    #[index(btree)]
    pub watcher_task_id: i64,
    #[index(btree)]
    pub target_task_id: i64,
    pub created_at: String,
}

/// core.allium: `RetiredFeedItem`. One row per (feed_epic_id, external_id) a
/// human deleted from a feed's subtree, so the feed's next cycle does not put
/// the same item straight back (`feeds.allium: IngestSkipsRetiredFeedItems`).
///
/// At most one row per (feed_epic_id, external_id)
/// (`core/RetiredFeedItem: UniqueRetiredFeedItemPerFeed`) — enforced the same
/// way [`create_repo_group_sub_epic`] documents: reducers run to completion
/// one at a time, so a check-then-insert here has no race window and needs no
/// declared unique index.
#[spacetimedb::table(accessor = retired_feed_items, public)]
#[derive(Clone, Debug)]
pub struct RetiredFeedItem {
    #[primary_key]
    #[auto_inc]
    pub id: i64,
    /// Every retirement lookup and the whole-epic drop/cascade are keyed on
    /// this, never on `external_id` alone.
    #[index(btree)]
    pub feed_epic_id: i64,
    pub external_id: String,
    pub retired_at: String,
}

/// DEAD TABLE, kept for the same reason `Task.live_shells`/
/// `Task.oldest_live_shell_started_at` are: this store's migrations are
/// append-only and there is no supported way to drop a table short of
/// dump/rebuild/restore. #4965 retired the shell-tracking feature this
/// backed; no reducer inserts into it anymore.
#[spacetimedb::table(accessor = task_shells, public)]
#[derive(Clone, Debug)]
pub struct TaskShell {
    #[index(btree)]
    pub task_id: i64,
    pub shell_id: String,
    pub session_id: String,
    pub started_at: String,
}

#[spacetimedb::table(accessor = task_subagents, public)]
#[derive(Clone, Debug)]
pub struct TaskSubagent {
    /// Indexed: `delete_agent_state_for` runs once per task, and once per task
    /// in a deleted epic's whole subtree — so an unindexed scan here multiplies
    /// by the subtree size.
    #[index(btree)]
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

/// A named value scoped to one machine (`docs/specs/settings.allium`).
///
/// `id` is derived (`"{host}/{key}"`), the same trick `Subscription` below
/// uses — the uniqueness that matters is over the pair, and a single-column
/// primary key over the derived pair is how a store that indexes one column at
/// a time expresses it. No `#[auto_inc]` anywhere on this table: the pair is
/// known to the caller before the call, so there is no id to burn on restore
/// and no reducer-return-value gap to design around (contrast `Epic`
/// above).
///
/// host_id, host_label, the stored UserIdentity and its credential are NOT
/// rows here despite having been stored the same way before this table
/// existed — see settings.allium's Excludes.
#[spacetimedb::table(accessor = settings, public)]
#[derive(Clone, Debug)]
pub struct Setting {
    #[primary_key]
    pub id: String,
    #[index(btree)]
    pub host: String,
    pub key: String,
    pub value: String,
}

/// DEAD TABLE, kept for the same reason `TaskShell` and `Todo` are. #4972
/// removed saved repo-filter presets, the feature this backed. SpacetimeDB
/// does automigrate a removed table, but only an empty one — a store holding
/// any preset row refuses the publish ("table contains data"), and the
/// removal disconnects every client. Only `seed_filter_presets` still writes
/// it, so a restore of an old snapshot lands.
///
/// It was a named repo-filter combination, scoped to one machine, keyed on
/// `(host, name)`; `repo_paths` a JSON-encoded array the module never parsed.
#[spacetimedb::table(accessor = filter_presets, public)]
#[derive(Clone, Debug)]
pub struct FilterPreset {
    #[primary_key]
    pub id: String,
    #[index(btree)]
    pub host: String,
    pub name: String,
    pub repo_paths: String,
    pub mode: String,
}

/// A recorded knowledge-base entry (`docs/specs/learnings.allium`), moved here
/// in Phase 10 (task #4914). Never actually per-machine data — it was only
/// ever filed that way — so unlike `Setting`/`FilterPreset` above, nothing on
/// this table is scoped by `host`. It is subscribed to unconditionally, the
/// same way `repo_paths` is: a learning's visibility is governed entirely by
/// its own `scope`/`scope_ref`, not by which machine recorded it or which
/// machine is asking.
///
/// No column here needs the `""`/`0` sentinel treatment `Task`/`Epic`
/// use for their optional columns: that trick exists only because a
/// subscription's `WHERE` clause cannot test a SATS sum type, and no
/// subscription ever filters on any column of this table. `detail`,
/// `scope_ref`, `source_task_id` and `last_upvoted_at` stay genuine
/// `Option<_>`, matching SQLite's nullability exactly.
#[spacetimedb::table(accessor = learnings, public)]
#[derive(Clone, Debug)]
pub struct Learning {
    #[primary_key]
    #[auto_inc]
    pub id: i64,
    pub kind: String,
    pub summary: String,
    pub detail: Option<String>,
    pub scope: String,
    pub scope_ref: Option<String>,
    /// JSON-encoded array, opaque to this module — the same convention as
    /// `Task::labels` and `FilterPreset::repo_paths`.
    pub tags: String,
    pub status: String,
    /// The task whose agent proposed this entry, or `None` for a
    /// human-authored one. `SET NULL` on that task's delete
    /// (`core.allium: Learning.source_task`) — see [`delete_task`]'s cascade.
    pub source_task_id: Option<i64>,
    pub upvote_count: i64,
    pub last_upvoted_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// `bincode`-free: four little-endian bytes per `f32`, produced by
    /// `src/service/embeddings.rs::serialize_embedding` and never parsed by
    /// this module — the same opaque-blob treatment as `tags` above, just
    /// binary rather than JSON. `None` until the embedding backfill computes
    /// one (`docs/specs/learnings.allium`'s embedding backfill note).
    pub embedding: Option<Vec<u8>>,
}

/// One row per surfacing event of a learning to a task
/// (`core.allium: Retrieval`). Moved here alongside `Learning` in Phase 10 —
/// `rate_learning`'s retrieval-precondition check (`RateLearningViaMcp`) has
/// to see every host's retrievals for a task, not only the recording host's
/// own, so this is unconditionally subscribed to as well.
///
/// `learning_verdicts` has no table here or in SQLite: migration v74 dropped
/// it, and a verdict has never been more than an in-flight effect on
/// `Learning.upvote_count` — see [`apply_learning_verdict`].
#[spacetimedb::table(accessor = learning_retrievals, public)]
#[derive(Clone, Debug)]
pub struct LearningRetrieval {
    #[primary_key]
    #[auto_inc]
    pub id: i64,
    #[index(btree)]
    pub task_id: i64,
    #[index(btree)]
    pub learning_id: i64,
    pub source: String,
    pub retrieved_at: String,
}

/// One recorded telemetry event (`docs/plans/2026-09-17-spacetimedb-migration-plan.md`'s
/// Phase 11, task #4915) — append-only, with no user-observable rule beyond
/// "recorded". Unconditionally subscribed, the same as `learnings`: nothing
/// here is scoped by owner or host, and no subscription ever filters by any
/// of its columns, so no column needs the `""`/`0` sentinel treatment.
///
/// `task_usage` has no table here: it was removed outright — table, model and
/// MCP tool — in an unrelated change well before this migration reached it,
/// so there was nothing left to move.
#[spacetimedb::table(accessor = usage_events, public)]
#[derive(Clone, Debug)]
pub struct UsageEvent {
    #[primary_key]
    #[auto_inc]
    pub id: i64,
    pub recorded_at: String,
    pub category: String,
    pub action: String,
    pub detail: Option<String>,
    pub actor: String,
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

/// The single host allowed to run recurring background polling for a scope
/// with no natural owner of its own — a feed epic (`Epic` carries no `host`
/// column) or a host-less review task (`Task.host = ""`).
/// `core.allium: PollOwner`.
///
/// Permanent by design: no expiry column, no renewal reducer. `claim_*_owner`
/// fills an ABSENT row; `override_*_owner` unconditionally reassigns an
/// EXISTING one — see both below. Both are find-or-create by
/// `(scope, scope_id)`, race-free by construction the same way
/// `create_repo_group_sub_epic` is: reducers run one at a time, so the lookup
/// each does IS the whole safety argument.
///
/// New in this migration, so it starts at the end of the column order like
/// every other addition here — but unlike `Task`/`Epic`, this table has no
/// SQLite counterpart at all, appended or otherwise: it only ever gains rows
/// once a shared store exists for two hosts to contend a claim over, so a
/// fresh local dump legitimately has nothing to contribute
/// (`src/spacetime/dump.rs::Source::Empty`).
#[spacetimedb::table(accessor = poll_owners, public)]
#[derive(Clone, Debug)]
pub struct PollOwner {
    #[primary_key]
    #[auto_inc]
    pub id: i64,
    /// `"task"` or `"epic"` — see `POLL_SCOPE_TASK`/`POLL_SCOPE_EPIC`.
    /// Caller-supplied, but validated at the reducer boundary by
    /// `require_poll_scope` before any row is touched, so a typo'd scope is
    /// rejected rather than silently creating an orphaned row no consuming
    /// rule ever looks for.
    pub scope: String,
    /// The Task or Epic id this row names, per `scope`. No foreign key,
    /// matching how `TaskWatcher`'s watcher/target ids are stored.
    #[index(btree)]
    pub scope_id: i64,
    /// The owning `Host.id`.
    pub host: String,
    pub claimed_at: String,
}

const POLL_SCOPE_TASK: &str = "task";
const POLL_SCOPE_EPIC: &str = "epic";

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
        // Found in passing while adding the two arms below: `poll_owners` DOES
        // generate ids (`SharedTable::id_column` says so, and
        // `restore.rs::burn_id_sequences` filters on exactly that), but had no
        // arm here — a restore carrying any poll_owners row would call this
        // with table="poll_owners" and hit the `unknown table` error below.
        // Pre-existing since Phase 7 (task #4865); fixed here rather than only
        // reported, since it is the same one-line shape as every other arm.
        "poll_owners" => burn_table!(poll_owners, blank_poll_owner()),
        "learnings" => burn_table!(learnings, blank_learning()),
        "learning_retrievals" => burn_table!(learning_retrievals, blank_learning_retrieval()),
        "usage_events" => burn_table!(usage_events, blank_usage_event()),
        // `task_shells`, `task_subagents`, `hosts`, `subscriptions`, `settings`
        // and `filter_presets` generate no ids, so there is nothing to burn.
        // Accepted rather than rejected so a caller can loop over every shared
        // table without a special case.
        "task_shells" | "task_subagents" | "hosts" | "subscriptions" | "settings"
        | "filter_presets" => {}
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
/// particular. There is no `archived` any more — `epics.allium:
/// ArchivedStatusMigration` retired it (task #4971): a finished epic either
/// stays in `done` or is deleted.
const DONE: &str = "done";
const BACKLOG: &str = "backlog";
const KNOWN_STATUSES: [&str; 4] = [BACKLOG, "running", "review", DONE];

/// Derive an epic's status from its children's, or `None` for "leave it alone".
///
/// The whole of `epics.allium: EpicStatusRecalculation`'s derivation, as a pure
/// function over the child statuses. It is pure on purpose: this is the one
/// piece of logic the migration moved server-side, and the argument for moving
/// it is about WHICH children are visible rather than about how the answer is
/// computed — so the computation is testable without a store, and the
/// visibility is what the reducer below supplies.
///
/// `children` carries every child's status, tasks and sub-epics alike.
///
/// `None` means no write. That is distinct from writing the same value back: a
/// write stamps `updated_at`, and on the forward arm `completed_at` too.
pub fn derive_epic_status(current: &str, children: &[String]) -> Option<&'static str> {
    // An unknown status anywhere is a refusal, not a guess. The realistic
    // producer is a board running a newer binary than this module, and both
    // guesses are wrong in a way nobody can see: treating it as done finishes
    // an epic that is not finished, and treating it as unfinished holds one
    // open forever. Declining to write leaves the epic where it is and leaves
    // the next recalculation — by then perhaps against an updated module — free
    // to get it right.
    if children
        .iter()
        .any(|s| !KNOWN_STATUSES.contains(&s.as_str()))
    {
        return None;
    }

    // No children is NOT all-done. A freshly created epic has none and must
    // not be born done.
    if children.is_empty() {
        return None;
    }
    if children.iter().all(|s| s == DONE) {
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
pub fn stamps_completion(prior: &str, next: &str) -> bool {
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
    // A TASK BORN IN DONE IS A COMPLETED TASK. `stamps_completion` covers every
    // TRANSITION into done, but a create is not a transition, and a Done card
    // with no `completed_at` sorts to the bottom of the column forever
    // (`board-layout.allium`, "Done Column Ordering"). The SQLite side stamps
    // it in `insert_task_row` for the same reason. Nothing creates a Done task
    // today; this is here so that whatever does first does not have to know.
    let row = if row.status == DONE && row.completed_at.is_empty() {
        Task {
            completed_at: now(ctx),
            ..row
        }
    } else {
        row
    };
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
        created_by: String::new(),
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
        created_by: String::new(),
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

fn blank_poll_owner() -> PollOwner {
    PollOwner {
        id: 0,
        scope: String::new(),
        scope_id: 0,
        host: String::new(),
        claimed_at: String::new(),
    }
}

fn blank_learning() -> Learning {
    Learning {
        id: 0,
        kind: String::new(),
        summary: String::new(),
        detail: None,
        scope: String::new(),
        scope_ref: None,
        tags: String::new(),
        status: String::new(),
        source_task_id: None,
        upvote_count: 0,
        last_upvoted_at: None,
        created_at: String::new(),
        updated_at: String::new(),
        embedding: None,
    }
}

fn blank_learning_retrieval() -> LearningRetrieval {
    LearningRetrieval {
        id: 0,
        task_id: 0,
        learning_id: 0,
        source: String::new(),
        retrieved_at: String::new(),
    }
}

fn blank_usage_event() -> UsageEvent {
    UsageEvent {
        id: 0,
        recorded_at: String::new(),
        category: String::new(),
        action: String::new(),
        detail: None,
        actor: String::new(),
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
        (false, false) => Err("a task with no epic sits on a user board and needs an owner".into()),
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

/// Always inserts zero rows in production — a fresh local dump never has any
/// `PollOwner` claims to seed (see the table's own doc comment) — but kept as
/// a genuine find-or-update-or-insert, not a stub, so a future dump-from-a-
/// live-shared-store path is not blocked on this reducer being rewritten.
#[spacetimedb::reducer]
pub fn seed_poll_owners(ctx: &ReducerContext, rows: Vec<PollOwner>) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_poll_owners needs each row's real id".into());
        }
        if ctx.db.poll_owners().id().find(row.id).is_some() {
            ctx.db.poll_owners().id().update(row);
        } else {
            ctx.db.poll_owners().insert(row);
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

#[spacetimedb::reducer]
pub fn seed_settings(ctx: &ReducerContext, rows: Vec<Setting>) -> Result<(), String> {
    for row in rows {
        if row.id.is_empty() {
            return Err("a setting needs its derived id".into());
        }
        if ctx.db.settings().id().find(row.id.clone()).is_some() {
            ctx.db.settings().id().update(row);
        } else {
            ctx.db.settings().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_filter_presets(ctx: &ReducerContext, rows: Vec<FilterPreset>) -> Result<(), String> {
    for row in rows {
        if row.id.is_empty() {
            return Err("a filter preset needs its derived id".into());
        }
        if ctx.db.filter_presets().id().find(row.id.clone()).is_some() {
            ctx.db.filter_presets().id().update(row);
        } else {
            ctx.db.filter_presets().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_learnings(ctx: &ReducerContext, rows: Vec<Learning>) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_learnings needs each learning's real id".into());
        }
        if ctx.db.learnings().id().find(row.id).is_some() {
            ctx.db.learnings().id().update(row);
        } else {
            ctx.db.learnings().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_learning_retrievals(
    ctx: &ReducerContext,
    rows: Vec<LearningRetrieval>,
) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_learning_retrievals needs each row's real id".into());
        }
        if ctx.db.learning_retrievals().id().find(row.id).is_some() {
            ctx.db.learning_retrievals().id().update(row);
        } else {
            ctx.db.learning_retrievals().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_usage_events(ctx: &ReducerContext, rows: Vec<UsageEvent>) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_usage_events needs each row's real id".into());
        }
        if ctx.db.usage_events().id().find(row.id).is_some() {
            ctx.db.usage_events().id().update(row);
        } else {
            ctx.db.usage_events().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_retired_feed_items(
    ctx: &ReducerContext,
    rows: Vec<RetiredFeedItem>,
) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_retired_feed_items needs each row's real id".into());
        }
        if ctx.db.retired_feed_items().id().find(row.id).is_some() {
            ctx.db.retired_feed_items().id().update(row);
        } else {
            ctx.db.retired_feed_items().insert(row);
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

/// Apply a task patch. Extracted from the reducer so it is testable without a
/// store — see the macro's note about what it does not guarantee.
pub fn apply_task_patch(row: &mut Task, patch: TaskPatch) {
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
        last_peer_message_sent_at,
        last_peer_message_received_at,
        phoenix,
        host,
        owner,
        completed_at,
    );
}

/// Apply an epic patch. See [`apply_task_patch`].
pub fn apply_epic_patch(row: &mut Epic, patch: EpicPatch) {
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

    apply_task_patch(&mut row, patch);

    // The completion stamp is the store's, not the caller's, unless the caller
    // named one explicitly. A client that computed it from its own clock would
    // make the Done column's ordering depend on whose laptop was fast.
    if !caller_named_a_completion && stamps_completion(&prior_status, &row.status) {
        row.completed_at = now(ctx);
    }
    row.updated_at = now(ctx);

    let now_in_epic = moved_to_epic.unwrap_or(was_in_epic);
    let status_changed = prior_status != row.status;
    write_task(ctx, row)?;

    // ONLY WHEN THE DERIVATION'S INPUTS MOVED. `derive_epic_status` reads the
    // child statuses and nothing else, so a patch that changes a url, a
    // sub-status or a timestamp cannot change any ancestor's answer — and
    // `patch_task` is the most frequent mutation on the board. Recalculating
    // unconditionally walked the whole ancestry, scanning tasks and epics at
    // each level, for every keystroke-driven write.
    //
    // BOTH epics when it moved, and in that order. A task leaving one epic
    // makes it possibly complete and makes the other possibly incomplete;
    // recalculating only the destination leaves the source stuck open.
    if status_changed || now_in_epic != was_in_epic {
        recalculate_epic_chain(ctx, was_in_epic);
        if now_in_epic != was_in_epic {
            recalculate_epic_chain(ctx, now_in_epic);
        }
    }
    Ok(())
}

/// Walk `epic_id`'s ancestry, itself first, for the nearest epic carrying a
/// `feed_command` — the same key `delete_task`, `delete_epic`'s retirement
/// pass and `upsert_feed_tasks_inner` all resolve against. Mirrors
/// `src/db/queries/tasks.rs`'s recursive `nearest_feed_epic` CTE
/// (`core/Epic.nearest_feed_epic`).
fn nearest_feed_epic(ctx: &ReducerContext, epic_id: i64) -> Option<i64> {
    let mut next = epic_id;
    for _ in 0..MAX_EPIC_DEPTH {
        if next == 0 {
            return None;
        }
        let Some(epic) = ctx.db.epics().id().find(next) else {
            return None;
        };
        if !epic.feed_command.is_empty() {
            return Some(epic.id);
        }
        next = epic.parent_epic_id;
    }
    None
}

/// `core/RetiredFeedItem: UniqueRetiredFeedItemPerFeed` as an idempotent
/// insert — a second retirement of the same (feed_epic_id, external_id) is a
/// no-op, mirroring SQLite's `INSERT OR IGNORE`.
fn retire_feed_item(ctx: &ReducerContext, feed_epic_id: i64, external_id: &str) {
    let exists = ctx
        .db
        .retired_feed_items()
        .feed_epic_id()
        .filter(&feed_epic_id)
        .any(|r| r.external_id == external_id);
    if !exists {
        ctx.db.retired_feed_items().insert(RetiredFeedItem {
            id: 0,
            feed_epic_id,
            external_id: external_id.to_string(),
            retired_at: now(ctx),
        });
    }
}

/// Delete a task and everything that only referred to it.
///
/// The watcher rows go with it in the same transaction. A watch pointing at a
/// task that no longer exists is not a row anybody can act on, and leaving it
/// would make "who is watching me?" answerable with a ghost.
///
/// Also detaches this task's learnings (`source_task_id` set to `None`, a
/// learning outlives its source as orphaned provenance) and cascades its
/// `learning_retrievals` rows — reproducing, as explicit reducer logic, the
/// two SQLite `ON DELETE` clauses `learnings.source_task_id` and
/// `learning_retrievals.task_id` used to carry
/// (`docs/specs/learnings.allium`'s Storage Backend section).
#[spacetimedb::reducer]
pub fn delete_task(ctx: &ReducerContext, id: i64) -> Result<(), String> {
    let Some(row) = ctx.db.tasks().id().find(id) else {
        return Ok(());
    };
    let epic_id = row.epic_id;
    // tasks.allium: DeleteTask's retirement clause. A manual task (no
    // external_id) or one under no feed epic in its chain retires nothing —
    // there is no cycle to suppress.
    if !row.external_id.is_empty() && epic_id != 0 {
        if let Some(feed_epic_id) = nearest_feed_epic(ctx, epic_id) {
            retire_feed_item(ctx, feed_epic_id, &row.external_id);
        }
    }
    ctx.db.tasks().id().delete(id);
    delete_agent_state_for(ctx, id);
    // Two indexed lookups rather than one scan of every watch in the store.
    // Collected first because deleting while walking an index is not something
    // the table API promises.
    let watches: Vec<i64> = ctx
        .db
        .task_watchers()
        .watcher_task_id()
        .filter(&id)
        .chain(ctx.db.task_watchers().target_task_id().filter(&id))
        .map(|w| w.id)
        .collect();
    for watch in watches {
        ctx.db.task_watchers().id().delete(watch);
    }
    detach_learnings_from_task(ctx, id);
    recalculate_epic_chain(ctx, epic_id);
    Ok(())
}

/// `source_task_id` is not indexed — nothing else ever looks a learning up by
/// it, and a full scan on a task delete is the same cost class `rescope_epic_learnings`
/// already accepts for `scope_ref`.
fn detach_learnings_from_task(ctx: &ReducerContext, task_id: i64) {
    update_matching_learnings(
        ctx,
        |l| l.source_task_id == Some(task_id),
        |row| Learning {
            source_task_id: None,
            ..row
        },
    );
    let retrievals: Vec<i64> = ctx
        .db
        .learning_retrievals()
        .task_id()
        .filter(&task_id)
        .map(|r| r.id)
        .collect();
    delete_learning_retrievals(ctx, retrievals);
}

/// Collect every learning matching `pred`, apply `f`, and write each back.
/// Shared by every reducer that scans the whole table for a bulk update —
/// `detach_learnings_from_task`, `rescope_epic_learnings` and
/// `archive_stale_learnings` all had this collect-then-loop shape separately.
fn update_matching_learnings(
    ctx: &ReducerContext,
    pred: impl Fn(&Learning) -> bool,
    f: impl Fn(Learning) -> Learning,
) {
    let matching: Vec<Learning> = ctx.db.learnings().iter().filter(|l| pred(l)).collect();
    for row in matching {
        ctx.db.learnings().id().update(f(row));
    }
}

/// Delete each `learning_retrievals` row by id. Shared by
/// `detach_learnings_from_task` and `delete_learning`, whose retrieval
/// cascades otherwise repeated the identical loop.
fn delete_learning_retrievals(ctx: &ReducerContext, ids: impl IntoIterator<Item = i64>) {
    for id in ids {
        ctx.db.learning_retrievals().id().delete(id);
    }
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
        .task_id()
        .filter(&task_id)
        .collect::<Vec<_>>()
    {
        ctx.db.task_shells().delete(row);
    }
    for row in ctx
        .db
        .task_subagents()
        .task_id()
        .filter(&task_id)
        .collect::<Vec<_>>()
    {
        ctx.db.task_subagents().delete(row);
    }
}

/// Move a task into an epic, out of one, or between two.
///
/// Separate from [`patch_task`] because it is the one field whose change has to
/// move something else with it: `core.allium: OwnerTracksUserBoardTask` ties
/// the owner to the epic being absent, so a task leaving its epic gains an
/// owner and a task joining one loses theirs. A patch route that could set
/// `epic_id` alone would produce a task on two boards or on none.
///
/// `owner` is what the task takes when it lands on a user board, and is ignored
/// when `epic_id` names an epic.
#[spacetimedb::reducer]
pub fn set_task_epic(
    ctx: &ReducerContext,
    id: i64,
    epic_id: i64,
    owner: String,
) -> Result<(), String> {
    let Some(task) = ctx.db.tasks().id().find(id) else {
        return Ok(());
    };
    let was_in = task.epic_id;
    if was_in == epic_id {
        return Ok(());
    }
    write_task(
        ctx,
        Task {
            epic_id,
            owner: if epic_id == 0 { owner } else { String::new() },
            updated_at: now(ctx),
            ..task
        },
    )?;
    // Both, and in that order. The one it left may now be complete; the one it
    // joined may no longer be.
    recalculate_epic_chain(ctx, was_in);
    recalculate_epic_chain(ctx, epic_id);
    Ok(())
}

/// `PhoenixRespawn`, atomic: inserts the successor and clears the
/// predecessor's `phoenix` flag in one transaction. Either both happen or
/// neither does — `TheFlagIsTheReceipt` holds literally, so a retry after a
/// failure cannot create a second successor. Mirrors
/// `src/db/queries/tasks.rs::respawn_phoenix_successor` exactly.
///
/// No internal recalculation: the caller (`PhoenixRespawn` in
/// `src/service/tasks/crud.rs`) already calls the already-routed
/// `recalculate_epic_status` itself on success, the same as every feed
/// ingestion call site below does for its own writes.
#[spacetimedb::reducer]
pub fn respawn_phoenix_successor(
    ctx: &ReducerContext,
    predecessor: i64,
    successor: Task,
) -> Result<(), String> {
    let Some(predecessor_row) = ctx.db.tasks().id().find(predecessor) else {
        return Err(format!("predecessor task {predecessor} not found"));
    };
    write_task(
        ctx,
        Task {
            id: 0,
            ..successor
        },
    )?;
    // Nothing between the check above and here can touch `predecessor_row` —
    // reducers run one at a time, and the write above only inserts a NEW
    // successor row — so the row already in hand is still current; no need
    // to re-fetch it.
    ctx.db.tasks().id().update(Task {
        phoenix: false,
        ..predecessor_row
    });
    Ok(())
}

/// Atomically set `sub_status` for many tasks. One reducer taking the whole
/// batch, so it applies whole or not at all — `TaskCrud::batch_patch_sub_status`'s
/// own doc comment already promises this shape.
///
/// No recalculation: `sub_status` is derived board state, not a
/// status/epic-linkage change (`TaskServiceApi::batch_patch_sub_status`'s doc
/// comment states this explicitly), so it carries no
/// `recalculate_epic_status` obligation on either backing.
#[spacetimedb::reducer]
pub fn batch_patch_sub_status(
    ctx: &ReducerContext,
    updates: Vec<SubStatusUpdate>,
) -> Result<(), String> {
    let stamp = now(ctx);
    for update in updates {
        let Some(row) = ctx.db.tasks().id().find(update.task_id) else {
            continue;
        };
        ctx.db.tasks().id().update(Task {
            sub_status: update.sub_status,
            updated_at: stamp.clone(),
            ..row
        });
    }
    Ok(())
}

/// One `(task_id, sub_status)` pair in a [`batch_patch_sub_status`] call.
#[derive(spacetimedb::SpacetimeType, Clone, Debug)]
pub struct SubStatusUpdate {
    pub task_id: i64,
    pub sub_status: String,
}

// -- Task watchers ------------------------------------------------------------

/// Insert a watch: `watcher_task_id` wants to be notified when
/// `target_task_id` finishes or is deleted first. Idempotent — inserting an
/// existing (watcher, target) pair is a no-op, checked by hand since the
/// module's `task_watchers` carries no uniqueness index over the pair (unlike
/// SQLite's `INSERT OR IGNORE`). Race-free for the same reason
/// [`create_repo_group_sub_epic`] below needs no index either: reducers run
/// one at a time.
#[spacetimedb::reducer]
pub fn create_task_watcher(
    ctx: &ReducerContext,
    watcher_task_id: i64,
    target_task_id: i64,
) -> Result<(), String> {
    let exists = ctx
        .db
        .task_watchers()
        .watcher_task_id()
        .filter(&watcher_task_id)
        .any(|w| w.target_task_id == target_task_id);
    if !exists {
        ctx.db.task_watchers().insert(TaskWatcher {
            id: 0,
            watcher_task_id,
            target_task_id,
            created_at: now(ctx),
        });
    }
    Ok(())
}

/// Delete every `task_watchers` row in `ids`. Shared tail of the three
/// deletes below, which differ only in which index picks `ids`.
fn delete_watcher_rows(ctx: &ReducerContext, ids: Vec<i64>) {
    for id in ids {
        ctx.db.task_watchers().id().delete(id);
    }
}

/// Remove a specific watch. Idempotent — no-op if it doesn't exist.
#[spacetimedb::reducer]
pub fn delete_task_watcher(
    ctx: &ReducerContext,
    watcher_task_id: i64,
    target_task_id: i64,
) -> Result<(), String> {
    let ids: Vec<i64> = ctx
        .db
        .task_watchers()
        .watcher_task_id()
        .filter(&watcher_task_id)
        .filter(|w| w.target_task_id == target_task_id)
        .map(|w| w.id)
        .collect();
    delete_watcher_rows(ctx, ids);
    Ok(())
}

/// Remove every watch where `target_task_id` is the target. Called after
/// firing finish/delete notifications for that target.
#[spacetimedb::reducer]
pub fn delete_watches_of_target(ctx: &ReducerContext, target_task_id: i64) -> Result<(), String> {
    let ids: Vec<i64> = ctx
        .db
        .task_watchers()
        .target_task_id()
        .filter(&target_task_id)
        .map(|w| w.id)
        .collect();
    delete_watcher_rows(ctx, ids);
    Ok(())
}

/// Remove every watch where `watcher_task_id` is the watcher. Called when the
/// watcher itself is deleted.
#[spacetimedb::reducer]
pub fn delete_watches_by_watcher(ctx: &ReducerContext, watcher_task_id: i64) -> Result<(), String> {
    let ids: Vec<i64> = ctx
        .db
        .task_watchers()
        .watcher_task_id()
        .filter(&watcher_task_id)
        .map(|w| w.id)
        .collect();
    delete_watcher_rows(ctx, ids);
    Ok(())
}

// -- Feed ingestion -----------------------------------------------------------
//
// Feed SCRIPT EXECUTION stays local — a SpacetimeDB module has no subprocess
// capability (feeds.allium). What lands here is the upsert of the rows the
// script produced: the CLIENT resolves every domain default a feed item needs
// (`sub_status` from `SubStatus::default_for(item.status)`, the inferred
// `url_type`) before sending, the same boundary `create_task`/`create_epic`
// already draw — a reducer takes a fully-formed row, not a partial one it has
// to complete. See [`FeedTaskUpsertItem`].
//
// No internal recalculation: every call site in `src/feed/` already calls the
// already-routed `recalculate_epic_status` explicitly afterward
// (`recalculate_epic_status_after_feed`), so duplicating it here would only be
// a second, harmless, idempotent call with no caller that needs it.

/// One feed item, already resolved to the fields a task row needs. Not a
/// [`Task`]: a feed item never carries `status`/`sub_status`/`repo_path`/
/// `base_branch`/`wrap_up_mode` on an UPDATE (those are USER-managed fields
/// preserved across a re-poll — see [`upsert_feed_item`]), so shaping this as
/// a full `Task` would invite a caller to believe setting one of those on an
/// existing row does something. It does not, by construction.
#[derive(spacetimedb::SpacetimeType, Clone, Debug)]
pub struct FeedTaskUpsertItem {
    pub external_id: String,
    pub title: String,
    pub description: String,
    pub repo_path: String,
    pub status: String,
    pub sub_status: String,
    pub base_branch: String,
    pub tag: String,
    pub labels: String,
    pub sort_order: Option<i64>,
    pub url: String,
    pub url_type: String,
    pub wrap_up_mode: String,
}

/// Insert or update one feed item under `epic_id`, preserving the same fields
/// the SQLite `ON CONFLICT DO UPDATE SET` preserves and updating the same
/// ones it updates.
///
/// **Updated on conflict:** title, description, tag, labels, sort_order, and
/// url/url_type — but only when the existing row has no url yet; an existing
/// non-null url always wins, so a feed cannot blank out or replace a url a
/// user (or an earlier, richer emission) already set.
///
/// **Preserved on conflict:** status, sub_status, repo_path, base_branch,
/// wrap_up_mode, completed_at — user-managed or store-owned fields a re-poll
/// must not disturb. `status`/`sub_status` not moving is what makes a re-poll
/// not a completion; `completed_at` not moving is a direct consequence — see
/// [`write_task`]'s own stamp-on-insert logic, which this reuses for the
/// INSERT branch instead of re-deriving it.
///
/// Looked up by `(epic_id, external_id)` via a scan of `epic_id`'s index —
/// the module carries no partial unique index over the pair the way SQLite's
/// `ON CONFLICT(epic_id, external_id) WHERE external_id IS NOT NULL` target
/// does, and needs none: reducers run one at a time, so there is no second
/// call to race with a check-then-act sequence.
///
/// `feed_epic_id` is `nearest_feed_epic(epic_id)`, resolved once by the caller
/// and reused for every item in the batch (`feeds.allium:
/// IngestSkipsRetiredFeedItems`): a retired `external_id` with no existing
/// survivor row is refused outright; an existing survivor (e.g. the
/// worktree-holding case `ArchivedStatusMigration` left in done) still matches
/// above and is refreshed regardless of retirement.
fn upsert_feed_item(
    ctx: &ReducerContext,
    epic_id: i64,
    feed_epic_id: Option<i64>,
    item: &FeedTaskUpsertItem,
    created_by: &str,
) {
    let existing = ctx
        .db
        .tasks()
        .epic_id()
        .filter(&epic_id)
        .find(|t| t.external_id == item.external_id);

    if existing.is_none() {
        if let Some(feed_epic_id) = feed_epic_id {
            let retired = ctx
                .db
                .retired_feed_items()
                .feed_epic_id()
                .filter(&feed_epic_id)
                .any(|r| r.external_id == item.external_id);
            if retired {
                return;
            }
        }
    }

    let row = match existing {
        Some(existing) => {
            let (url, url_type) = if existing.url.is_empty() {
                (item.url.clone(), item.url_type.clone())
            } else {
                (existing.url.clone(), existing.url_type.clone())
            };
            Task {
                title: item.title.clone(),
                description: item.description.clone(),
                tag: item.tag.clone(),
                labels: item.labels.clone(),
                sort_order: item.sort_order,
                url,
                url_type,
                updated_at: now(ctx),
                ..existing
            }
        }
        None => Task {
            id: 0,
            title: item.title.clone(),
            description: item.description.clone(),
            repo_path: item.repo_path.clone(),
            status: item.status.clone(),
            sub_status: item.sub_status.clone(),
            base_branch: item.base_branch.clone(),
            epic_id,
            external_id: item.external_id.clone(),
            tag: item.tag.clone(),
            labels: item.labels.clone(),
            sort_order: item.sort_order,
            url: item.url.clone(),
            url_type: item.url_type.clone(),
            wrap_up_mode: item.wrap_up_mode.clone(),
            created_at: now(ctx),
            updated_at: now(ctx),
            // Overrides `blank_task()`'s SCRATCH_OWNER: a feed task always
            // has an epic, so `validate_task_ownership` requires an ABSENT
            // owner here, not a non-empty sentinel.
            owner: String::new(),
            // Which host's feed run produced this row (`core.allium:
            // Task.created_by`, `feeds.allium: UpsertFeedTasks`). Insert
            // only, matching `created_by`'s "stamped once, never rewritten"
            // rule elsewhere — the update branch above never touches it.
            created_by: created_by.to_string(),
            ..blank_task()
        },
    };
    // write_task's stamp-on-Done-insert logic covers the INSERT branch; on
    // the UPDATE branch `status`/`completed_at` are carried over unchanged
    // from `existing`, so the condition cannot fire there.
    let _ = write_task(ctx, row);
}

/// Delete every task in `epic_id` whose `external_id` is set and not in
/// `keep`. Shared by [`upsert_feed_tasks_inner`]'s single-epic pass and
/// [`delete_stale_subtree_feed_tasks`]'s per-child-epic loop.
fn delete_stale_feed_tasks_in_epic(
    ctx: &ReducerContext,
    epic_id: i64,
    keep: &std::collections::HashSet<&str>,
) {
    let stale: Vec<i64> = ctx
        .db
        .tasks()
        .epic_id()
        .filter(&epic_id)
        .filter(|t| !t.external_id.is_empty() && !keep.contains(t.external_id.as_str()))
        .map(|t| t.id)
        .collect();
    for id in stale {
        ctx.db.tasks().id().delete(id);
    }
}

/// Shared body of [`upsert_feed_tasks`] and [`upsert_feed_tasks_additive`].
/// `delete_absent` selects the stale-delete pass the same way
/// `src/db/queries/tasks.rs::upsert_feed_tasks_inner` does.
fn upsert_feed_tasks_inner(
    ctx: &ReducerContext,
    epic_id: i64,
    items: Vec<FeedTaskUpsertItem>,
    created_by: &str,
    delete_absent: bool,
) -> Result<(), String> {
    if ctx.db.epics().id().find(epic_id).is_none() {
        return Err(format!("epic {epic_id} not found for upsert_feed_tasks"));
    }
    // feeds.allium: IngestSkipsRetiredFeedItems. One resolution, reused for
    // every item — mirrors src/db/queries/tasks.rs::upsert_feed_tasks_inner's
    // single `nearest_feed_epic` query.
    let feed_epic_id = nearest_feed_epic(ctx, epic_id);
    for item in &items {
        upsert_feed_item(ctx, epic_id, feed_epic_id, item, created_by);
    }
    if delete_absent {
        let keep: std::collections::HashSet<&str> =
            items.iter().map(|i| i.external_id.as_str()).collect();
        delete_stale_feed_tasks_in_epic(ctx, epic_id, &keep);
    }
    Ok(())
}

/// Upsert tasks from a feed, reconciling: every stale feed task in `epic_id`
/// absent from `items` is removed. `feeds.allium: UpsertFeedTasks`.
///
/// `created_by` names the calling host's own owner identity
/// (`local_host().owner`) — stamped on every newly INSERTED task, never on an
/// update. May be empty: an install that has never connected to a shared
/// store has no identity to stamp, and that is a real, honest state rather
/// than a call this reducer refuses (`core.allium: Task.created_by`).
#[spacetimedb::reducer]
pub fn upsert_feed_tasks(
    ctx: &ReducerContext,
    epic_id: i64,
    items: Vec<FeedTaskUpsertItem>,
    created_by: String,
) -> Result<(), String> {
    upsert_feed_tasks_inner(ctx, epic_id, items, &created_by, true)
}

/// The insert/update half of [`upsert_feed_tasks`] WITHOUT its stale-delete
/// pass — items absent from `items` are left alone. For a partially degraded
/// emission whose omissions are not trustworthy evidence a task is gone
/// (`feeds.allium: DegradedNonEmptyEmission`). `created_by` as
/// [`upsert_feed_tasks`] documents.
#[spacetimedb::reducer]
pub fn upsert_feed_tasks_additive(
    ctx: &ReducerContext,
    epic_id: i64,
    items: Vec<FeedTaskUpsertItem>,
    created_by: String,
) -> Result<(), String> {
    upsert_feed_tasks_inner(ctx, epic_id, items, &created_by, false)
}

/// Delete stale feed tasks across the WHOLE subtree of `parent_id` (every
/// direct child epic), keeping only `keep_external_ids`. Manual tasks
/// (`external_id` empty) are always preserved.
#[spacetimedb::reducer]
pub fn delete_stale_subtree_feed_tasks(
    ctx: &ReducerContext,
    parent_id: i64,
    keep_external_ids: Vec<String>,
) -> Result<(), String> {
    let keep: std::collections::HashSet<&str> =
        keep_external_ids.iter().map(String::as_str).collect();
    let child_epics: Vec<i64> = ctx
        .db
        .epics()
        .parent_epic_id()
        .filter(&parent_id)
        .map(|e| e.id)
        .collect();
    for epic_id in child_epics {
        delete_stale_feed_tasks_in_epic(ctx, epic_id, &keep);
    }
    Ok(())
}

/// Retire one `(feed_epic_id, external_id)` directly — the write side of
/// `db::TaskCrud::create_retired_feed_item`, called from outside the delete
/// paths above (e.g. a future direct-retirement call site). Idempotent; see
/// [`retire_feed_item`].
#[spacetimedb::reducer]
pub fn create_retired_feed_item(
    ctx: &ReducerContext,
    feed_epic_id: i64,
    external_id: String,
) -> Result<(), String> {
    retire_feed_item(ctx, feed_epic_id, &external_id);
    Ok(())
}

/// feeds.allium: `DropClosedRetiredFeedItems`. Drop every `retired_feed_items`
/// row keyed on `feed_epic_id` whose `external_id` is absent from
/// `keep_external_ids` — called only after a TRUSTED (mirroring, non-additive)
/// cycle whose parsed emission no longer carries the id, so the upstream item
/// closed and a later reopen shows up as new.
#[spacetimedb::reducer]
pub fn drop_closed_retired_feed_items(
    ctx: &ReducerContext,
    feed_epic_id: i64,
    keep_external_ids: Vec<String>,
) -> Result<(), String> {
    let keep: std::collections::HashSet<&str> =
        keep_external_ids.iter().map(String::as_str).collect();
    let stale: Vec<i64> = ctx
        .db
        .retired_feed_items()
        .feed_epic_id()
        .filter(&feed_epic_id)
        .filter(|r| !keep.contains(r.external_id.as_str()))
        .map(|r| r.id)
        .collect();
    for id in stale {
        ctx.db.retired_feed_items().id().delete(id);
    }
    Ok(())
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

    apply_epic_patch(&mut row, patch);
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
///
/// epics.allium: `DeleteEpic`'s retirement clause runs first, over the WHOLE
/// doomed subtree computed before anything is deleted — reading it after
/// would see nothing. Every doomed epic's own `retired_feed_items` rows are
/// then dropped with it: deleting a feed epic is a reset, not a prune, and
/// SQLite's `ON DELETE CASCADE` has no module-side equivalent to do this for
/// free.
#[spacetimedb::reducer]
pub fn delete_epic(ctx: &ReducerContext, id: i64) -> Result<(), String> {
    let Some(row) = ctx.db.epics().id().find(id) else {
        return Ok(());
    };
    let parent = row.parent_epic_id;
    let doomed = collect_epic_subtree_ids(ctx, id);
    retire_feed_tasks_before_epic_delete(ctx, &doomed);
    delete_epic_subtree(ctx, id, 0);
    for &doomed_id in &doomed {
        let stale: Vec<i64> = ctx
            .db
            .retired_feed_items()
            .feed_epic_id()
            .filter(&doomed_id)
            .map(|r| r.id)
            .collect();
        for retired_id in stale {
            ctx.db.retired_feed_items().id().delete(retired_id);
        }
    }
    recalculate_epic_chain(ctx, parent);
    Ok(())
}

/// Every epic id in `root`'s subtree, itself included — computed BEFORE any
/// delete, so [`delete_epic`] knows which ids are "doomed" while the rows
/// describing them still exist. Mirrors
/// `src/db/queries/epics.rs::retire_feed_tasks_before_epic_delete`'s recursive
/// CTE.
fn collect_epic_subtree_ids(ctx: &ReducerContext, root: i64) -> std::collections::HashSet<i64> {
    let mut doomed = std::collections::HashSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !doomed.insert(id) {
            continue;
        }
        for child in ctx.db.epics().parent_epic_id().filter(&id) {
            stack.push(child.id);
        }
    }
    doomed
}

/// epics.allium: `DeleteEpic`'s retirement clause. For every feed task
/// anywhere in `doomed`'s subtree, write a `retired_feed_items` row keyed on
/// its `nearest_feed_epic` — UNLESS that epic is itself part of `doomed`, in
/// which case there is nothing surviving to retire under (the feed epic's own
/// delete is a reset, and its existing records are dropped by [`delete_epic`]
/// instead). Must run before the subtree's tasks/epics are actually deleted.
fn retire_feed_tasks_before_epic_delete(ctx: &ReducerContext, doomed: &std::collections::HashSet<i64>) {
    let tasks: Vec<(i64, String)> = doomed
        .iter()
        .flat_map(|&epic_id| {
            ctx.db
                .tasks()
                .epic_id()
                .filter(&epic_id)
                .filter(|t| !t.external_id.is_empty())
                .map(|t| (t.epic_id, t.external_id))
                .collect::<Vec<_>>()
        })
        .collect();
    for (epic_id, external_id) in tasks {
        let Some(feed_epic_id) = nearest_feed_epic(ctx, epic_id) else {
            continue;
        };
        if doomed.contains(&feed_epic_id) {
            continue;
        }
        retire_feed_item(ctx, feed_epic_id, &external_id);
    }
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
        .parent_epic_id()
        .filter(&id)
        .collect::<Vec<_>>()
    {
        delete_epic_subtree(ctx, child.id, depth + 1);
    }
    for task in ctx.db.tasks().epic_id().filter(&id).collect::<Vec<_>>() {
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

        recalculate_one(ctx, &epic);
        next = epic.parent_epic_id;
    }
}

/// Derive and write one epic's status, from every one of its children.
fn recalculate_one(ctx: &ReducerContext, epic: &Epic) {
    let children: Vec<String> = ctx
        .db
        .tasks()
        .epic_id()
        .filter(&epic.id)
        .map(|t| t.status)
        .chain(
            ctx.db
                .epics()
                .parent_epic_id()
                .filter(&epic.id)
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
}

/// Find-or-create the `RepoGroup` sub-epic of `parent_id` titled `title`.
/// Mirrors `src/db/queries/epics.rs::create_repo_group_sub_epic`, minus the
/// unique-constraint retry arm that mirror needs and this does not: SQLite
/// has concurrent writers and a partial unique index to referee them; a
/// SpacetimeDB reducer runs to completion before the next one starts, so the
/// look-up this function does IS the whole safety argument — there is no
/// window for a second caller's insert to land between it and this one's.
///
/// **`created_by` is not optional here.** An epic has no `owner` field at
/// all, and the per-epic subscription (`WHERE id = {epic}`) covers only that
/// epic's own row and its tasks — never a NEW CHILD epic's row, whatever the
/// child's parent. `own_creations` (`WHERE created_by = {identity}`) is the
/// only subscription that can ever make a freshly created or freshly found
/// sub-epic visible to the caller that needs its id back — see
/// `create_epic`'s `matches_created_epic` precedent, which this follows.
/// `created_by` is written only on a genuine insert; the found-existing arm
/// leaves a prior creator's stamp untouched, same as
/// `LocalHostOwnerIsWrittenOnce`-shaped fields elsewhere in this module never
/// get silently reassigned to whoever asked most recently.
///
/// There is no archived state for a found epic to be unarchived out of any
/// more, so a match is simply left alone and the only other case is a fresh
/// insert. Unlike a managed epic (`epics.allium`'s `ProvisionManagedEpics`
/// guidance on rename-stability), this reducer is NOT rename-stable: it
/// matches on `(parent_id, title)`, same as
/// `src/db/queries/epics.rs::create_repo_group_sub_epic`, so a user rename of
/// a repo-group sub-epic makes the next grouped cycle create a new one.
#[spacetimedb::reducer]
pub fn create_repo_group_sub_epic(
    ctx: &ReducerContext,
    parent_id: i64,
    title: String,
    created_by: String,
) -> Result<(), String> {
    let existing = ctx
        .db
        .epics()
        .parent_epic_id()
        .filter(&parent_id)
        .find(|e| e.title == title && e.origin == "repo-group");
    if existing.is_none() {
        ctx.db.epics().insert(Epic {
            title,
            parent_epic_id: parent_id,
            origin: "repo-group".to_string(),
            created_by,
            created_at: now(ctx),
            updated_at: now(ctx),
            ..blank_epic()
        });
    }
    Ok(())
}

/// Create-or-return a managed-feed-role epic, `feed_role` set from the start.
/// Mirrors `src/db/queries/epics.rs::create_managed_role_epic` on the same
/// terms [`create_repo_group_sub_epic`] documents: race-free by construction,
/// no unique-index retry arm needed, `created_by` required for the same
/// `own_creations` reason.
#[spacetimedb::reducer]
pub fn create_managed_role_epic(
    ctx: &ReducerContext,
    title: String,
    parent_epic_id: i64,
    role: String,
    feed_command: String,
    feed_interval_secs: i64,
    created_by: String,
) -> Result<(), String> {
    let existing = ctx
        .db
        .epics()
        .parent_epic_id()
        .filter(&parent_epic_id)
        .find(|e| e.feed_role == role);
    if existing.is_none() {
        ctx.db.epics().insert(Epic {
            title,
            parent_epic_id,
            feed_role: role,
            feed_command,
            feed_interval_secs,
            // `origin` stays at `blank_epic()`'s "manual" default, unlisted
            // here on purpose: the SQLite INSERT this mirrors
            // (`src/db/queries/epics.rs::create_managed_role_epic`) never sets
            // it either, so a managed-role epic's origin is "manual" today —
            // only `create_repo_group_sub_epic` above writes "repo-group",
            // and nothing anywhere reads "managed" as an origin value.
            created_by,
            created_at: now(ctx),
            updated_at: now(ctx),
            ..blank_epic()
        });
    }
    Ok(())
}

// -- The dispatch claim ------------------------------------------------------
//
// THE MOST SAFETY-CRITICAL RULE IN THE SYSTEM, and the one a shared store both
// endangers and fixes. `dispatch.allium: DispatchClaimExclusive` says one task
// is claimed by one dispatcher; on a single machine a single SQL statement made
// that true. Two machines racing for the same backlog subtask had no such
// statement, and the losing one would provision a second worktree over the
// winner's.
//
// A reducer is a transaction, so the read that chooses the row and the write
// that claims it cannot be interleaved by another host. That is the whole fix,
// and it is why these do not live on the client.
//
// WHAT A CLAIM WRITES is one decision and WHICH ROWS IT ACCEPTS is another —
// the same split `CLAIM_SET` and its per-caller `WHERE` make on the SQLite
// side. `apply_claim` is the first; the two reducers are the second.

/// Everything a claim writes to the row it wins.
fn apply_claim(ctx: &ReducerContext, task: Task) -> Task {
    Task {
        status: "running".into(),
        sub_status: "active".into(),
        // Seeded so the dispatch watchdog measures from the claim rather than
        // from whenever the agent first says something. An unseeded claim looks
        // like a task that has been silent since the epoch.
        last_pre_tool_use_at: now(ctx),
        updated_at: now(ctx),
        ..task
    }
}

/// Whether this host may claim the task, by `dispatch.allium`'s
/// `requires: task.is_locally_owned`.
///
/// A foreign-owned backlog subtask is one whose worktree sits on another
/// machine. Claiming it here would dispatch an agent with nowhere to work.
/// Absent (`""`) is local: a task that has never been dispatched belongs to
/// whoever gets to it. `pub` so `MemoryReducerCaller`
/// (src/sync/memory_caller.rs) calls this directly instead of re-deriving the
/// same check — the same reuse `derive_epic_status` and friends already get.
pub fn claimable_by(task: &Task, host: &str) -> bool {
    task.host.is_empty() || task.host == host
}

// THERE IS NO `claim_next_backlog_task`, AND THAT IS DELIBERATE.
//
// The obvious design is a reducer that picks the next candidate and claims it
// in one transaction. It was written, and then removed, because a reducer
// returns no value: the chain's caller needs to know WHICH task it claimed —
// it is about to provision that task's worktree — and there is no channel to
// tell it.
//
// The named claim above is enough, because a board choosing a candidate for an
// epic is NOT choosing from a partial view. A subscription to an epic is
// `WHERE epic_id = N`, which returns every subtask of it regardless of who
// owns them, so a board that is chaining an epic can see all of that epic's
// direct children. The visibility problem that forced the status derivation
// server-side (`derive_epic_status`) is about ANCESTORS and descendants across
// epics; it does not reach one epic's own subtask list.
//
// So the client orders the candidates by `claim_order_key`'s rule and offers
// them one at a time, and this reducer arbitrates: a task another host already
// took is refused, and the client moves to the next. Exclusivity is still the
// store's, which is the part that had to move.

/// LOSING IS AN ERROR HERE, and that is how the caller finds out.
///
/// A reducer returns no value, so "did I win?" has to be carried by the one
/// channel a reducer does have: accepted or refused
/// (`sync.allium: EveryMutationIsAtomicAndAnswered`). Reading the row back
/// instead would not work — a claim does not stamp the winner's name on it, so
/// two hosts asking "is it running now?" after a race would both see `running`
/// and both believe they won, which is the exact failure this reducer exists to
/// prevent.
///
/// The refusal is therefore ORDINARY rather than exceptional. The client maps
/// it to "somebody else got there first" and moves to the next candidate; it is
/// a transport failure, not this, that means the store is down.
#[spacetimedb::reducer]
pub fn claim_backlog_task(ctx: &ReducerContext, id: i64, host: String) -> Result<(), String> {
    let Some(task) = ctx.db.tasks().id().find(id) else {
        return Err(format!("task {id} no longer exists"));
    };
    if task.status != BACKLOG {
        return Err(format!(
            "task {id} is {} rather than backlog, so it is already claimed",
            task.status
        ));
    }
    if !claimable_by(&task, &host) {
        return Err(format!(
            "task {id}'s worktree is on {}, so {host} cannot claim it",
            task.host
        ));
    }
    // THROUGH `write_task`, not around it. Neither this nor the release below
    // touches `epic_id` or `owner`, so nothing is broken by a direct update
    // today — but `write_task`'s whole claim is that it is the only path, and a
    // second path is one the next claim-adjacent reducer copies.
    write_task(ctx, apply_claim(ctx, task))
}

/// Undo a claim whose provisioning failed.
///
/// Guarded on `worktree` being absent, and that guard is the whole safety of
/// it: a claim that got as far as a worktree is a dispatch in progress, and
/// releasing it would put a running agent's task back in the backlog for
/// another host to claim underneath it.
#[spacetimedb::reducer]
pub fn release_backlog_claim(ctx: &ReducerContext, id: i64) -> Result<(), String> {
    let Some(task) = ctx.db.tasks().id().find(id) else {
        return Err(format!("task {id} no longer exists"));
    };
    if task.status != "running" {
        return Err(format!("task {id} is not claimed"));
    }
    if !task.worktree.is_empty() {
        return Err(format!(
            "task {id} already has a worktree, so releasing it would put a running \
             agent's task back in the backlog"
        ));
    }
    write_task(
        ctx,
        Task {
            status: BACKLOG.into(),
            sub_status: "none".into(),
            last_pre_tool_use_at: String::new(),
            updated_at: now(ctx),
            ..task
        },
    )
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

// -- Settings (Phase 9) -------------------------------------------------------

/// The derived key of a setting row. Must agree character for
/// character with the SQLite side's equivalent, the same requirement
/// `subscription_id` below states for `Subscription`.
fn host_scoped_id(host: &str, key: &str) -> String {
    format!("{host}/{key}")
}

#[spacetimedb::reducer]
pub fn save_setting(
    ctx: &ReducerContext,
    host: String,
    key: String,
    value: String,
) -> Result<(), String> {
    if host.trim().is_empty() {
        return Err("a host id must not be empty".to_string());
    }
    let id = host_scoped_id(&host, &key);
    match ctx.db.settings().id().find(&id) {
        Some(existing) => ctx.db.settings().id().update(Setting { value, ..existing }),
        None => ctx.db.settings().insert(Setting {
            id,
            host,
            key,
            value,
        }),
    };
    Ok(())
}

#[spacetimedb::reducer]
pub fn clear_setting(ctx: &ReducerContext, host: String, key: String) -> Result<(), String> {
    let id = host_scoped_id(&host, &key);
    ctx.db.settings().id().delete(&id);
    Ok(())
}

// -- Hosts and subscriptions ------------------------------------------------

/// Upsert this machine's row into the shared host registry.
///
/// Decided on task #4907, not assumed: `ensure_host_identity`/`rename_host`/
/// `adopt_user_identity` (`src/db/queries/settings.rs`) keep writing the
/// LOCAL settings row unconditionally, connected or not — that write is the
/// durable identity credential, not a shared table with one copy, so it is
/// deliberately NOT routed through `SharedWriter` the way every other method
/// in this file is. This reducer is the separate MIRROR push that makes the
/// resulting row visible to the rest of the registry, called from
/// `sync.allium: RegisterHostOnConnect` (every identity settle) and
/// `RegisterHostOnRename` (a live rename while connected) — see those rules
/// for when it fires and why re-sending an unchanged row on every reconnect
/// is correct rather than wasteful.
///
/// Plain upsert-by-id: `id` and `owner` are set once each and never change
/// again for a given host (`core.allium: LocalHostOwnerIsWrittenOnce`), so
/// overwriting them here on every call is harmless — there is nothing to lose
/// by not special-casing "first register" vs. "later register".
#[spacetimedb::reducer]
pub fn register_host(
    ctx: &ReducerContext,
    id: String,
    label: String,
    owner: String,
) -> Result<(), String> {
    if id.trim().is_empty() {
        return Err("a host id must not be empty".to_string());
    }
    match ctx.db.hosts().id().find(&id) {
        Some(existing) => {
            ctx.db.hosts().id().update(Host {
                label,
                owner,
                ..existing
            });
        }
        None => {
            ctx.db.hosts().insert(Host { id, label, owner });
        }
    }
    Ok(())
}

// -- Poll ownership -----------------------------------------------------

/// Find-or-create-or-reassign a [`PollOwner`] row for `(scope, scope_id)`.
///
/// `force = false` (a claim — `pr-workflow.allium: PollPrStatus`,
/// `feeds.allium: FeedTick`) fills an ABSENT row and leaves an existing one
/// alone, whoever it names. `force = true` (an override —
/// `pr-workflow.allium: OverridePrPollOwner`, `feeds.allium:
/// OverrideFeedOwner`) unconditionally reassigns an existing row too — the
/// only way an existing claim ever moves. One function rather than a pair:
/// the two only ever differed in what happens when a row already exists.
///
/// Shared by [`claim_poll_owner`] and [`override_poll_owner`], which are
/// themselves shared by both scopes (task and epic) — `scope` is caller-
/// validated input, not a typed enum, matching how every other
/// module-boundary enum-shaped value here (`Task.status`, `Task.url_type`,
/// `Task.wrap_up_mode`, …) is a plain validated `String` rather than a
/// SATS enum with its own reducer per variant.
fn write_poll_owner_row(ctx: &ReducerContext, scope: &str, scope_id: i64, host: String, force: bool) {
    let existing = ctx
        .db
        .poll_owners()
        .scope_id()
        .filter(&scope_id)
        .find(|p| p.scope == scope);
    match existing {
        Some(row) if force => {
            ctx.db.poll_owners().id().update(PollOwner {
                host,
                claimed_at: now(ctx),
                ..row
            });
        }
        Some(_) => {}
        None => {
            ctx.db.poll_owners().insert(PollOwner {
                id: 0,
                scope: scope.to_string(),
                scope_id,
                host,
                claimed_at: now(ctx),
            });
        }
    }
}

/// Reject anything but `"task"`/`"epic"` — the typo-safety
/// `write_poll_owner_row`'s callers need, at the same input-validation
/// boundary this module already enforces every other caller-supplied enum
/// string at (see e.g. `KNOWN_STATUSES`).
fn require_poll_scope(scope: &str) -> Result<(), String> {
    if scope == POLL_SCOPE_TASK || scope == POLL_SCOPE_EPIC {
        Ok(())
    } else {
        Err(format!(
            "poll scope must be {POLL_SCOPE_TASK:?} or {POLL_SCOPE_EPIC:?}, got {scope:?}"
        ))
    }
}

/// Claim an unowned scope. `core.allium: PollOwner`.
#[spacetimedb::reducer]
pub fn claim_poll_owner(
    ctx: &ReducerContext,
    scope: String,
    scope_id: i64,
    host: String,
) -> Result<(), String> {
    require_poll_scope(&scope)?;
    write_poll_owner_row(ctx, &scope, scope_id, host, false);
    Ok(())
}

/// Reassign a scope's ownership unconditionally. `pr-workflow.allium:
/// OverridePrPollOwner`, `feeds.allium: OverrideFeedOwner`.
#[spacetimedb::reducer]
pub fn override_poll_owner(
    ctx: &ReducerContext,
    scope: String,
    scope_id: i64,
    host: String,
) -> Result<(), String> {
    require_poll_scope(&scope)?;
    write_poll_owner_row(ctx, &scope, scope_id, host, true);
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
/// two backings produces two rows that are the same subscription. `pub` so
/// `MemoryReducerCaller` (src/sync/memory_caller.rs) calls this directly
/// instead of carrying a third copy — the same reuse `derive_epic_status` and
/// friends already get.
pub fn subscription_id(subscriber: &str, epic_id: i64) -> String {
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

// -- Agent session state (Phase 6b) ------------------------------------------

/// The statuses/sub-statuses this section's reducers read or write, in the
/// store's spelling. Named for the same reason `DONE`/`BACKLOG` above are:
/// every other status/sub-status is passed in by the caller
/// (`sub_status` in `record_pre_tool_use`, `status` inside a full row), so
/// only the values these reducers themselves decide need a name here.
const RUNNING: &str = "running";
const REVIEW: &str = "review";
const ACTIVE: &str = "active";
const AWAITING_REVIEW: &str = "awaiting_review";
const NEEDS_INPUT: &str = "needs_input";

//
// The denormalised counters (`live_subagents`, `stop_pending`) and the
// `task_subagents` table that backs them. Mirrors
// `src/db/queries/{subagents,tasks}.rs` — see `docs/specs/
// agent-health.allium` for the guarantees these reproduce; nothing here
// changes what a hook does, only where the counting happens.
//
// EVENT TIME, NOT WRITE TIME. `last_pre_tool_use_at`, `last_notification_at`
// and `stop_pending_at` are the CLIENT's clock — the instant the hook fired —
// passed in as arguments, the same way `created_at` is a client timestamp
// (see `encode.rs`'s note on `ReducerWriter::now`). `updated_at` alone is the
// STORE's clock (`now(ctx)`), because it is bookkeeping about the write, not
// a fact about the agent. Getting this backwards would matter: `agent-health.
// allium`'s `HookUserPromptSubmit` guidance is explicit that the tie-break
// between a deferred `Stop` and the prompt that supersedes it must compare
// EVENT times — "any ordering derived from write order inherits the race the
// rule is trying to resolve" — and a write-time comparison across two
// reducers invoked from two different hook PROCESSES (each with its own
// network latency to the store) is exactly the write-order race that
// guidance warns against.
//
// AMBIGUOUS OUTCOMES ARE REFUSED, NOT GUESSED. A reducer returns no value, so
// the client reads its effect back off the row via `ctx.db` inside the
// `_then` callback (the same mechanism `create_task`'s id read-back uses) —
// but every one of these acts on a row that already exists and whose id the
// caller already has, so there is no id to match: the row is read by primary
// key, not by guessing which one arrived. `try_record_stop` and
// `record_user_prompt_submit` each have a branch (flip vs. defer; resume vs.
// refresh) that is unambiguous to read back ONLY once the precondition
// (`status = Running`, or `status in {Running, Review}`) is known to have
// held — and refusing when it does not turns "the precondition failed" into
// an ordinary `ReducerOutcome::Refused` instead of a state indistinguishable
// from "it just succeeded quietly". See this task's plan doc
// (docs/plans/2026-09-21-phase-6b-agent-session-state-reducers.md), decision 3.

/// Recompute `live_subagents` from `task_subagents` and write it, if the task
/// still exists. Mirrors `src/db/queries/subagents.rs::sync_count`. A missing
/// task is a silent no-op, matching the SQL `UPDATE ... WHERE id = ?` that
/// simply touches zero rows.
fn sync_subagent_count(ctx: &ReducerContext, task_id: i64) -> Result<i64, String> {
    let count = ctx.db.task_subagents().task_id().filter(&task_id).count() as i64;
    if let Some(row) = ctx.db.tasks().id().find(task_id) {
        write_task(
            ctx,
            Task {
                live_subagents: count,
                ..row
            },
        )?;
    }
    Ok(count)
}

/// Evict `task_subagents` rows for `task_id` whose `session_id` differs from
/// `incoming`. Mirrors `subagents.rs::fence_session` / `agent-health.allium:
/// SubagentSessionFence`.
fn fence_subagent_session(ctx: &ReducerContext, task_id: i64, incoming: &str) {
    for row in ctx
        .db
        .task_subagents()
        .task_id()
        .filter(&task_id)
        .filter(|r| r.session_id != incoming)
        .collect::<Vec<_>>()
    {
        ctx.db.task_subagents().delete(row);
    }
}

/// Apply a deferred `Stop` if this write is the one that drained the last
/// subagent. Mirrors `src/db/queries/mod.rs::apply_pending_stop_if_drained`
/// — the SAME shared predicate `subagent_stop` and `subagent_clear` both
/// route through below.
///
/// `last_pre_tool_use_at`/`last_notification_at` are cleared to the module's
/// empty-string sentinel, matching `STOP_FLIP_SET`'s `NULL`.
fn apply_pending_stop_if_drained(ctx: &ReducerContext, task_id: i64) -> Result<bool, String> {
    let Some(row) = ctx.db.tasks().id().find(task_id) else {
        return Ok(false);
    };
    if row.status == RUNNING && row.stop_pending && row.live_subagents == 0 {
        flip_to_review(ctx, row)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Flip `row` to `Review` — clearing the hook-activity timestamps and the
/// deferred-Stop bit — and recalculate the epic that leaves as a derivation
/// input. Shared by [`apply_pending_stop_if_drained`]'s drain branch and
/// `try_record_stop`'s immediate flip below: the only two places a task ever
/// makes this transition.
fn flip_to_review(ctx: &ReducerContext, row: Task) -> Result<(), String> {
    let epic_id = row.epic_id;
    write_task(
        ctx,
        Task {
            status: REVIEW.into(),
            sub_status: AWAITING_REVIEW.into(),
            last_pre_tool_use_at: String::new(),
            last_notification_at: String::new(),
            stop_pending: false,
            ..row
        },
    )?;
    recalculate_epic_chain(ctx, epic_id);
    Ok(())
}

/// Delete the `task_subagents` row for `(task_id, agent_id)`, if any. Neither
/// table has a primary key, so `subagent_start`'s dedupe-then-insert
/// "replace" is delete-then-insert rather than an update.
fn delete_subagent_entry(ctx: &ReducerContext, task_id: i64, agent_id: &str) {
    for row in ctx
        .db
        .task_subagents()
        .task_id()
        .filter(&task_id)
        .filter(|r| r.agent_id == agent_id)
        .collect::<Vec<_>>()
    {
        ctx.db.task_subagents().delete(row);
    }
}

/// Delete every `task_subagents` row for `task_id`.
fn delete_all_subagents(ctx: &ReducerContext, task_id: i64) {
    for row in ctx
        .db
        .task_subagents()
        .task_id()
        .filter(&task_id)
        .collect::<Vec<_>>()
    {
        ctx.db.task_subagents().delete(row);
    }
}

/// `HookSubagentStart` in `docs/specs/agent-health.allium`. `started_at` is
/// the client's clock, stored verbatim (RFC 3339, matching
/// `subagents.rs::subagent_start`'s `now.to_rfc3339()`) — this column is never
/// compared across rows, so it carries no format requirement of its own.
#[spacetimedb::reducer]
pub fn subagent_start(
    ctx: &ReducerContext,
    task_id: i64,
    agent_id: String,
    session_id: String,
    started_at: String,
) -> Result<(), String> {
    fence_subagent_session(ctx, task_id, &session_id);
    // Dedupe first, then insert fresh: an upsert by (task_id, agent_id).
    delete_subagent_entry(ctx, task_id, &agent_id);
    ctx.db.task_subagents().insert(TaskSubagent {
        task_id,
        agent_id,
        session_id,
        started_at,
    });
    sync_subagent_count(ctx, task_id)?;
    Ok(())
}

/// `HookSubagentStop` in `docs/specs/agent-health.allium`. An unrecognised
/// `agent_id` is a no-op, not an underflow — the delete simply matches
/// nothing, and the count is recomputed from the table either way.
#[spacetimedb::reducer]
pub fn subagent_stop(
    ctx: &ReducerContext,
    task_id: i64,
    agent_id: String,
    session_id: String,
) -> Result<(), String> {
    fence_subagent_session(ctx, task_id, &session_id);
    delete_subagent_entry(ctx, task_id, &agent_id);
    sync_subagent_count(ctx, task_id)?;
    apply_pending_stop_if_drained(ctx, task_id)?;
    Ok(())
}

/// Clear every `task_subagents` row for `task_id`, and apply any deferred
/// `Stop` this drains. For `DetachTmux` (`docs/specs/split-pane.allium`), the
/// one draining clear point that owns no status of its own.
#[spacetimedb::reducer]
pub fn subagent_clear(ctx: &ReducerContext, task_id: i64) -> Result<(), String> {
    delete_all_subagents(ctx, task_id);
    sync_subagent_count(ctx, task_id)?;
    apply_pending_stop_if_drained(ctx, task_id)?;
    Ok(())
}

/// [`subagent_clear`] minus the drain, plus voiding `stop_pending`
/// unconditionally. For the three non-draining clear points — `SessionStart`
/// (`ClearSubagentsOnSessionStart`), crash and dispatch-claim — which void a
/// deferred Stop rather than apply it. A missing task is a silent no-op for
/// the `stop_pending` write, matching the SQL's `UPDATE ... WHERE id = ?`.
#[spacetimedb::reducer]
pub fn subagent_clear_and_void_pending_stop(
    ctx: &ReducerContext,
    task_id: i64,
) -> Result<(), String> {
    delete_all_subagents(ctx, task_id);
    sync_subagent_count(ctx, task_id)?;
    if let Some(row) = ctx.db.tasks().id().find(task_id) {
        write_task(
            ctx,
            Task {
                stop_pending: false,
                ..row
            },
        )?;
    }
    Ok(())
}

/// `HookStop` in `docs/specs/agent-health.allium`. Refuses when the task is
/// not `Running` (including when it does not exist) rather than a silent
/// no-op: see this file's "Agent session state" section header for why that
/// is what lets the client read `Flipped` vs. `Deferred` back unambiguously.
/// `stop_pending_at` is the client's clock (millisecond precision, matching
/// `tasks.rs::try_record_stop`'s `format_datetime_millis`) — the value
/// `record_user_prompt_submit` later compares its own prompt time against, so
/// it must be an EVENT time, not this write's commit time.
#[spacetimedb::reducer]
pub fn try_record_stop(
    ctx: &ReducerContext,
    id: i64,
    stop_pending_at: String,
) -> Result<(), String> {
    let Some(row) = ctx.db.tasks().id().find(id) else {
        return Err(format!("task {id} not found"));
    };
    if row.status != RUNNING {
        return Err(format!("task {id} is not running"));
    }
    if row.live_subagents == 0 {
        flip_to_review(ctx, row)?;
    } else {
        write_task(
            ctx,
            Task {
                stop_pending: true,
                stop_pending_at,
                ..row
            },
        )?;
    }
    Ok(())
}

/// `HookPreToolUse` in `docs/specs/agent-health.allium`. A missing task, or
/// one that is not `Running`, is a silent no-op — matching the SQL `UPDATE
/// ... WHERE id = ? AND status = ?` that simply touches zero rows. `sub_status`
/// arrives already resolved: the classification (`classify_agent_activity`)
/// runs on the client, against a snapshot it already paid to read — this
/// reducer only applies the decision. `at` is the client's clock (second
/// precision, matching `tasks.rs::record_pre_tool_use`'s `format_datetime`).
#[spacetimedb::reducer]
pub fn record_pre_tool_use(
    ctx: &ReducerContext,
    id: i64,
    sub_status: String,
    at: String,
) -> Result<(), String> {
    let Some(row) = ctx.db.tasks().id().find(id) else {
        return Ok(());
    };
    if row.status != RUNNING {
        return Ok(());
    }
    write_task(
        ctx,
        Task {
            sub_status,
            last_pre_tool_use_at: at,
            ..row
        },
    )
}

/// `HookNotification` in `docs/specs/agent-health.allium`. `mode` arrives
/// already resolved from the notification kind — `NotificationWrite::from_kind`
/// runs on the client, same reasoning as `record_pre_tool_use`'s `sub_status`
/// — so this reducer only applies one of four already-decided writes. The
/// live-work predicate for `raise_if_no_own_work_live` is the one thing
/// evaluated HERE rather than on the client: it must read the row's committed
/// `live_subagents` at write time, not a snapshot that could be
/// stale by the time this reducer runs (`agent-health.allium: HookNotification`'s
/// "Evaluation time" guidance). `at` is the client's clock (second precision).
#[spacetimedb::reducer]
pub fn record_notification(
    ctx: &ReducerContext,
    id: i64,
    mode: String,
    at: String,
) -> Result<(), String> {
    if mode == "ignore" {
        return Ok(());
    }
    let Some(row) = ctx.db.tasks().id().find(id) else {
        return Ok(());
    };
    if row.status != RUNNING {
        return Ok(());
    }
    match mode.as_str() {
        "clear" => write_task(
            ctx,
            Task {
                sub_status: ACTIVE.into(),
                last_notification_at: String::new(),
                ..row
            },
        ),
        "raise" => write_task(
            ctx,
            Task {
                sub_status: NEEDS_INPUT.into(),
                last_notification_at: at,
                ..row
            },
        ),
        "raise_if_no_own_work_live" => {
            if row.live_subagents == 0 {
                write_task(
                    ctx,
                    Task {
                        sub_status: NEEDS_INPUT.into(),
                        last_notification_at: at,
                        ..row
                    },
                )
            } else {
                Ok(())
            }
        }
        other => Err(format!("unknown notification mode {other:?}")),
    }
}

/// `HookUserPromptSubmit` in `docs/specs/agent-health.allium`. Refuses when the
/// task is neither `Running` nor `Review` (including when it does not exist)
/// — see this file's "Agent session state" section header for why that is
/// what lets the client read `Resumed` vs. `Refreshed` back unambiguously.
///
/// `activity_at` (second precision) is what `last_pre_tool_use_at` takes;
/// `prompt_at` (millisecond precision) is compared against `stop_pending_at`
/// to decide whether to void it — mirroring `tasks.rs::record_user_prompt_submit`'s
/// own two-precision split of a single client `now`. Ties (equal timestamps)
/// preserve the bit; a `stop_pending_at` predating the field (the module's
/// empty-string sentinel) reads as "fired before any prompt" and is voided.
/// Both are EVENT times, not this write's commit time — see this file's
/// section header for why that matters here specifically.
#[spacetimedb::reducer]
pub fn record_user_prompt_submit(
    ctx: &ReducerContext,
    id: i64,
    activity_at: String,
    prompt_at: String,
) -> Result<(), String> {
    let Some(row) = ctx.db.tasks().id().find(id) else {
        return Err(format!("task {id} not found"));
    };
    if row.status != RUNNING && row.status != REVIEW {
        return Err(format!("task {id} is neither running nor in review"));
    }
    let resumed = row.status == REVIEW;
    let epic_id = row.epic_id;
    let void_pending_stop = row.stop_pending
        && (row.stop_pending_at.is_empty() || row.stop_pending_at.as_str() < prompt_at.as_str());
    write_task(
        ctx,
        Task {
            status: RUNNING.into(),
            sub_status: ACTIVE.into(),
            last_pre_tool_use_at: activity_at,
            stop_pending: if void_pending_stop {
                false
            } else {
                row.stop_pending
            },
            ..row
        },
    )?;
    if resumed {
        recalculate_epic_chain(ctx, epic_id);
    }
    Ok(())
}

/// Atomically set `pr_learnings_gate_shown_at` if it is not already set.
/// Refusal carries the "already shown or task missing" answer the same way
/// `claim_backlog_task` reports a lost race — the client reads `Applied` as
/// `true` ("this call set it, block the PR") and `Refused` as `false` via
/// `ReducerOutcome::won()`, no read-back needed. `at` is the client's clock.
#[spacetimedb::reducer]
pub fn mark_pr_learnings_gate_shown(
    ctx: &ReducerContext,
    id: i64,
    at: String,
) -> Result<(), String> {
    let Some(row) = ctx.db.tasks().id().find(id) else {
        return Err(format!("task {id} not found"));
    };
    if !row.pr_learnings_gate_shown_at.is_empty() {
        return Err(format!("task {id} has already shown the PR learnings gate"));
    }
    write_task(
        ctx,
        Task {
            pr_learnings_gate_shown_at: at,
            ..row
        },
    )
}

// -- Learnings (Phase 10, task #4914) ---------------------------------------

/// The fields a learning patch may change. Deliberately narrow, mirroring
/// `db::LearningPatch<'a>`'s own doc comment: `embedding` is the only field a
/// production caller writes (the startup backfill), and `status` exists for
/// `ArchiveStaleLearning`'s bulk sweep below and for tests seeding
/// archived/rejected rows directly. There is no field-editing path for a
/// learning's content.
///
/// `embedding` is doubly optional (`Patch<Option<Vec<u8>>>`), the same shape
/// `TaskPatch::sort_order` uses: the row's own `embedding` column is itself
/// `Option<Vec<u8>>`, so "do not touch" and "clear the stored embedding" are
/// two different `None`s. In practice only `Some(Some(bytes))` is ever sent —
/// nothing clears an embedding back to absent.
#[derive(spacetimedb::SpacetimeType, Clone, Debug, Default)]
pub struct LearningPatch {
    pub status: Patch<String>,
    pub summary: Patch<String>,
    pub embedding: Patch<Option<Vec<u8>>>,
}

/// Apply a learning patch. See [`apply_task_patch`].
fn apply_learning_patch(row: &mut Learning, patch: LearningPatch) {
    apply_patch!(row, patch, status, summary, embedding);
}

/// Create a learning. The row arrives with `id = 0`; the caller learns the
/// generated id by watching it arrive on its subscription, the same as
/// [`create_task`] — see that reducer's doc comment for why a reducer cannot
/// answer with it directly.
///
/// Enforces `docs/specs/learnings.allium`'s `ApprovedLearningsHaveScopeRef`
/// server-side, on the same reasoning `write_task` enforces
/// `OwnerTracksUserBoardTask`: a client-side check is a courtesy to a
/// well-behaved caller, and a shared table has more than one.
#[spacetimedb::reducer]
pub fn create_learning(ctx: &ReducerContext, row: Learning) -> Result<(), String> {
    validate_learning_scope(&row.scope, &row.scope_ref)?;
    ctx.db.learnings().insert(Learning {
        // Never trust an incoming id on a create — see `create_task`.
        id: 0,
        ..row
    });
    Ok(())
}

fn validate_learning_scope(scope: &str, scope_ref: &Option<String>) -> Result<(), String> {
    match (scope, scope_ref) {
        ("user", Some(_)) => Err("a user-scoped learning must not carry a scope_ref".to_string()),
        ("user", None) => Ok(()),
        (_, None) => Err(format!("a {scope}-scoped learning needs a scope_ref")),
        (_, Some(_)) => Ok(()),
    }
}

/// Change some fields of a learning. A missing learning is a silent no-op,
/// the same bargain [`patch_task`] makes.
#[spacetimedb::reducer]
pub fn patch_learning(ctx: &ReducerContext, id: i64, patch: LearningPatch) -> Result<(), String> {
    let Some(mut row) = ctx.db.learnings().id().find(id) else {
        return Ok(());
    };
    apply_learning_patch(&mut row, patch);
    row.updated_at = now(ctx);
    ctx.db.learnings().id().update(row);
    Ok(())
}

/// Delete a learning and the retrieval rows that only referred to it.
///
/// Unlike [`delete_task`], a missing id is refused rather than a silent
/// no-op: `DeleteLearningViaMcp` (`docs/specs/learnings.allium`) returns an
/// error for an id that was never created or was already deleted, and the
/// service layer's not-found mapping depends on that refusal reaching it.
/// The retrieval cascade runs in the same transaction as the delete,
/// reproduced as explicit reducer logic rather than a SQLite
/// `ON DELETE CASCADE` clause.
#[spacetimedb::reducer]
pub fn delete_learning(ctx: &ReducerContext, id: i64) -> Result<(), String> {
    if ctx.db.learnings().id().find(id).is_none() {
        return Err(format!("learning {id} not found"));
    }
    ctx.db.learnings().id().delete(id);
    let retrievals: Vec<i64> = ctx
        .db
        .learning_retrievals()
        .learning_id()
        .filter(&id)
        .map(|r| r.id)
        .collect();
    delete_learning_retrievals(ctx, retrievals);
    Ok(())
}

/// Re-scope every epic-scoped learning pointing at `from` to `to` instead.
///
/// Epic-shaped arguments, a `learnings` write — see `docs/conventions.md`'s
/// store seam section for why this sits here rather than being folded into
/// an epic reducer, and `ReScopeLearningsOnRepoGroupDelete` in
/// `docs/specs/learnings.allium` for the rule this implements. `scope_ref` is
/// not an embedding input, so no row's `embedding` is touched.
#[spacetimedb::reducer]
pub fn rescope_epic_learnings(ctx: &ReducerContext, from: i64, to: i64) -> Result<(), String> {
    let from_ref = from.to_string();
    let to_ref = to.to_string();
    update_matching_learnings(
        ctx,
        |l| l.scope == "epic" && l.scope_ref.as_deref() == Some(from_ref.as_str()),
        |row| Learning {
            scope_ref: Some(to_ref.clone()),
            ..row
        },
    );
    Ok(())
}

/// Record that `learning_id` was surfaced to `task_id` via `source`.
#[spacetimedb::reducer]
pub fn record_learning_retrieval(
    ctx: &ReducerContext,
    task_id: i64,
    learning_id: i64,
    source: String,
) -> Result<(), String> {
    ctx.db.learning_retrievals().insert(LearningRetrieval {
        task_id,
        learning_id,
        source,
        retrieved_at: now(ctx),
        ..blank_learning_retrieval()
    });
    Ok(())
}

/// One verdict in a batch — mirrors `db::LearningVerdict` (`helped` | `wrong`)
/// as an opaque string, the same treatment every other enum-typed column in
/// this module gets.
#[derive(spacetimedb::SpacetimeType, Clone, Debug)]
pub struct LearningVerdictInput {
    pub learning_id: i64,
    pub verdict: String,
}

/// Apply a batch of verdicts' in-flight score effects.
///
/// The verdict itself is not persisted — migration v74 dropped
/// `learning_verdicts`, and a verdict has never been more than this effect on
/// `Learning.upvote_count` (`docs/specs/learnings.allium`'s Retrievals &
/// Verdicts). A missing learning is skipped rather than failing the whole
/// batch: the retrieval precondition that makes a verdict valid is enforced
/// client-side by `LearningServiceApi::apply_verdicts` before this is ever
/// called, so a batch here has already passed that check.
#[spacetimedb::reducer]
pub fn apply_learning_verdicts(
    ctx: &ReducerContext,
    verdicts: Vec<LearningVerdictInput>,
) -> Result<(), String> {
    for v in verdicts {
        let Some(row) = ctx.db.learnings().id().find(v.learning_id) else {
            continue;
        };
        let delta: i64 = match v.verdict.as_str() {
            "helped" => 1,
            "wrong" => -1,
            other => return Err(format!("unknown verdict {other}")),
        };
        let now = now(ctx);
        let last_upvoted_at = if delta > 0 {
            Some(now.clone())
        } else {
            row.last_upvoted_at.clone()
        };
        ctx.db.learnings().id().update(Learning {
            upvote_count: row.upvote_count + delta,
            last_upvoted_at,
            updated_at: now,
            ..row
        });
    }
    Ok(())
}

/// Archive every approved learning with a non-positive score that has gone
/// untouched since before `cutoff`. Mirrors the SQLite bulk `UPDATE`
/// `ArchiveStaleLearning` (`docs/specs/learnings.allium`) ran directly;
/// returns the number of rows archived.
#[spacetimedb::reducer]
pub fn archive_stale_learnings(ctx: &ReducerContext, cutoff: String) -> Result<(), String> {
    let now = now(ctx);
    update_matching_learnings(
        ctx,
        |l| l.status == "approved" && l.upvote_count <= 0 && l.updated_at <= cutoff,
        |row| Learning {
            status: "archived".to_string(),
            updated_at: now.clone(),
            ..row
        },
    );
    Ok(())
}

// -- Usage events (Phase 11, task #4915) -------------------------------------

/// Record one usage event, then drop the oldest rows beyond `cap`.
///
/// The prune is the store's half of the cap the SQLite path enforced with a
/// `DELETE ... WHERE id <= MAX(id) - cap` in the same transaction as the
/// insert: here it runs in the same reducer, so no reader ever sees the table
/// over the cap. Ids are monotonic under `#[auto_inc]`, so "oldest" is
/// "lowest id", exactly as it was in SQLite.
///
/// No caller needs the new row's id — recording returns nothing — so there is
/// no read-back and nothing for `own_creations` to cover.
#[spacetimedb::reducer]
pub fn record_usage_event(ctx: &ReducerContext, row: UsageEvent, cap: i64) -> Result<(), String> {
    if cap <= 0 {
        return Err(format!("usage cap must be positive, got {cap}"));
    }
    // The inserted row's own id IS the highest id, because `#[auto_inc]`
    // never hands out anything but the next value — no need to scan the
    // table for a max that insert already told us.
    let inserted = ctx.db.usage_events().insert(UsageEvent {
        // Never trust an incoming id on a create — see `create_task`.
        id: 0,
        ..row
    });
    prune_usage_events(ctx, inserted.id, cap);
    Ok(())
}

fn prune_usage_events(ctx: &ReducerContext, highest: i64, cap: i64) {
    let threshold = highest - cap;
    if threshold <= 0 {
        return;
    }
    let stale: Vec<i64> = ctx
        .db
        .usage_events()
        .iter()
        .filter(|e| e.id <= threshold)
        .map(|e| e.id)
        .collect();
    for id in stale {
        ctx.db.usage_events().id().delete(id);
    }
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

    /// No children is NOT "all children done". An epic with nothing in it
    /// keeps whatever status it has — a freshly created one is backlog and
    /// must not be born done.
    #[test]
    fn an_epic_with_no_children_keeps_its_status() {
        assert_eq!(derive_epic_status("backlog", &children(&[])), None);
        assert_eq!(derive_epic_status("running", &children(&[])), None);
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
    // The dispatch claim (dispatch.allium: DispatchClaimExclusive)
    // -----------------------------------------------------------------

    /// A task nobody has dispatched belongs to whoever gets to it.
    #[test]
    fn an_unowned_task_is_claimable_by_anyone() {
        let task = blank_task();
        assert!(claimable_by(&task, "host-a"));
        assert!(claimable_by(&task, "host-b"));
    }

    /// A task whose worktree sits on another machine is not. Claiming it would
    /// dispatch an agent with nowhere to work.
    #[test]
    fn a_foreign_owned_task_is_passed_over() {
        let task = Task {
            host: "host-a".into(),
            ..blank_task()
        };
        assert!(claimable_by(&task, "host-a"));
        assert!(!claimable_by(&task, "host-b"));
    }

    // -----------------------------------------------------------------
    // Patches reach every field
    // -----------------------------------------------------------------
    //
    // THE GUARANTEE `apply_patch!` DOES NOT GIVE. Leaving a name out of an
    // invocation compiles: the patch field is `pub`, so there is no dead-field
    // warning, and that column just silently stops being patchable. Nothing
    // else in the suite would notice — a patch reducer that ignores one field
    // still returns Ok. So each of these builds a patch with EVERY field set to
    // a value the blank row does not hold, applies it, and asserts the blank is
    // gone everywhere. A column added to the struct and forgotten in the macro
    // fails here.

    /// A distinctive value for every `String` field, so "unchanged" is
    /// unmistakable.
    const MARK: &str = "patched";

    #[test]
    fn every_field_of_a_task_patch_reaches_the_row() {
        let mut row = blank_task();
        row.id = 7;
        let before = row.clone();

        apply_task_patch(
            &mut row,
            TaskPatch {
                title: Some(MARK.into()),
                description: Some(MARK.into()),
                repo_path: Some(MARK.into()),
                status: Some(DONE.into()),
                worktree: Some(MARK.into()),
                tmux_window: Some(MARK.into()),
                plan_path: Some(MARK.into()),
                epic_id: Some(9),
                sub_status: Some(MARK.into()),
                tag: Some(MARK.into()),
                sort_order: Some(Some(3)),
                base_branch: Some(MARK.into()),
                external_id: Some(MARK.into()),
                labels: Some("[\"x\"]".into()),
                last_pre_tool_use_at: Some(MARK.into()),
                last_notification_at: Some(MARK.into()),
                wrap_up_mode: Some(MARK.into()),
                url: Some(MARK.into()),
                url_type: Some(MARK.into()),
                pr_learnings_gate_shown_at: Some(MARK.into()),
                auto_run_plan: Some(true),
                live_subagents: Some(2),
                stop_pending: Some(true),
                stop_pending_at: Some(MARK.into()),
                last_peer_message_sent_at: Some(MARK.into()),
                last_peer_message_received_at: Some(MARK.into()),
                phoenix: Some(true),
                host: Some(MARK.into()),
                owner: Some(MARK.into()),
                completed_at: Some(MARK.into()),
            },
        );

        // The id is not patchable and must be untouched.
        assert_eq!(row.id, before.id);
        // Everything else moved.
        assert_eq!(row.title, MARK);
        assert_eq!(row.description, MARK);
        assert_eq!(row.repo_path, MARK);
        assert_eq!(row.status, DONE);
        assert_eq!(row.worktree, MARK);
        assert_eq!(row.tmux_window, MARK);
        assert_eq!(row.plan_path, MARK);
        assert_eq!(row.epic_id, 9);
        assert_eq!(row.sub_status, MARK);
        assert_eq!(row.tag, MARK);
        assert_eq!(row.sort_order, Some(3));
        assert_eq!(row.base_branch, MARK);
        assert_eq!(row.external_id, MARK);
        assert_eq!(row.labels, "[\"x\"]");
        assert_eq!(row.last_pre_tool_use_at, MARK);
        assert_eq!(row.last_notification_at, MARK);
        assert_eq!(row.wrap_up_mode, MARK);
        assert_eq!(row.url, MARK);
        assert_eq!(row.url_type, MARK);
        assert_eq!(row.pr_learnings_gate_shown_at, MARK);
        assert!(row.auto_run_plan);
        assert_eq!(row.live_subagents, 2);
        assert!(row.stop_pending);
        assert_eq!(row.stop_pending_at, MARK);
        assert_eq!(row.last_peer_message_sent_at, MARK);
        assert_eq!(row.last_peer_message_received_at, MARK);
        assert!(row.phoenix);
        assert_eq!(row.host, MARK);
        assert_eq!(row.owner, MARK);
        assert_eq!(row.completed_at, MARK);
        // created_at/updated_at are not patch fields; the reducer stamps them.
        assert_eq!(row.created_at, before.created_at);
    }

    #[test]
    fn every_field_of_an_epic_patch_reaches_the_row() {
        let mut row = blank_epic();
        row.id = 7;

        apply_epic_patch(
            &mut row,
            EpicPatch {
                title: Some(MARK.into()),
                description: Some(MARK.into()),
                status: Some(DONE.into()),
                plan_path: Some(MARK.into()),
                sort_order: Some(Some(3)),
                auto_dispatch: Some(true),
                parent_epic_id: Some(9),
                feed_command: Some(MARK.into()),
                feed_interval_secs: Some(60),
                group_by_repo: Some(true),
                feed_role: Some(MARK.into()),
                origin: Some(MARK.into()),
                feed_append_only: Some(true),
                completed_at: Some(MARK.into()),
            },
        );

        assert_eq!(row.id, 7);
        assert_eq!(row.title, MARK);
        assert_eq!(row.description, MARK);
        assert_eq!(row.status, DONE);
        assert_eq!(row.plan_path, MARK);
        assert_eq!(row.sort_order, Some(3));
        assert!(row.auto_dispatch);
        assert_eq!(row.parent_epic_id, 9);
        assert_eq!(row.feed_command, MARK);
        assert_eq!(row.feed_interval_secs, 60);
        assert!(row.group_by_repo);
        assert_eq!(row.feed_role, MARK);
        assert_eq!(row.origin, MARK);
        assert!(row.feed_append_only);
        assert_eq!(row.completed_at, MARK);
    }

    /// The other half: an EMPTY patch changes nothing at all. Without this, a
    /// macro arm that wrote unconditionally would pass the tests above.
    #[test]
    fn an_empty_patch_changes_no_field() {
        let mut row = blank_task();
        row.title = "original".into();
        row.epic_id = 4;
        row.host = "host-a".into();
        let before = row.clone();

        apply_task_patch(&mut row, TaskPatch::default());

        assert_eq!(row.title, before.title);
        assert_eq!(row.epic_id, before.epic_id);
        assert_eq!(row.host, before.host);
        assert_eq!(row.status, before.status);
        assert_eq!(row.sort_order, before.sort_order);
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
