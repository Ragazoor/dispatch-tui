//! Table definitions: every shared table, in the column order its SQLite original has.

// Tables

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

pub(crate) const POLL_SCOPE_TASK: &str = "task";

pub(crate) const POLL_SCOPE_EPIC: &str = "epic";

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
