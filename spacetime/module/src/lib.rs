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

mod agent_state;
mod blanks;
mod config;
mod feed;
mod feed_ingest;
mod grouping;
mod learning;
mod packages;
mod rules;
mod seed;
mod sequence;
mod support;
mod tables;
mod tasks_epics;

pub use agent_state::*;
pub use blanks::*;
pub use config::*;
pub use feed::*;
pub use feed_ingest::*;
pub use grouping::*;
pub use learning::*;
pub use packages::*;
pub use rules::*;
pub use seed::*;
pub use sequence::*;
pub use support::*;
pub use tables::*;
pub use tasks_epics::*;

#[cfg(test)]
mod tests;

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

/// The data rules this module enforces, published for a board to read.
///
/// **A table of its own, not a column of `schema_version`.** A board reads it
/// before anything else, and it must be readable from a module of ANY age. A
/// column appended to `schema_version` would make an older module's row fail to
/// decode on the board (a panic on the SDK's thread, then a connection that
/// never answers). A table an older module does not have is refused by the
/// store as "no such table", which the board turns into a named reason: that
/// module predates the version gate.
///
/// One row, `id` 1. Not a shared table and absent from the snapshot: like
/// `schema_version` it describes the store rather than the domain.
#[spacetimedb::table(accessor = rules_version, public)]
#[derive(Clone, Debug)]
pub struct RulesVersion {
    #[primary_key]
    pub id: i64,
    /// [`RULES_VERSION`], stamped by the module from its own constant — never
    /// taken from a caller. A board compares it for equality with the number
    /// it was built with (`sync.allium: AStoreWithOtherRulesFailsTheAttempt`).
    pub version: i64,
}

/// Keep the published rules version equal to this module's own, whenever
/// anyone connects.
///
/// **Why on connect rather than in `init` or `set_schema_version`.** `init`
/// runs only on a brand-new database, and nothing in production calls
/// `set_schema_version` after a publish — the number would keep describing the
/// module the database was CREATED with, and a board would refuse the very
/// module it just published. A connection is the one event that always follows
/// a publish and always precedes a board's read, and the lifecycle reducer runs
/// in the module's current code, so the stamp cannot go stale. It writes only
/// when the number differs, so a steady-state connection costs one read.
#[spacetimedb::reducer(client_connected)]
pub fn stamp_rules_version_on_connect(ctx: &ReducerContext) {
    let row = RulesVersion {
        id: 1,
        version: RULES_VERSION,
    };
    match ctx.db.rules_version().id().find(1) {
        Some(existing) if existing.version == RULES_VERSION => {}
        Some(_) => {
            ctx.db.rules_version().id().update(row);
        }
        None => {
            ctx.db.rules_version().insert(row);
        }
    }
}
