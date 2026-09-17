//! The shared store as this subsystem needs to see it, and an in-process model
//! of it.
//!
//! Spec: `docs/specs/spacetime-seed.allium` — `IdSequence` and the
//! `BurnIdSequences` rule.

use anyhow::Result;
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::Mutex;

use super::snapshot::{Row, SharedTable, Snapshot, TableExtract};

/// What a restore needs from the store it is restoring into.
///
/// Narrow on purpose. This is not an interface to the shared domain — it is the
/// handful of operations the escape hatch performs, so that the hatch can be
/// exercised without a server and so that the real client has a small, obvious
/// surface to satisfy.
///
/// **A restore is resumable, not atomic, and the difference is deliberate.**
/// There is no all-or-nothing seam here because no implementation could honour
/// one: rows are chunked into several reducer calls per table to fit the
/// command line, so a restore is many transactions however it is written. What
/// makes a half-finished restore safe is a different property — every write
/// matches on the key, and the burn is a no-op once the counter is clear — so
/// running it again finishes the job rather than duplicating it. That is what
/// the idempotency tests pin.
#[async_trait]
pub trait SharedStore: Send + Sync {
    /// The schema version this store holds, so a restore can refuse rows that
    /// describe a different one.
    async fn schema_version(&self) -> Result<i64>;

    /// Write rows, matching on the table's key columns. A row whose key is
    /// already present is overwritten with this version of it; a row whose key
    /// is not present is inserted. **Ids are written as given** — never
    /// remapped, never renumbered, never densified.
    ///
    /// This is what makes a second restore a no-op and a restore interrupted
    /// halfway safe to run again.
    async fn upsert_rows(&self, table: SharedTable, rows: &[Row]) -> Result<()>;

    /// Leave the table's id counter strictly past `ceiling`.
    ///
    /// Named for the obligation, not the technique. The store generates an id
    /// only when the caller supplies none, so restoring explicit ids does not
    /// move the counter and there is no operation that sets one; the only way
    /// to advance it is to make the store generate values and discard them.
    /// An implementation that meets the obligation some other way is equally
    /// correct — what is not correct is reporting success without meeting it.
    ///
    /// **This runs BEFORE the rows are written, and the order is not a
    /// preference.** The throwaway inserts ask the store to generate 1, 2, 3 and
    /// so on — precisely the ids a restore is about to write. Burn afterwards
    /// and every one of those inserts lands on an existing primary key; the
    /// store rejects it, and a reducer whose insert is rejected aborts. There is
    /// no way to advance the counter past a row without generating that row's
    /// id, so burning a table that already holds low ids is not slow, it is
    /// impossible. Burning first, against an empty table, has neither problem.
    ///
    /// (The migration design says "burn the counter after loading". That is
    /// wrong for the reason above, found while building this. See the
    /// `BurnIdSequences` rule in `spacetime-seed.allium`, which records the
    /// corrected order.)
    ///
    /// **Already past `ceiling` still costs exactly one throwaway id**, not
    /// zero and not a loop that spins. Nothing exposes the counter's value, so
    /// the only way to learn it is to generate one — which is why a second
    /// restore of the same snapshot is cheap but not free, and why the counter
    /// creeps by one each time. Verified on a live instance: see the
    /// "SpacetimeDB" section of `docs/reference.md`.
    async fn advance_id_sequence_past(&self, table: SharedTable, ceiling: i64) -> Result<()>;

    /// Every row of a table, for re-dumping and for tests.
    async fn rows(&self, table: SharedTable) -> Result<Vec<Row>>;

    /// The generated ids present in a table, ascending.
    async fn row_ids(&self, table: SharedTable) -> Result<Vec<i64>> {
        let rows = self.rows(table).await?;
        let mut ids = TableExtract::new(table, rows).row_ids();
        ids.sort_unstable();
        Ok(ids)
    }

    /// Read the whole shared domain back out. The other half of the round trip:
    /// a backup you cannot re-dump is a backup you cannot verify.
    async fn dump(&self, schema_version: i64) -> Result<Snapshot> {
        let mut extracts = Vec::with_capacity(SharedTable::ALL.len());
        for table in SharedTable::ALL {
            extracts.push(TableExtract::new(table, self.rows(table).await?));
        }
        Ok(Snapshot::new(schema_version, extracts))
    }
}

/// The columns that together identify a row, as this fake keys them.
///
/// **Private to the fake on purpose.** The real store matches on the primary
/// key inside its own reducers and never consults this, so putting it on the
/// shared `SharedTable` vocabulary would describe a decision nothing in
/// production reads.
///
/// Secondary uniqueness is deliberately not modelled: `repo_base_branches` is
/// also unique on `(repo_path, branch)` and `task_watchers` on
/// `(watcher, target)`, so restoring a row whose primary key is free but whose
/// pair is taken would still be rejected by the real store. That cannot happen
/// restoring into an empty store, which is what a seed and a rebuild both are.
fn key_columns(table: SharedTable) -> &'static [&'static str] {
    match table {
        SharedTable::Tasks
        | SharedTable::Epics
        | SharedTable::Todos
        | SharedTable::TaskWatchers
        | SharedTable::RepoPaths
        | SharedTable::RepoBaseBranches
        | SharedTable::Hosts
        | SharedTable::Subscriptions => &["id"],
        SharedTable::TaskShells => &["task_id", "shell_id"],
        SharedTable::TaskSubagents => &["task_id", "agent_id"],
    }
}

/// A row's identity, as the table's key columns describe it.
///
/// Rendered as a sortable string rather than a tuple of values so one map type
/// serves both a single integer id and a two-column composite. Integers are
/// zero-padded so iteration is numeric rather than lexical — a convenience when
/// reading a failure message, not something any assertion depends on (every
/// consumer of `rows` sorts).
fn row_key(table: SharedTable, row: &Row) -> String {
    key_columns(table)
        .iter()
        .map(|column| match row.get(*column) {
            Some(serde_json::Value::Number(n)) => n
                .as_i64()
                .map_or_else(|| n.to_string(), |i| format!("{i:020}")),
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        })
        .collect::<Vec<_>>()
        .join("\u{1}")
}

/// An in-process model of the shared store, used by the tests.
///
/// It models exactly one rule of the real thing, and that rule is the whole
/// reason this subsystem exists:
///
/// > A generated id is assigned **only when the column is zero**. Inserting an
/// > explicit non-zero id does not advance the sequence.
///
/// Because the model is code we wrote, a test passing against it is evidence
/// about *our* side of the contract — that the restore burns, burns every
/// generating table, burns past rather than up to the ceiling, and does not
/// walk the counter backwards — and not evidence about the real store's
/// behaviour. The rule above was checked against a live instance; see the
/// "SpacetimeDB" section of `docs/reference.md`.
#[derive(Debug, Default)]
pub struct MemoryStore {
    state: Mutex<MemoryState>,
}

#[derive(Debug, Default)]
struct MemoryState {
    /// Rows keyed by identity, so "write matching by key" is the only
    /// behaviour available. An accidental append would otherwise read as a
    /// working restore right up until somebody counts.
    rows: BTreeMap<SharedTable, BTreeMap<String, Row>>,
    /// The id the next generated insert receives. Starts at 1, like the real
    /// store's.
    next_id: BTreeMap<SharedTable, i64>,
    schema_version: i64,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_schema_version(schema_version: i64) -> Self {
        let store = Self::new();
        store.set_schema_version(schema_version);
        store
    }

    pub fn set_schema_version(&self, schema_version: i64) {
        self.lock().schema_version = schema_version;
    }

    /// The id the next generated insert will receive.
    pub fn next_generated_id(&self, table: SharedTable) -> i64 {
        self.lock().next_id.get(&table).copied().unwrap_or(1)
    }

    /// Insert a row the way the board does — asking the store for an id rather
    /// than supplying one — and return the id it was given.
    ///
    /// This is the operation the load-bearing test uses. Asserting on the burn
    /// loop would prove the code calls itself; asserting on what this returns
    /// proves the property.
    ///
    /// **Fails when the generated id is already taken**, because that is what
    /// the real store does: the id is a primary key, and a reducer whose insert
    /// violates it aborts. Modelled rather than glossed over, because it is the
    /// constraint that decides the order of a restore — see
    /// [`SharedStore::advance_id_sequence_past`].
    pub async fn insert_generating_id(&self, table: SharedTable) -> Result<i64> {
        let Some(column) = table.id_column() else {
            anyhow::bail!("{} does not generate ids", table.name());
        };
        let mut state = self.lock();
        let next = state.next_id.entry(table).or_insert(1);
        let id = *next;
        *next += 1;
        let mut row = Row::new();
        row.insert(column.to_owned(), serde_json::Value::from(id));
        let key = row_key(table, &row);
        let rows = state.rows.entry(table).or_default();
        if rows.contains_key(&key) {
            anyhow::bail!(
                "{}: generated id {id} is already taken — the counter was \
                 behind the rows this table already holds",
                table.name()
            );
        }
        rows.insert(key, row);
        Ok(id)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MemoryState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[async_trait]
impl SharedStore for MemoryStore {
    async fn schema_version(&self) -> Result<i64> {
        Ok(self.lock().schema_version)
    }

    async fn upsert_rows(&self, table: SharedTable, rows: &[Row]) -> Result<()> {
        let mut state = self.lock();
        for row in rows {
            let mut row = row.clone();
            if let Some(column) = table.id_column() {
                let supplied = row
                    .get(column)
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0);
                if supplied == 0 {
                    // The caller supplied no id, so the store generates one and
                    // the counter moves.
                    let next = state.next_id.entry(table).or_insert(1);
                    row.insert(column.to_owned(), serde_json::Value::from(*next));
                    *next += 1;
                }
                // THE TRAP, and it is the `else` that is not written here: an
                // explicit id is taken as given and the counter is left exactly
                // where it was, which is why a naive seed leaves it at 1 and the
                // next created task collides with the oldest restored one.
            }
            let key = row_key(table, &row);
            state.rows.entry(table).or_default().insert(key, row);
        }
        Ok(())
    }

    async fn advance_id_sequence_past(&self, table: SharedTable, ceiling: i64) -> Result<()> {
        if !table.generates_ids() {
            return Ok(());
        }
        // Burn by generating and discarding, because nothing sets a counter.
        //
        // A do-while, not a while: the real store gives no way to READ the
        // counter, so learning where it stands costs one generated id. This
        // loop is written the way the reducer has to be written, throwaway and
        // all, rather than the way an in-process fake could get away with.
        // Consequence, and it is the real store's too: a second restore of the
        // same snapshot is cheap but not free, and the counter creeps by one.
        //
        // Bounded by construction: every iteration advances the counter by at
        // least one, so the loop ends at or before `ceiling + 1`.
        loop {
            let generated = self.insert_generating_id(table).await?;
            {
                let mut state = self.lock();
                let mut discard = Row::new();
                if let Some(column) = table.id_column() {
                    discard.insert(column.to_owned(), serde_json::Value::from(generated));
                }
                let key = row_key(table, &discard);
                state.rows.entry(table).or_default().remove(&key);
            }
            // Strictly greater, not equal: a counter sitting exactly on the
            // ceiling hands that id out next, which is the same collision one
            // iteration later.
            if generated >= ceiling {
                return Ok(());
            }
        }
    }

    async fn rows(&self, table: SharedTable) -> Result<Vec<Row>> {
        Ok(self
            .lock()
            .rows
            .get(&table)
            .map(|rows| rows.values().cloned().collect())
            .unwrap_or_default())
    }
}
