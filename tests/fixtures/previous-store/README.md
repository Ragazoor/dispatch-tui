# previous-store

A SpacetimeDB standalone store as the module `main` runs left it, with a
little data in it. The upgrade smoke test
(`src/runtime/tests/startup_smoke.rs`) starts a copy of it and runs the new
binary's startup over it: the same user must see exactly the same tasks and
epics afterwards (`docs/specs/startup.allium`:
`StartupIsSmokeTestedAgainstARealStore`).

- main commit: bfed8999
- spacetime version: 2.12.0
- module hash: sha256:f3718fb732fc2399d82b9bebee5bb4291e6d71527f2b60dd46921960df0c837d

The module hash is also `data/managed-module.sha256`, the record the managed
store keeps; `scripts/check-previous-store.sh` checks it against `main`'s
`src/spacetime/module.wasm`.

## What is here

- `data/` — the store's data directory, without `cache/`, `logs/`,
  `module_logs/`, `spacetime.pid`, `db.lock` and the extracted
  `dispatch_module.wasm` (the store rebuilds or rewrites them).
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

After a module change lands on `main`, run `scripts/rotate-previous-store.sh`
on `main`, or on a branch that merged `main` without changing the module. It
rebuilds this directory from scratch from `main`'s module (fresh keys, logins
and rows; the generator is `generate_previous_store_fixture` in
`src/runtime/tests/startup_smoke.rs`) and fills in the fields above. It refuses
when the checkout's module is not `main`'s.

It does not publish over the old store: every publish appends the module to
the store's commit log, so the directory would grow by about 1.6 MB each time.
