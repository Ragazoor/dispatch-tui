//! The snapshot artefact: what a backup, a recovery and a seed all are.
//!
//! Spec: `docs/specs/spacetime-seed.allium` — `SharedTable`, `TableExtract`,
//! `Snapshot`, `RefusalReason` and `Refusal` all correspond one-to-one with
//! declarations there.

use serde::{Deserialize, Serialize};

/// Bumped when the snapshot layout changes in a way an older reader cannot
/// interpret. A reader refuses a version it does not know rather than guessing
/// — see [`crate::spacetime::restore`].
///
/// **Version 2**: every extract carries the names of its columns, and the
/// integer schema version the artefact used to record is gone. A format-1
/// snapshot is therefore refused, and that is deliberate rather than
/// collateral: it never recorded its columns, so there is nothing to check it
/// against under the new rule, and the alternatives are guessing or restoring
/// unchecked. See `spacetime-seed.allium`'s `config.snapshot_format_version`.
pub const SNAPSHOT_FORMAT_VERSION: u32 = 2;

/// How many tables a complete snapshot carries.
///
/// [`SharedTable::ALL`] is declared with this as its length, so a disagreement
/// is a compile error rather than a failing test.
///
/// **Neither guards the drift they are named for.** Adding a variant to
/// [`SharedTable`] without adding it to `ALL` compiles: the array stays the
/// old length, the count stays the old count, the dump never reads the new
/// table, and every backup taken afterwards silently omits it. The exhaustive
/// matches on the enum force you to *think about* a new variant; nothing
/// forces it into `ALL`. Deriving `ALL` from an exhaustive match would close
/// that, and is worth doing when the next table arrives (`poll_owners` was
/// the eleventh and still needed this done by hand; `learnings` and
/// `learning_retrievals`, the thirteenth and fourteenth, needed it again;
/// `usage_events`, the fifteenth, needed it once more; `retired_feed_items`,
/// the sixteenth, needed it yet again).
pub const SHARED_TABLE_COUNT: usize = 17;

/// One row, carried whole. Deliberately untyped: this module does not describe
/// the shape of a task row — `core.allium` does — and a second description here
/// would be a second thing to keep in step on every schema change, which is the
/// duplication the migration exists to remove.
pub type Row = serde_json::Map<String, serde_json::Value>;

/// The shared domain, table by table.
///
/// Serialised by its SQL name so a snapshot on disk is readable by a human
/// holding a broken board and a text editor, which is the situation this whole
/// subsystem is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedTable {
    Tasks,
    Epics,
    Todos,
    TaskWatchers,
    TaskShells,
    TaskSubagents,
    RepoPaths,
    RepoBaseBranches,
    Hosts,
    Subscriptions,
    PollOwners,
    Settings,
    FilterPresets,
    Learnings,
    LearningRetrievals,
    UsageEvents,
    RetiredFeedItems,
}

impl SharedTable {
    /// Every shared table. Iterated by the dump, so a variant added here is
    /// covered by the next backup without anything else changing.
    pub const ALL: [SharedTable; SHARED_TABLE_COUNT] = [
        SharedTable::Tasks,
        SharedTable::Epics,
        SharedTable::Todos,
        SharedTable::TaskWatchers,
        SharedTable::TaskShells,
        SharedTable::TaskSubagents,
        SharedTable::RepoPaths,
        SharedTable::RepoBaseBranches,
        SharedTable::Hosts,
        SharedTable::Subscriptions,
        SharedTable::PollOwners,
        SharedTable::Settings,
        SharedTable::FilterPresets,
        SharedTable::Learnings,
        SharedTable::LearningRetrievals,
        SharedTable::UsageEvents,
        SharedTable::RetiredFeedItems,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SharedTable::Tasks => "tasks",
            SharedTable::Epics => "epics",
            SharedTable::Todos => "todos",
            SharedTable::TaskWatchers => "task_watchers",
            SharedTable::TaskShells => "task_shells",
            SharedTable::TaskSubagents => "task_subagents",
            SharedTable::RepoPaths => "repo_paths",
            SharedTable::RepoBaseBranches => "repo_base_branches",
            SharedTable::Hosts => "hosts",
            SharedTable::Subscriptions => "subscriptions",
            SharedTable::PollOwners => "poll_owners",
            SharedTable::Settings => "settings",
            SharedTable::FilterPresets => "filter_presets",
            SharedTable::Learnings => "learnings",
            SharedTable::LearningRetrievals => "learning_retrievals",
            SharedTable::UsageEvents => "usage_events",
            SharedTable::RetiredFeedItems => "retired_feed_items",
        }
    }

    /// The column holding a generated id, for the tables that have one.
    ///
    /// Part of the shared domain is identified by something the store never
    /// generates — a live shell by its task and its own id, a host by the
    /// opaque id its machine minted. Those rows have nothing to preserve and
    /// nothing to burn.
    ///
    /// `repo_base_branches` is the one that reads the other way round: its
    /// domain identity is the `(repo_path, branch)` pair, but the table still
    /// carries a generated `id` as its primary key, so it does need burning.
    /// Identity in the domain and identity in the store are not the same
    /// question, and this method answers the second one.
    pub fn id_column(self) -> Option<&'static str> {
        match self {
            SharedTable::Tasks
            | SharedTable::Epics
            | SharedTable::Todos
            | SharedTable::TaskWatchers
            | SharedTable::RepoPaths
            | SharedTable::RepoBaseBranches
            | SharedTable::PollOwners
            | SharedTable::Learnings
            | SharedTable::LearningRetrievals
            | SharedTable::UsageEvents
            | SharedTable::RetiredFeedItems => Some("id"),
            SharedTable::TaskShells
            | SharedTable::TaskSubagents
            | SharedTable::Hosts
            | SharedTable::Subscriptions
            | SharedTable::Settings
            | SharedTable::FilterPresets => None,
        }
    }

    /// The columns the shared store types as booleans.
    ///
    /// **The snapshot's canonical representation of a boolean is `true`/`false`,
    /// and this is the list that makes it so.** SQLite has no boolean storage
    /// class — it keeps 0 and 1 — so a dump from the board has to convert, or
    /// the same board would produce two different files depending on which
    /// store dumped it, and a diff between a board backup and a server backup
    /// would show every boolean as changed.
    ///
    /// Declared here rather than read from SQLite's own column types, because
    /// those are not a guide: the same concept is spelled `BOOLEAN` on
    /// `tasks.auto_run_plan` and `INTEGER` on `tasks.stop_pending`.
    pub fn boolean_columns(self) -> &'static [&'static str] {
        match self {
            SharedTable::Tasks => &["auto_run_plan", "stop_pending", "phoenix"],
            SharedTable::Epics => &["auto_dispatch", "group_by_repo", "feed_append_only"],
            // `todos` is dumped as always-empty since #4970, so it never
            // passes through the SQLite read that applies this list.
            SharedTable::Todos
            | SharedTable::TaskWatchers
            | SharedTable::TaskShells
            | SharedTable::TaskSubagents
            | SharedTable::RepoPaths
            | SharedTable::RepoBaseBranches
            | SharedTable::Hosts
            | SharedTable::Subscriptions
            | SharedTable::PollOwners
            | SharedTable::Settings
            | SharedTable::FilterPresets
            | SharedTable::Learnings
            | SharedTable::LearningRetrievals
            | SharedTable::UsageEvents
            | SharedTable::RetiredFeedItems => &[],
        }
    }

    /// Columns the module stores as non-optional although the domain treats
    /// them as absent-able, with the value that stands for absent.
    ///
    /// # Why any of this exists
    ///
    /// **SpacetimeDB SQL cannot filter on an optional column.** Not through a
    /// syntax this code gets wrong — there is no syntax. An `Option<T>` is a
    /// SATS *sum type*, and the SQL reference says the language "does not
    /// provide a way to construct them, nore does it provide any scalar
    /// operators for them" (<https://spacetimedb.com/docs/reference/sql/>). A
    /// `WHERE owner = '...'` against an optional column is refused with
    /// "cannot be parsed as type `(some: String | none: ())`", and `IS NULL`,
    /// `!= none` and `some('x')` are all refused too.
    ///
    /// A subscription is a `WHERE` clause. So a column nobody can filter on is
    /// a column no board can subscribe by — and subscribing is the whole
    /// mechanism that keeps a colleague's private work off this machine
    /// (`sync.allium: SendsOnlyWhatWasSubscribedTo`).
    ///
    /// # Why so many
    ///
    /// Only `tasks.owner` and `tasks.epic_id` are filtered on today. Every
    /// other column here is future-proofing, and the reason to do it now is
    /// that **changing a column's type is not automigratable**: it is free
    /// while no server exists and a manual migration afterwards. `tasks.host`
    /// is the clearest case of a filter this list anticipates — host-scoped PR
    /// polling needs exactly that `WHERE`.
    ///
    /// # What is deliberately NOT here
    ///
    /// `sort_order`, on both `tasks` and `epics`. Zero is a real sort order
    /// that this codebase actually writes, and null means something ELSE
    /// entirely: the read path orders by `COALESCE(sort_order, id)`, so null
    /// says "fall back to the id" rather than "sort me first". A sentinel would
    /// silently reorder cards. Nothing would ever subscribe by sort order, so
    /// the exception costs nothing.
    ///
    /// # Why each sentinel is safe
    ///
    /// Every value here is unreachable as a real one, and by construction
    /// rather than by convention:
    ///
    /// - [`Sentinel::Zero`] for id references. `#[auto_inc]` treats 0 as "no id
    ///   supplied" — the mechanism the whole seed-and-burn path is built on —
    ///   so a real id is 1 or above. Zero is already this store's word for "no
    ///   id here". Also `epics.feed_interval_secs`, where
    ///   `models::MIN_FEED_INTERVAL_SECS` is 60 and migration v91 clamps
    ///   anything below it.
    /// - [`Sentinel::EmptyString`] for everything else. Paths, timestamps,
    ///   urls, tags and identities have no meaningful empty value, and
    ///   `subscriptions.subscriber` already works exactly this way.
    ///   `hosts.label` is the one that leans on a rule rather than on the type:
    ///   `host.allium: RenameHost` refuses an empty or whitespace-only label,
    ///   which is what keeps `""` distinguishable from a name somebody chose.
    ///
    /// # The failure this invites
    ///
    /// A reader that skips the mapping sees `epic_id = 0` as "belongs to epic
    /// 0" and `owner = ""` as "owned by the empty person". Both look plausible
    /// and are wrong. The defence is that this list has exactly two consumers
    /// and they are the two ends of one conversion, both in
    /// `src/spacetime/cli_store.rs` — the encode on the way in and the decode
    /// on the way out — plus the parity test in
    /// `src/spacetime/tests/module_schema.rs`, which reads it to know which
    /// non-optional module columns are deliberately so.
    pub fn sentinel_columns(self) -> &'static [(&'static str, Sentinel)] {
        use Sentinel::{EmptyString as S, Zero as Z};
        match self {
            SharedTable::Tasks => &[
                ("worktree", S),
                ("tmux_window", S),
                ("plan_path", S),
                ("epic_id", Z),
                ("tag", S),
                ("external_id", S),
                ("last_pre_tool_use_at", S),
                ("last_notification_at", S),
                ("wrap_up_mode", S),
                ("url", S),
                ("url_type", S),
                ("pr_learnings_gate_shown_at", S),
                ("stop_pending_at", S),
                ("live_shells", Z),
                ("oldest_live_shell_started_at", S),
                ("last_peer_message_sent_at", S),
                ("last_peer_message_received_at", S),
                ("host", S),
                ("owner", S),
                // The Done column's ordering key. A timestamp has no
                // meaningful empty value, so "" is unreachable as a real one.
                ("completed_at", S),
                // Who created the row (`core.allium: Task.created_by`). An
                // identity has no meaningful empty value, same as `owner`.
                ("created_by", S),
            ],
            SharedTable::Epics => &[
                ("plan_path", S),
                ("parent_epic_id", Z),
                ("feed_command", S),
                ("feed_interval_secs", Z),
                ("completed_at", S),
                // The epic twin of `Task.created_by` above.
                ("created_by", S),
            ],
            SharedTable::Todos => &[
                ("task_id", Z),
                ("epic_id", Z),
                ("parent_id", Z),
                ("owner", S),
            ],
            SharedTable::RepoPaths => &[("verify_command", S)],
            SharedTable::Hosts => &[("label", S), ("owner", S)],
            // Every column is already required on these four. PollOwners in
            // particular has nothing optional to begin with: a row that
            // exists always carries all four fields, unlike e.g. `hosts`
            // where `label` is unset until a human names the machine.
            SharedTable::TaskWatchers
            | SharedTable::TaskShells
            | SharedTable::TaskSubagents
            | SharedTable::RepoBaseBranches
            | SharedTable::Subscriptions
            | SharedTable::PollOwners
            | SharedTable::Settings
            | SharedTable::FilterPresets
            | SharedTable::Learnings
            | SharedTable::LearningRetrievals
            | SharedTable::UsageEvents
            // `retired_feed_items.feed_epic_id` is already required on both
            // sides — nothing here to sentinel.
            | SharedTable::RetiredFeedItems => &[],
        }
    }

    /// The sentinel for `column`, or `None` if it carries no sentinel.
    pub fn sentinel_for(self, column: &str) -> Option<Sentinel> {
        self.sentinel_columns()
            .iter()
            .find(|(name, _)| *name == column)
            .map(|(_, sentinel)| *sentinel)
    }

    /// For a table with no SQLite counterpart, the columns its row is assembled
    /// from — in module order — paired with the `settings` key each reads.
    ///
    /// `hosts` is the only such table: this install's own identity lives in
    /// `settings` rather than in a table of its own, so the dump builds the row
    /// rather than selecting it.
    ///
    /// **Declared as data because the positional module-vs-SQLite check cannot
    /// cover it.** That check compares each shared table against its SQLite
    /// counterpart, and skips the tables that have none — which leaves the one
    /// table whose row is assembled by hand as the only one nothing verifies.
    /// Listing the columns here lets the same test compare THIS against the
    /// module instead, so appending a column to `hosts` and forgetting the dump
    /// fails at the module change rather than mid-restore, during the incident
    /// the snapshot exists for.
    ///
    /// An empty slice means "assembled from nothing" — i.e. a SQLite-backed
    /// table, which is read rather than assembled.
    /// **Doubles as the expected column list for every always-empty table**
    /// (`dump::Source::Empty`: `poll_owners`, and the dead `task_shells`,
    /// `todos` and `filter_presets`), even though nothing is actually assembled from a settings
    /// key the way `hosts` is — `dump::Source::Empty` reads no settings row at all, and
    /// the second element of each pair is unused there. It is still the
    /// right home for the list: this method's whole job, per the schema
    /// parity test (`src/spacetime/tests/module_schema.rs`), is "the expected
    /// columns for a table `dump::is_sqlite_backed` says has no real SQLite
    /// source", and `poll_owners` is exactly that — it just has a different
    /// reason (see the table's own doc comment in `spacetime/module/src/lib.rs`)
    /// than `hosts` does for being one.
    pub fn assembled_columns(self) -> &'static [(&'static str, &'static str)] {
        match self {
            SharedTable::Hosts => &[
                ("id", crate::db::HOST_ID_KEY),
                ("label", crate::db::HOST_LABEL_KEY),
                ("owner", crate::db::USER_IDENTITY_KEY),
            ],
            SharedTable::PollOwners => &[
                ("id", ""),
                ("scope", ""),
                ("scope_id", ""),
                ("host", ""),
                ("claimed_at", ""),
            ],
            // #4965 dropped SQLite's `task_shells` along with the
            // shell-tracking feature it backed, so a local dump can no
            // longer read this table — it joins `poll_owners` as always
            // empty. The second element of each pair is unused, exactly as
            // it is for `poll_owners`.
            // #4970 dropped SQLite's `todos` with the TODO overlay; same
            // treatment as `task_shells` above.
            SharedTable::Todos => &[
                ("id", ""),
                ("title", ""),
                ("done", ""),
                ("sort_order", ""),
                ("created_at", ""),
                ("task_id", ""),
                ("epic_id", ""),
                ("parent_id", ""),
                ("owner", ""),
            ],
            SharedTable::TaskShells => &[
                ("task_id", ""),
                ("shell_id", ""),
                ("session_id", ""),
                ("started_at", ""),
            ],
            // Assembled rather than read: a local `settings` row has no
            // `host` column of its own (there is exactly one machine per
            // SQLite file, so the value has always been implicit), and `id`
            // is derived (`"{host}/{key}"`), not stored anywhere locally. See
            // `dump::read_local_settings`.
            SharedTable::Settings => &[("id", ""), ("host", ""), ("key", ""), ("value", "")],
            // #4972 dropped SQLite's `filter_presets` with saved repo-filter
            // presets; same treatment as `task_shells` above.
            SharedTable::FilterPresets => &[
                ("id", ""),
                ("host", ""),
                ("name", ""),
                ("repo_paths", ""),
                ("mode", ""),
            ],
            SharedTable::Tasks
            | SharedTable::Epics
            | SharedTable::TaskWatchers
            | SharedTable::TaskSubagents
            | SharedTable::RepoPaths
            | SharedTable::RepoBaseBranches
            | SharedTable::Subscriptions
            | SharedTable::Learnings
            | SharedTable::LearningRetrievals
            | SharedTable::UsageEvents
            | SharedTable::RetiredFeedItems => &[],
        }
    }

    /// Columns the SpacetimeDB module carries that SQLite has no counterpart
    /// for, in the order the module appends them.
    ///
    /// The migration adds shared-board concepts a single-user SQLite board never
    /// needed, so the two schemas are not identical and the difference has to be
    /// written down somewhere. Here, beside [`Self::boolean_columns`], because
    /// it is the same shape of knowledge and because two readers need it: the
    /// parity test in `src/spacetime/tests/module_schema.rs`, and the seeding
    /// client that must supply a value for each one
    /// (`spacetime-seed.allium: BackfillTaskOwner`).
    ///
    /// An exhaustive match, so an eleventh table cannot skip the question.
    pub fn module_only_columns(self) -> &'static [&'static str] {
        match self {
            // `owner` is the user board an epic-less task sits on
            // (`core.allium: OwnerTracksUserBoardTask`); `created_by` is who
            // created the task regardless of epic membership (`core.allium:
            // Task.created_by`). `live_shells`/`oldest_live_shell_started_at`
            // are dead columns #4965 dropped from SQLite (see the `Task`
            // struct's doc comment in `spacetime/module/src/lib.rs`) — they
            // are module-only now for a different reason than the other two,
            // but the effect on this list is the same.
            SharedTable::Tasks => &[
                "owner",
                "created_by",
                "live_shells",
                "oldest_live_shell_started_at",
            ],
            // Who created the epic (`core.allium: Epic.created_by`) — an
            // epic has no `owner` counterpart, since nobody's user board
            // holds one.
            SharedTable::Epics => &["created_by"],
            SharedTable::Todos
            | SharedTable::TaskWatchers
            | SharedTable::TaskShells
            | SharedTable::TaskSubagents
            | SharedTable::RepoPaths
            | SharedTable::RepoBaseBranches
            // None of these exist in SQLite at all, so none has a column to
            // reconcile. See `dump::is_sqlite_backed`.
            | SharedTable::Hosts
            | SharedTable::Subscriptions
            | SharedTable::PollOwners
            | SharedTable::Settings
            | SharedTable::FilterPresets
            // `learnings`/`learning_retrievals`/`usage_events`/
            // `retired_feed_items` are SQLite-backed and their module column
            // order matches their SQLite column order exactly (see the
            // module's own doc comment), so none has a module-only column to
            // reconcile.
            | SharedTable::Learnings
            | SharedTable::LearningRetrievals
            | SharedTable::UsageEvents
            | SharedTable::RetiredFeedItems => &[],
        }
    }

    /// Whether this table's identity comes from a counter the store advances,
    /// and therefore whether it needs burning after a restore.
    pub fn generates_ids(self) -> bool {
        self.id_column().is_some()
    }
}

/// The value a non-optional module column uses to mean "absent".
///
/// Two, because the columns are of two kinds: an id reference and everything
/// else. See [`SharedTable::sentinel_columns`] for why either is unreachable as
/// a real value, and for why this exists at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sentinel {
    /// `""`. Paths, timestamps, urls, tags, labels and identities.
    EmptyString,
    /// `0`. Id references, and `epics.feed_interval_secs`.
    Zero,
}

impl Sentinel {
    /// This sentinel as it appears in a snapshot row.
    pub fn as_json(self) -> serde_json::Value {
        match self {
            Self::EmptyString => serde_json::Value::String(String::new()),
            Self::Zero => serde_json::Value::from(0),
        }
    }

    /// Whether `value` is this sentinel, i.e. whether it means "absent".
    pub fn matches(self, value: &serde_json::Value) -> bool {
        match self {
            Self::EmptyString => value.as_str() == Some(""),
            Self::Zero => value.as_i64() == Some(0),
        }
    }
}

/// One table's contribution to a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableExtract {
    pub table: SharedTable,
    /// The names of the columns `rows` carry.
    ///
    /// **Recorded rather than derived from the rows**, because a table with no
    /// rows has nothing to derive them from — and a reader that had nothing to
    /// say about an empty table would have to treat "unknown" as "fine", which
    /// is the silent restore the schema check exists to prevent. This is what
    /// a restore compares against the store's own columns; see
    /// [`crate::spacetime::restore`].
    ///
    /// It sits here rather than as one schema block on [`Snapshot`] because the
    /// extract already names its table, and the question at restore time is
    /// asked per table: can these rows go into this table?
    pub columns: Vec<String>,
    pub rows: Vec<Row>,
}

impl TableExtract {
    pub fn new(table: SharedTable, columns: Vec<String>, rows: Vec<Row>) -> Self {
        Self {
            table,
            columns,
            rows,
        }
    }

    /// A table that has no rows but still declares its schema. An extract with
    /// no columns makes no claim at all, and "no claim" must never read as
    /// "matches" — see `spacetime-seed.allium`:
    /// `TableExtract.EveryExtractNamesItsColumns`.
    pub fn empty(table: SharedTable, columns: Vec<String>) -> Self {
        Self::new(table, columns, Vec::new())
    }

    /// The columns this extract and `other` do not share, in either direction,
    /// as sets.
    ///
    /// **A set, not a sequence.** A restore writes rows by column name, so the
    /// order the columns are listed in cannot change the outcome, and refusing
    /// over it would refuse a restore that was going to be correct. That is a
    /// different question from the append-only column ORDER the shared store's
    /// own migrations require, which governs how its schema may evolve rather
    /// than whether a given set of rows can be written. See
    /// `src/spacetime/tests/module_schema.rs`, which is positional for that
    /// reason.
    pub fn columns_differing_from(&self, other: &[String]) -> Vec<String> {
        let ours: std::collections::BTreeSet<&str> =
            self.columns.iter().map(String::as_str).collect();
        let theirs: std::collections::BTreeSet<&str> = other.iter().map(String::as_str).collect();
        ours.symmetric_difference(&theirs)
            .map(|s| (*s).to_owned())
            .collect()
    }

    /// The generated id of each row, in row order. Empty for a table whose
    /// identity is not generated — see [`SharedTable::id_column`].
    ///
    /// A row in a generating table whose id is absent or non-integer is skipped
    /// rather than defaulted to zero: zero is the value that *asks* the store
    /// for a fresh id, so defaulting to it here would turn a malformed row into
    /// a silent renumbering.
    pub fn row_ids(&self) -> Vec<i64> {
        let Some(column) = self.table.id_column() else {
            return Vec::new();
        };
        self.rows
            .iter()
            .filter_map(|row| row.get(column).and_then(serde_json::Value::as_i64))
            .collect()
    }

    /// The largest id present, or 0 when the table is empty or does not
    /// generate ids. Zero is the right answer for "nothing to clear": a counter
    /// starting at 1 is already past it, so the burn is a no-op rather than a
    /// special case.
    pub fn highest_id(&self) -> i64 {
        let Some(column) = self.table.id_column() else {
            return 0;
        };
        self.rows
            .iter()
            .filter_map(|row| row.get(column).and_then(serde_json::Value::as_i64))
            .max()
            .unwrap_or(0)
    }
}

/// A snapshot of the entire shared domain at one moment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// The snapshot format itself.
    pub format_version: u32,
    /// When the read happened. Metadata for the human holding the file; nothing
    /// branches on it.
    pub taken_at: String,
    extracts: Vec<TableExtract>,
}

impl Snapshot {
    /// Stamps `taken_at` itself, so the two dump paths cannot end up spelling
    /// the timestamp in different formats.
    pub fn new(extracts: Vec<TableExtract>) -> Self {
        Self {
            format_version: SNAPSHOT_FORMAT_VERSION,
            taken_at: chrono::Utc::now().to_rfc3339(),
            extracts,
        }
    }

    pub fn extracts(&self) -> &[TableExtract] {
        &self.extracts
    }

    /// Mutable access for the seed's backfills, which amend the extracts
    /// before a restore (`spacetime-seed.allium`: `BackfillTaskOwner`).
    pub(crate) fn extracts_mut(&mut self) -> &mut [TableExtract] {
        &mut self.extracts
    }

    pub fn extract(&self, table: SharedTable) -> Option<&TableExtract> {
        self.extracts.iter().find(|e| e.table == table)
    }

    pub fn highest_id(&self, table: SharedTable) -> i64 {
        self.extract(table).map_or(0, TableExtract::highest_id)
    }

    /// Rows in a deterministic order, for comparing two snapshots. A dump does
    /// not promise row order, so comparing snapshots directly would report a
    /// difference that is not one.
    pub fn canonical_rows(&self) -> Vec<(SharedTable, Vec<String>)> {
        let mut out: Vec<(SharedTable, Vec<String>)> = self
            .extracts
            .iter()
            .map(|extract| {
                let mut rows: Vec<String> = extract
                    .rows
                    .iter()
                    .map(|row| serde_json::to_string(row).unwrap_or_default())
                    .collect();
                rows.sort();
                (extract.table, rows)
            })
            .collect();
        out.sort_by_key(|(table, _)| *table);
        out
    }

    /// Why this snapshot cannot be restored, if it cannot. Checked before the
    /// first row is written, so an operator who sees a refusal knows the store
    /// is untouched.
    pub fn completeness_refusal(&self) -> Option<Refusal> {
        let mut seen: Vec<SharedTable> = Vec::new();
        for extract in &self.extracts {
            if seen.contains(&extract.table) {
                return Some(Refusal::new(
                    RefusalReason::Incomplete,
                    format!("table {} appears more than once", extract.table.name()),
                ));
            }
            seen.push(extract.table);
        }
        let missing: Vec<&str> = SharedTable::ALL
            .iter()
            .filter(|t| !seen.contains(t))
            .map(|t| t.name())
            .collect();
        if !missing.is_empty() {
            return Some(Refusal::new(
                RefusalReason::Incomplete,
                format!("snapshot is missing table(s): {}", missing.join(", ")),
            ));
        }
        None
    }

    /// Remove a table's extract. Test scaffolding for the refusal path: a
    /// snapshot this malformed cannot be produced by [`super::dump_from_sqlite`].
    #[cfg(any(test, feature = "test-support"))]
    pub fn drop_extract(&mut self, table: SharedTable) {
        self.extracts.retain(|e| e.table != table);
    }

    /// Rewrite one task's id. Test scaffolding for the implausible-ceiling
    /// refusal, which needs a snapshot no dump would produce.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_task_id_for_test(&mut self, from: i64, to: i64) {
        for extract in &mut self.extracts {
            if extract.table != SharedTable::Tasks {
                continue;
            }
            for row in &mut extract.rows {
                if row.get("id").and_then(serde_json::Value::as_i64) == Some(from) {
                    row.insert("id".into(), serde_json::Value::from(to));
                }
            }
        }
    }

    /// Add a column to a table's extract without adding it to any row. Test
    /// scaffolding for the schema refusal: what it models is a snapshot taken
    /// from a schema this store does not have, which no dump of THIS store
    /// could produce.
    #[cfg(any(test, feature = "test-support"))]
    pub fn add_column_for_test(&mut self, table: SharedTable, column: &str) {
        for extract in &mut self.extracts {
            if extract.table == table {
                extract.columns.push(column.to_owned());
            }
        }
    }

    /// Duplicate a table's extract, for the same reason as [`Self::drop_extract`].
    #[cfg(any(test, feature = "test-support"))]
    pub fn duplicate_extract_for_test(&mut self, table: SharedTable) {
        if let Some(extract) = self.extract(table).cloned() {
            self.extracts.push(extract);
        }
    }
}

/// Why a restore refused. Every arm refuses to write anything at all: a
/// partially-restored store is worse than an untouched one, because it looks
/// like a board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalReason {
    /// Written by a version of the tool this one cannot read.
    FormatUnsupported,
    /// The rows describe a schema this store does not have.
    SchemaMismatch,
    /// The snapshot does not cover every shared table exactly once.
    Incomplete,
    /// A seed was asked of a store that already holds tasks or epics.
    StoreNotEmpty,
    /// A task or epic row still carries the retired `archived` status.
    ArchivedRowsPresent,
    /// An import found a task, epic or learning whose id the target already
    /// gives to a different row.
    IdConflict,
}

/// What an operator is told when a restore refuses.
///
/// The reason is a closed set so a caller can branch on it; the detail is free
/// text so a human can act on it. "Restore failed" is not an acceptable message
/// for an operation somebody reaches for during an incident.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub reason: RefusalReason,
    pub detail: String,
}

impl Refusal {
    pub fn new(reason: RefusalReason, detail: impl Into<String>) -> Self {
        Self {
            reason,
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = match self.reason {
            RefusalReason::FormatUnsupported => "unsupported snapshot format",
            RefusalReason::SchemaMismatch => "schema mismatch",
            RefusalReason::Incomplete => "incomplete snapshot",
            RefusalReason::StoreNotEmpty => "store not empty",
            RefusalReason::ArchivedRowsPresent => "archived rows present",
            RefusalReason::IdConflict => "id conflict",
        };
        write!(f, "{reason}: {}", self.detail)
    }
}

impl std::error::Error for Refusal {}
