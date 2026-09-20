# 4868 — Version the shared schema, not the whole SQLite database

## The bug

A restore compares `Snapshot.schema_version` against the store's. Neither
integer is about the shared tables.

- The snapshot's number is SQLite's `PRAGMA user_version`
  (`src/spacetime/dump.rs::dump_from_sqlite`) — the migration counter for the
  whole database, local tables included.
- The store's number is `SCHEMA_VERSION`, hand-written in
  `spacetime/module/src/lib.rs` and linked to nothing.

So it is too strict (a local-only migration invalidates every earlier backup)
and too loose (the constant goes stale unnoticed) at the same time.

## The decision

Drop the integer. Compare the shared tables' **column sets**, per table,
order-insensitive. Both sides already produce them, so the answer is derived
rather than remembered. Agreed with the user on 2026-09-19; see
`docs/specs/spacetime-seed.allium`, `RefuseMismatchedSchema`.

Four parts:

1. Column sets replace the integer.
2. `snapshot_format_version` goes to 2. A format-1 snapshot never recorded its
   columns, so it cannot be checked and is refused as `format_unsupported`.
3. Set comparison, not ordered — a restore writes rows by column name. The
   positional check in `src/spacetime/tests/module_schema.rs` stays as it is;
   it exists for SpacetimeDB's append-only rule, a different concern.
4. The comparison runs **before the first write**. Today the equivalent check
   lives in `cli_store.rs::encode_row_for_reducer`, per row, after the id burn
   and after earlier tables are already written — so it aborts a restore
   halfway. It becomes a backstop, not the gate.

### Not done here: removing the module's `SchemaVersion` table

Tempting, since nothing will read `SCHEMA_VERSION` afterwards. Rejected:
dropping a table is not automigratable, and
`tests/spacetime_module.rs::the_committed_module_automigrates_into_the_working_tree_module`
both requires the automigration to succeed and uses that very row as its probe
for "migrated rather than rebuilt". Phase 1 (#4859) owns the module's schema and
can cut it with a fresh publish. The constant's doc comment is corrected here to
stop claiming it gates a restore.

## Steps

Spec first, then a failing test, then code — for each step.

### 1. Spec (done first, separately)

`docs/specs/spacetime-seed.allium`: `TableExtract` gains `columns`; `Snapshot`
loses `schema_version`; `snapshot_format_version` = 2; `TakeSnapshot`,
`RestoreSnapshot`, `RefuseMismatchedSchema` and `RefuseIncompleteSnapshot`
rewritten. `allium check` green.

### 2. `TableExtract` carries its columns

Test first: an extract of an empty table still names its columns.

- `src/spacetime/snapshot.rs`: add `columns: Vec<String>`; `new` and `empty`
  take them. Reject an extract with no columns — "no claim" must not read as
  "matches".

### 3. Both dump paths record columns

Test first: a SQLite dump and a store dump of the same board agree on columns,
and an empty table still has them.

- `dump.rs::read_sqlite_table` already has `stmt.column_names()`.
- `dump.rs::read_host_identity` uses `SharedTable::assembled_columns()`,
  including on the no-host early return.
- `store.rs::dump` asks the store.

### 4. The store reports its columns

Test first: `cli_store` returns the server's columns for a table.

- `store.rs`: `SharedStore::schema_version()` → `columns(table)`.
- `cli_store.rs`: implement from the existing memoised `column_shapes`.
- `MemoryStore`: `with_schema_version` → settable columns.

### 5. Restore compares column sets up front

Test first, and these are the two that matter:

- a snapshot whose `tasks` extract lacks a column the store has is refused
  `schema_mismatch` with **nothing written and no id burned**;
- a snapshot whose columns match but are in a different order restores.

Plus: a format-1 snapshot is refused `format_unsupported`.

- `restore.rs`: replace the version check with the per-extract set comparison,
  keeping the existing precedence (format → schema → completeness).

### 6. Drop the integer everywhere else

- `snapshot.rs`: remove `Snapshot::schema_version`; bump
  `SNAPSHOT_FORMAT_VERSION` to 2.
- `main.rs`: `store.dump(schema_version)` → `store.dump()`.
- `spacetime/module/src/lib.rs`: correct `SCHEMA_VERSION`'s doc comment — it no
  longer gates a restore. Table and reducers stay (see above).
- `docs/reference.md`: the SpacetimeDB section, and the `set_schema_version`
  line.

### 7. Converge

`allium weed` on the spec, then the repo's verify command.

## What this does not close

`SCHEMA_VERSION` is still hand-written and still unlinked to the migration
chain. After this change nothing reads it, so it can no longer refuse a good
backup — but it is dead weight until Phase 1 removes it.
