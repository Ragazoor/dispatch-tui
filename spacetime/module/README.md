# dispatch shared-domain module

The SpacetimeDB module holding dispatch's shared tables, plus the reducers the
Phase 0 escape hatch needs.

**This crate is excluded from the dispatch workspace.** It targets
`wasm32-unknown-unknown`, is built by `spacetime build` rather than `cargo`, and
is not linked into the `dispatch` binary. A plain `cargo test` at the repo root
never touches it.

Spec: [`docs/specs/spacetime-seed.allium`](../../docs/specs/spacetime-seed.allium).

## What is here, and what is not

Phase 0 needs somewhere to restore *into*, so this crate carries the shared
tables at the shape Phase 1 will formalise, and no more. It exists to make the
escape hatch real and checkable, not to be the finished schema — in particular
it has no `Task.owner`, no subscription model and no epic-rollup reducer. Phase
1 extends this crate; it does not replace it.

## Running it locally

```sh
spacetime start &                       # a local standalone instance
spacetime publish --project-path . dispatch-dev
spacetime call dispatch-dev burn_id_sequence '["tasks", 4096]'
spacetime sql dispatch-dev "SELECT * FROM tasks"
```

## Why `burn_id_sequence` looks the way it does

`#[auto_inc]` assigns a value only when the column is zero, so inserting rows
with explicit ids leaves the counter where it was and the next created task
collides with the oldest restored one. Nothing sets a counter, so the only way
to move it is to generate values and throw them away.

**It must run before the rows are loaded.** Generating ids 1, 2, 3 against a
table that already holds them violates the primary key, and a reducer whose
insert is rejected aborts. See the `BurnIdSequences` rule in the spec.
