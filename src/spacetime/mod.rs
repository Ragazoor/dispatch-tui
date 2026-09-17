//! The snapshot escape hatch: dump, restore and seed for the shared domain.
//!
//! Spec: [`docs/specs/spacetime-seed.allium`](../../docs/specs/spacetime-seed.allium).
//!
//! One artefact does three jobs, because the three want the same guarantees —
//! everything, unaltered, and restorable onto a store that then behaves as if
//! the rows had always been there:
//!
//! - **The backup.** The shared store has no replication. A snapshot is the
//!   only copy.
//! - **The recovery path.** SpacetimeDB's migrations are append-only, so
//!   removing a column, reordering one or adding a unique constraint is refused
//!   outright. The way out is dump, rebuild, restore. Nothing else offers one.
//! - **The seed.** The shared store starts from one existing board with its
//!   task and epic ids carried over unchanged, so every id already written into
//!   a worktree name, a branch name or a person's memory still means what it
//!   meant.
//!
//! The one non-obvious obligation is the id-sequence burn. See
//! [`store::SharedStore::advance_id_sequence_past`] and, for why it cannot be
//! skipped, `tests::sequence_burn`.

mod cli_store;
mod dump;
mod restore;
mod snapshot;
mod store;

#[cfg(test)]
mod tests;

pub use cli_store::SpacetimeCliStore;
pub use dump::dump_from_sqlite;
pub use restore::{restore, RestoreError};
pub use snapshot::{
    Refusal, RefusalReason, Row, SharedTable, Snapshot, TableExtract, SHARED_TABLE_COUNT,
    SNAPSHOT_FORMAT_VERSION,
};
pub use store::{MemoryStore, SharedStore};
