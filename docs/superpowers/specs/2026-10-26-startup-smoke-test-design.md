# Startup smoke test — design

Task #98538, epic #314. Status: design, awaiting approval.

## Why

`dispatch tui` failed to start for the operator after a `spacetime login` in
another session replaced the CLI token. Startup publishes the embedded module
whenever it differs from the store's, publishing needs the database owner, and
the new login was not the owner. The token change stayed invisible until a code
change made startup publish again. Recovering meant creating a new database
and moving all data by hand.

An earlier, different incident: a module change added user identities and
owner columns, and the operator's existing rows had no owner, so after
upgrading the board started but showed none of their work.

Both share one property: **nothing runs the whole startup path, from a store
the previous version left behind to a board that draws, before the change
reaches the operator.** Each step has its own tests (`src/runtime/tests/`,
`src/spacetime/tests/managed_store*.rs`, `tests/spacetime_module.rs`), but
`TuiRuntime::bootstrap` is only ever run over the in-memory store stand-in, and
the publish tests never look at what a board sees afterwards. A broken startup
is expensive because dispatch is the tool the operator would fix it with.

## Goals

1. A change that makes startup abort fails `cargo test --no-fail-fast` (and so
   CI) before it is merged.
2. A change that makes a user's existing tasks or epics disappear after an
   upgrade fails the same way. **The bar, agreed with the operator: the same
   user sees exactly the same tasks and epics before and after the upgrade.**
3. The failure names the startup step and carries the startup abort message
   (`StartupAbort::message`), so the cause is readable from the CI log.

## Non-goals

- **A changed `spacetime login` token.** Out of scope here; filed as a separate
  task (see "Follow-up"). The smoke test runs with its own CLI config and its
  own login, so it cannot see the operator's.
- **The pre-push hook.** The hook leaves `cargo test` out on purpose; a live
  store there would make every push pay for one. CI is the gate.
- **tmux, the terminal and the render loop.** The test stops where
  `TuiRuntime::bootstrap` returns. Window wiring after it has its own real-tmux
  tests (`tests/tmux_board_restart.rs`).
- **The embedding model's real download path** (learning #511). Every test
  gets the no-op embedding stub; this design does not change that.
- **Pinning the content of startup messages**, or every table. The data check
  covers tasks and epics, per the agreed bar.

## Anti-goals

- **Never touch the operator's machine state.** No `127.0.0.1:3000`, no
  `~/.local/share/dispatch`, no `~/.claude`, no `~/.config/spacetime` (which
  includes the signing key `spacetime start` falls back to; see "Where the
  previous store comes from"). Test 1's fresh store is started with key paths
  in its temp directory too. A smoke
  test that adopts the real store would stop it on exit (CLAUDE.md, "A bare
  `cargo run -- tui` runs the managed store").
- **No green run that did not run.** Without `spacetime` on `PATH` the tests
  print `skipping: spacetime not available on PATH` and pass — except under CI,
  where they fail (`spacetime_available_or_skip`, the rule
  `tests/spacetime_module.rs` already follows).
- **No fixture data written by the new code.** Data written by the code under
  test only proves the new code understands itself. The upgrade fixture must
  be data the *previous* module wrote.
- No wall-clock sleeps (`scripts/check-no-test-sleep.sh`).

## The design

Two tests, both in the normal suite, both against a throwaway SpacetimeDB
instance on a free port with its own data directory and its own CLI config
(`tests/common/spacetime_instance.rs::Instance` already provides this, with
the TCP-before-HTTP wait and the one transport retry, learning #806).

### Test 1 — fresh install

1. Start a throwaway instance. Empty data directory for the board, no
   `host.json`.
2. Run the real composition root, `TuiRuntime::bootstrap`, with:
   - `target: StoreTarget::Managed` over a `ManagedStore` whose
     `ManagedStoreLayout` names the throwaway address, data directory and log
     path, and whose publisher and store use the instance's CLI config
     (`with_config_path`). This is the managed path, the one that publishes —
     the path that failed.
   - `build_store: StoreParts::build` — the real store wiring, not the
     in-memory stand-in.
   - `paths`: `StartupPaths` under a temp dir, so the configuration check
     never reaches `$HOME`.
   - `port: 0`.
3. Assert bootstrap returns `Ok`: the module was published, the first
   connection was accepted, the identity was minted and written to
   `host.json`, the initial subscription was applied.
4. Create one task through the service layer, drop the runtime, run bootstrap
   a second time over the same data directory and store. Assert it returns
   `Ok` without publishing (the hash matches) and the board's reads contain
   the task. This covers `ConfirmUnchangedUserIdentity` with a real credential.

### Test 2 — upgrade from the previous module

1. Copy the committed **previous store** (see "Where the previous store comes
   from") into a temp directory and start a throwaway instance on the copy. It
   already runs the module `main` runs, and already holds data that module
   wrote. No code under test has touched it.
2. Write a `host.json` for the board from the fixture's committed *user*
   identity and its credential, so the board connects as the same person who
   wrote the data. *user* is also the database owner: the managed store's CLI
   config holds the same login, so one credential both publishes and connects.
3. Read the "before" set from the fixture's committed `expected.json`: the ids
   of the tasks and epics *user* created, plus the tasks in epics *user*
   created (`sync.allium`'s `own_creations` and the epics it covers). The
   generator wrote it when it made the fixture, under the previous module.
   The test never queries the previous module's tables: a branch that renames
   a column would otherwise break the test at the "before" read, not at the
   upgrade (and SQL returns unordered positional arrays, learning #806).
   *Decided (design author's call):* followed epics are left out
   of the first version — their rows arrive one round trip after bootstrap
   returns ("not waited for"), so an exact check on them would be racy. The
   fixture therefore holds no follow (Subscription) rows. Adding them, with a
   wait for the widened subscription, is a later step.
4. Run the real `TuiRuntime::bootstrap` as in test 1. When the branch changed
   the module, the store's recorded hash differs from the embedded module, so
   it publishes the new module over the old (`--delete-data=never` is already
   the publisher's behaviour, `PublishingNeverClearsData`). When the branch did
   not change the module, the hashes match and startup only connects: the test
   then proves "an existing store with data still starts and shows the same
   rows", not "the module migrates". A green run on a branch that did not
   touch the module says nothing about automigration.
5. Assert bootstrap returns `Ok`, and that the board's reads
   (`BoardReads`, the seam the board draws from) contain **exactly** the
   "before" task ids and epic ids. Missing ids fail with the list of missing
   ids; extra ids fail too.

On failure, each test reports which step failed and the abort message, so a
CI log reads like "upgrade smoke: bootstrap aborted: <message>".

### Where the previous store comes from

**Decided with the operator: commit a real store data directory**, not a
snapshot file. A snapshot can only be loaded by the current binary's restore,
which checks the snapshot format version, requires an extract for every table
in the current `SharedTable::ALL`, and calls the current seed reducers — so a
branch that adds a table, renames a seed reducer or bumps the snapshot format
would fail while loading the fixture, before the upgrade it is meant to test.
A store directory needs no dispatch code to load: `spacetime start` on a copy
of it brings back the previous module and its data as they were.

Committed under `tests/fixtures/previous-store/` (the builder may choose the
exact path):

- the SpacetimeDB standalone data directory, with the `dispatch` database
  published from `main`'s `module.wasm` and the fixture data in it, and the
  managed-store module-hash record for that module;
- the server's JWT signing key pair (`id_ecdsa`, `id_ecdsa.pub`). By default
  `spacetime start` signs with `~/.config/spacetime/id_ecdsa` — the
  operator's own key, which is what `tests/common/spacetime_instance.rs`'s
  `Instance::start` uses today. The committed logins are only valid under the
  key that signed them, so the fixture carries its key and the test starts the
  store with `--jwt-priv-key-path` and `--jwt-pub-key-path` pointing at it.
  Given paths that do not exist, `spacetime start` creates the pair there,
  which is how the generator makes it;
- `expected.json`: the task and epic id sets *user* sees, written by the
  generator at the same time as the data;
- the CLI config holding *user*'s login (the database owner), and the
  *other* identity with its credential. These are throwaway logins
  minted by the throwaway server whose keys are in the same directory; they
  grant nothing outside it. Say so in a README beside them;
- a README saying what the directory is, which `main` commit and which
  `spacetime` version made it, and how to rotate it.

**Verified by a spike (2026-10-26, spacetime 2.12.0).** A store started with
explicit key paths, published to, stopped, copied, and started again on the
copy (other port, same keys) accepts a re-publish from the original login,
and refuses one from a different login with `403 Forbidden: <identity> is not
authorized to perform action on database <db>: update database` — the
operator's incident, reproduced.

**Size.** With the module published and no data, the directory is 5.5 MB
(about 1.6 MB of it the module bytes, 4 MB `replicas/`), and well under 2 MB
gzipped. Leave out `cache/` (the store rebuilds it, 2.6 MB), `logs/` and
`spacetime.pid`. Each rotation adds a new copy to git history, so rotate once
per module change on `main`, not more. If the directory with data is over
about 10 MB, stop and ask before committing; the agreed fallback is a
hand-maintained snapshot file.

**How the test starts it.** The test starts the store itself, with the key
flags, on a free port; `bootstrap`'s managed store then *adopts* it (the
address answers), so the production spawner needs no key flags.

**Rotation (as built).** The previous store must match what `main` runs.
`scripts/rotate-previous-store.sh` rebuilds it from scratch with the fixture
generator, on a checkout whose `module.wasm` is local `main`'s (it refuses
otherwise), so a module rebuilt twice on a branch never becomes "previous".
*Changed during the build, agreed with the operator:* the first version
published `main`'s module over the committed store, which made the rotation
itself an upgrade, but every publish appends the module to the store's commit
log, so the directory grew about 1.6 MB per rotation (14 MB after one). A
fresh rebuild stays at about 10 MB on disk and 1 MB compressed. The upgrade
is still tested, by the upgrade smoke test, on every branch that changes the
module. A lagging fixture still tests a real upgrade, only from further back.

**Gate (as built).** `scripts/check-previous-store.sh`, with a self-test, in
the pre-push hook and CI. It fails when the fixture's recorded module hash
matches no `module.wasm` `main` ever committed, and passes with a note when
it matches an older one, so a module merge does not block every branch's push
until someone rotates. It skips, saying so, without a local `main` or in a
shallow checkout (CI's, including a push to `main`).

**`spacetime` version.** The directory format belongs to the `spacetime`
version that wrote it; CI pins that version. Bumping the pin starts the
committed directory under the new version, which is a useful check in itself.
If the new version cannot read it, rotate the fixture as part of the bump.

**Fixture content.** Small and hand-chosen, but written through the previous
module, not typed in: at least one epic with subtasks, tasks in each status
column, a task and an epic created by *other* (which must stay out of
*user*'s set before and after), and the host and settings rows a board
writes on its first connection. The first fixture is made with `main`'s
current module, by a one-off generator (an `#[ignore]` test or a script; the
builder chooses) that is committed so the next person can rebuild it.

## Constraints from the code

- `BootstrapInputs` and `TuiRuntime::bootstrap` are private to
  `src/runtime/mod.rs`. The tests either live in-crate (`src/runtime/tests/`)
  with the instance helper made reachable from there, or a narrow test-facing
  entry point is added. The builder chooses; either way production gains no
  new behaviour.
- `Instance::start` (`tests/common/spacetime_instance.rs`) passes no key
  paths, so every live-store test today signs with the operator's
  `~/.config/spacetime/id_ecdsa`, and creates it if it is missing. Fix it in
  passing: key paths under the instance's temp directory.
- `run_tui` checks `inside_tmux_session()` before bootstrap; the test calls
  bootstrap directly and so needs no tmux.
- The managed store's address is fixed in production
  (`ManagedStoreLayout::for_launch`), and the layout type already takes any
  address, data directory and config path. **But `ManagedStore::bring_up`
  returns `server: http://{MANAGED_STORE_ADDRESS}` whatever the layout says**,
  and `resolve_store_server` connects there. Pointed at a throwaway layout, the
  test would publish to the throwaway instance and then connect to the
  operator's real store on 3000. *Decided (design author's call):* fix it in
  production — `ManagedStoreReady.server` comes from the layout's address, and
  so does `ManagedStorePortTaken`'s address. In production the layout's address
  is the constant, so behaviour does not change; `startup.allium` says the
  board connects to the address the managed store was brought up on. This is a
  latent bug (two sources of one fact), and the smoke test's first anti-goal
  depends on it.
- Both tests are slow (a store start plus a publish). They belong with the
  other live-store targets and follow `docs/testing.md`'s timing notes.

## Spec

Apart from the managed-store address fix above, this adds tests, not
behaviour. The guarantee goes into
`docs/specs/startup.allium` as an `@invariant` beside the publish rules:
startup from a fresh install and from the previous module's store is exercised
end to end against a real store, and the same identity sees the same tasks and
epics afterwards. `docs/testing.md` lists the new target and the rotation step.

## Follow-up (separate task)

**Check the CLI login owns the managed database before publishing.** When the
`spacetime` CLI's identity is not the database owner, startup should abort
with a message that names both identities and the fix (log back in as the
owner, or restore the old CLI config), instead of a raw publish error. This is
a behaviour change to `startup.allium` and gets its own task. The committed
previous store makes it testable: start a copy with a CLI config holding a
login that is *not* the owner, and assert the abort.

## Left to the builder

- In-crate tests versus a test-facing entry point.
- How the test reads the "before" set (SQL over the instance, or a connection
  with the previous module's view).
- How the CI check gets `main`'s stamp.
- The exact fixture rows, within the content list above.
