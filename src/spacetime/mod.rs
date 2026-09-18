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
//! # The generated bindings
//!
//! [`bindings`] is the SDK's view of `spacetime/module/`, produced by
//! `scripts/regenerate-spacetime-bindings.sh` and committed rather than built.
//! Committing them keeps `cargo build` working with nothing but cargo —
//! generating them needs the `spacetime` CLI, which CI does not install. The
//! cost is a step to remember after a module change, which
//! `tests::bindings_parity` is what catches.
//!
//! The one non-obvious obligation is the id-sequence burn. See
//! [`store::SharedStore::advance_id_sequence_past`] and, for why it cannot be
//! skipped, `tests::sequence_burn`.

#[rustfmt::skip]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod bindings;
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
    Refusal, RefusalReason, Row, Sentinel, SharedTable, Snapshot, TableExtract, SHARED_TABLE_COUNT,
    SNAPSHOT_FORMAT_VERSION,
};
pub use store::{MemoryStore, SharedStore};
