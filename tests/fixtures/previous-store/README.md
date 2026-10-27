# previous-store

A SpacetimeDB standalone store as the module `main` runs left it, with a
little data in it. The upgrade smoke test
(`src/runtime/tests/startup_smoke.rs`) starts a copy of it and runs the new
binary's startup over it: the same user must see exactly the same tasks and
epics afterwards (`docs/specs/startup.allium`:
`StartupIsSmokeTestedAgainstARealStore`).

- main commit: 3da038a5
- spacetime version: 2.12.0
- module hash: sha256:b04e1aeddf00111f431770d5145d742d48d79125531247b16dc0c5562b2c8441

The module hash is also `data/managed-module.sha256`, the record the managed
store keeps; `scripts/check-previous-store.sh` checks it against `main`'s
`src/spacetime/module.wasm`.

## What is here

- `data/` — the store's data directory, without `cache/`, `logs/`,
  `spacetime.pid` and the extracted `dispatch_module.wasm` (the store rebuilds
  or rewrites them).
- `keys/` — the store's JWT signing key pair. Start the store with
  `--jwt-priv-key-path=keys/id_ecdsa --jwt-pub-key-path=keys/id_ecdsa.pub`, or
  `spacetime start` signs with the operator's own `~/.config/spacetime` key and
  none of the logins below verify.
- `cli.toml` — the CLI login of *user*, the database owner.
- `host.json` — *user*'s board identity: the same identity and credential.
- `expected.json` — the task and epic ids *user* sees, recorded when the data
  was written.

The store also holds an epic and a task created by a second identity,
*other*, which *user* must not see.

**These logins are throwaway.** They were issued by this directory's own key
for a store that only ever runs in tests, and grant nothing anywhere else.

## Moving it to a newer module

After a module change lands on `main`, run `scripts/rotate-previous-store.sh`.
It publishes `main`'s module over a copy of this store (never deleting data),
and replaces `data/` and the fields above. `expected.json` does not change:
the rows must survive the upgrade.

## Rebuilding it from scratch

`cargo test --lib generate_previous_store_fixture -- --ignored` writes a new
one from this binary's embedded module, with new keys and logins. Only do this
on a branch that has not changed the module, so the result is `main`'s.
