# dispatch shared-domain module

The SpacetimeDB module holding dispatch's shared tables, plus the reducers the
Phase 0 escape hatch needs. Module schema version 1.

**This crate is excluded from the dispatch workspace.** It targets
`wasm32-unknown-unknown`, is built by `spacetime build` rather than `cargo`, and
is not linked into the `dispatch` binary, so a root `cargo test` never compiles
it. Two things at the root still watch it from the outside:
`src/spacetime/tests/module_schema.rs` parses this source as text and compares
it to SQLite, and `tests/spacetime_module.rs` publishes it into a throwaway
instance when `spacetime` is on `PATH`. Compiling and unit-testing the crate
itself is `./scripts/check-spacetime-module.sh`, which the pre-push hook and CI
both run.

It is also its own workspace root (`[workspace]` in `Cargo.toml`). Excluding it
from the dispatch workspace stops cargo binding it there but does not stop cargo
walking further up, and inside a `.worktrees/` checkout the next manifest up is
the parent repo's.

Spec: [`docs/specs/spacetime-seed.allium`](../../docs/specs/spacetime-seed.allium).

## What is here, and what is not

The ten shared tables at module version 1, the sequence burn, the seed reducers
and `validate_task_ownership`. Phase 1 added `Task.owner`, the subscriber a
`Subscription` belongs to, and `SchemaVersion.module_version`.

Still absent: the epic-rollup reducer, the ordinary write path, and any rule
about *who* may subscribe to what. Those arrive with Phases 4 and 6.

## Two version numbers, on purpose

`SCHEMA_VERSION` mirrors SQLite's `user_version` and answers "which SQLite
schema did these rows come from?" — the question a restore asks.
`MODULE_SCHEMA_VERSION` answers "which module shape is holding them?". They were
one number in Phase 0 and parted company in Phase 1, when `Task.owner` changed
the module's shape with no SQLite migration behind it.

A publish does not re-run `init`, so `set_schema_version` is what re-stamps the
row afterwards. It takes the SQLite number as an argument and writes the module
number from its own constant — a module cannot be wrong about its own shape, and
a caller can.

## Column order is load-bearing, and so is `#[default(..)]`

SpacetimeDB will automigrate an **appended** column and refuses one inserted
anywhere else. Appending is necessary but not sufficient: an appended column
also needs a `#[default(CONSTANT)]` annotation, or the publish aborts with
*"requires a default value annotation"*. Nothing in the Rust source hints at
this; `tests/spacetime_module.rs` is what catches it.

`src/spacetime/tests/module_schema.rs` holds the other half — it compares every
table's SHARED columns against the live SQLite schema positionally, so a column
added in the middle fails there before anyone reaches a server.

Module-only columns (`SharedTable::module_only_columns`) are checked by presence
instead, not position, and that is forced rather than lax: this module appends,
SQLite appends, and the moment a module-only column is published every *later*
shared column lands after it here while sitting earlier in SQLite. The two
orders cannot both be append-only and identical. `Task.completed_at` is the
first column to sit past `owner` for that reason. What still holds, and what the
test asserts, is that the shared columns appear in SQLite's own order.

## Running it locally

```sh
spacetime start &                       # a local standalone instance
spacetime publish -p . --delete-data=never --yes dispatch-dev
spacetime call dispatch-dev set_schema_version 97
spacetime call dispatch-dev burn_id_sequence '["tasks", 4096]'
spacetime sql dispatch-dev "SELECT * FROM tasks"
```

`--delete-data=never` is worth keeping in the habit: without it, a schema change
the store cannot automigrate is "resolved" by destroying the database, and the
publish reports success either way.

Checking and testing the crate without a server:

```sh
./scripts/check-spacetime-module.sh      # from the repo root
```

## Why `burn_id_sequence` looks the way it does

`#[auto_inc]` assigns a value only when the column is zero, so inserting rows
with explicit ids leaves the counter where it was and the next created task
collides with the oldest restored one. Nothing sets a counter, so the only way
to move it is to generate values and throw them away.

**It must run before the rows are loaded.** Generating ids 1, 2, 3 against a
table that already holds them violates the primary key, and a reducer whose
insert is rejected aborts. See the `BurnIdSequences` rule in the spec.

## Why almost nothing here is `Option`

**SpacetimeDB SQL cannot filter on an optional column.** Not through a syntax
this repo got wrong — there is no syntax. An `Option<T>` is a SATS *sum type*,
and the [SQL reference](https://spacetimedb.com/docs/reference/sql/) says the
language "does not provide a way to construct them, nore does it provide any
scalar operators for them". Verified against a live 2.10.1 instance:

```
SELECT * FROM tasks WHERE owner = 'abc'
  → The literal expression `abc` cannot be parsed as type `(some: String | none: ())`
SELECT * FROM todos WHERE task_id = 7
  → The literal expression `7` cannot be parsed as type `(some: I64 | none: ())`
```

`owner = (some = 'abc')`, `some('abc')`, `IS NOT NULL` and `!= none` are all
rejected too. Non-optional columns filter fine.

A subscription **is** a `WHERE` clause. So an optional column is one no client
can subscribe by — and subscribing is the whole mechanism that keeps a
colleague's private work off somebody else's machine
(`sync.allium: SendsOnlyWhatWasSubscribedTo`).

So absence is a sentinel here: `""` for a string, `0` for an id reference. The
authoritative list, with the reasoning per column, is
`SharedTable::sentinel_columns` in `src/spacetime/snapshot.rs`; the conversion
to and from it lives in `src/spacetime/cli_store.rs`, and nothing above that
boundary sees a sentinel. `sort_order` is the one deliberate exception — zero is
a real sort order and null means "fall back to the id", so a sentinel would
silently reorder cards.

### This was a deliberate breaking change

Changing a column's type is **not** automigratable. The 28 columns were
de-nullified in one go, in Phase 4, because at that point no server existed and
the change was therefore free. Doing the same after Phase 5 seeds a real store
means a dump, a rebuild and a restore.

`the_committed_module_automigrates_into_the_working_tree_module` in
`tests/spacetime_module.rs` reports exactly this, and it did — with
`Changing the type of column plan_path in table epics from
(some: String | none: ()) to String requires a manual migration`. That failure
was the test working. Any future one is a change that needs the same
deliberation.
