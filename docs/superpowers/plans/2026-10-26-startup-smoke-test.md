# Startup Smoke Test Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `cargo test` fails when a change makes `dispatch tui` startup abort, or makes a user's existing tasks and epics disappear after an upgrade.

**Architecture:** Two in-crate tests run the real composition root, `TuiRuntime::bootstrap`, against a throwaway `spacetime start` on a free port: one from a fresh install, one from a committed store directory the previous module wrote. A one-off generator makes that directory; a script rotates it to `main`'s module.

**Tech Stack:** Rust, tokio, SpacetimeDB 2.12 standalone (`spacetime` CLI), bash.

**Spec:** `docs/superpowers/specs/2026-10-26-startup-smoke-test-design.md`

## Global Constraints

- Never touch `127.0.0.1:3000`, `~/.local/share/dispatch`, `~/.claude`, `~/.claude.json` or `~/.config/spacetime`. Every `spacetime start` passes `--jwt-priv-key-path`/`--jwt-pub-key-path`; every CLI call passes `--config-path`.
- Without `spacetime` on `PATH`: print `skipping: spacetime not available on PATH` and pass; under CI (`CI` set): fail. Same rule as `tests/common/spacetime_instance.rs::instance_tests_should_run`.
- No wall-clock sleeps; deadline-bounded polls carry `// allow-test-sleep: <why>` (`scripts/check-no-test-sleep.sh`).
- Upgrade bar, verbatim: **the same user sees exactly the same tasks and epics before and after the upgrade.**
- Fixture directory: leave out `cache/`, `logs/`, `spacetime.pid`. Over about 10 MB: stop and ask the operator.
- Spec first, then failing test, then code (CLAUDE.md, "Working With the User").
- Commit subjects are Conventional Commits.

## Review Focus

1. **A smoke test that silently connects to the operator's store.** Task 2's test pins that `bring_up` returns the layout's address; Task 5 asserts the bootstrapped `store_server` equals the throwaway URL.
2. **A fixture whose committed login no longer verifies** (keys missing or regenerated). Task 7 asserts bootstrap does not abort with `UserIdentityConflict` and that the fixture's key files exist before starting.
3. **Followed or other-user rows leaking into the "same tasks" set.** Task 6 creates rows as *other*; Task 7 asserts exact equality, so a leak fails.
4. **A green upgrade run that published nothing.** Task 7 logs whether the recorded hash differed (publish happened) so a reader of the CI log can tell; the design says a green run on a module-unchanged branch proves no migration.
5. **A store process left running after a failed assertion.** Task 4's helper kills the child in `Drop`; Task 4's test asserts the port is free after drop.

---

### Task 1: Spec — the managed store's address, and the smoke-test guarantee

**Files:**
- Modify: `docs/specs/startup.allium`

- [ ] **Step 1: Tend the spec** (`allium:tend`). Two changes:
  - In the managed-store bring-up rules (`ConnectWhenTheManagedModuleIsCurrent` and the publish rules, and `ManagedStorePortTaken`'s message), say the board connects to, and the port-taken abort names, **the address the managed store was brought up on** (its layout), never a second copy of the constant. Production's layout address is `127.0.0.1:3000`, so behaviour is unchanged.
  - Add an `@invariant StartupIsSmokeTestedAgainstARealStore` beside `PublishingNeverClearsData`: startup from a fresh install and from the previous module's committed store runs end to end against a real store in the suite, and the same identity sees exactly the same tasks and epics afterwards. Cite `src/runtime/tests/startup_smoke.rs` by path.
- [ ] **Step 2:** `allium check docs/specs/startup.allium` → no errors. Run `./scripts/check-doc-symbols.sh` → exit 0 (the cited test file does not exist yet; if the checker flags it, annotate `allow-phantom-symbol: created in Task 5`).
- [ ] **Step 3: Commit** `docs(specs): managed store connects to its own layout address`.

### Task 2: The managed store reports its layout's address

**Files:**
- Modify: `src/spacetime/managed_store.rs` (`ManagedStorePorts`, `address_action`, `ManagedStore::bring_up`, `SpacetimeManagedStore`)
- Modify: every `ManagedStorePorts` impl in `src/spacetime/tests/managed_store.rs` and `src/spacetime/tests/managed_store_real.rs`
- Test: `src/spacetime/tests/managed_store.rs`

**Interfaces:**
- Produces: `ManagedStorePorts::managed_address(&self) -> String` (`host:port`); `address_action(state: ManagedAddressState, address: &str) -> AddressAction`. `bring_up` returns `ManagedStoreReady { server: format!("http://{}", ports.managed_address()), .. }`.

- [ ] **Step 1: Write the failing tests** in `src/spacetime/tests/managed_store.rs`, using the file's existing mock ports with `managed_address` returning `"127.0.0.1:4123"`:
  - `bring_up_returns_the_layout_address_not_the_default`: `assert_eq!(store.bring_up().unwrap().server, "http://127.0.0.1:4123")`.
  - `a_taken_port_names_the_layout_address`: `address_action(ManagedAddressState::HeldBySomethingElse, "127.0.0.1:4123")` is `Abort(ManagedStorePortTaken { address: "127.0.0.1:4123".into() })`.
- [ ] **Step 2:** `cargo test --lib managed_store` → FAIL (does not compile: no `managed_address`).
- [ ] **Step 3: Implement.** Add the trait method; `SpacetimeManagedStore` returns `self.layout.address.clone()`; mocks return `MANAGED_STORE_ADDRESS` unless a test sets another. `bring_up` reads it once.
- [ ] **Step 4:** `cargo test --lib managed_store` → PASS. `cargo clippy --all-targets -- -D warnings` → clean.
- [ ] **Step 5: Commit** `fix(startup): connect to the managed store's own address`.

### Task 3: Live-store tests stop signing with the operator's key

**Files:**
- Modify: `tests/common/spacetime_instance.rs::Instance::start`

- [ ] **Step 1:** Add `--jwt-priv-key-path=<dir>/keys/id_ecdsa` and `--jwt-pub-key-path=<dir>/keys/id_ecdsa.pub` (under the instance's temp dir; `spacetime start` creates them) to the `start` args. Update the struct's doc comment: isolated through `--data-dir`, `--config-path` and the key paths.
- [ ] **Step 2:** `cargo test --no-fail-fast --test spacetime_module > /tmp/claude-…/scratchpad/t.txt 2>&1; echo $?` (scratchpad path) → `0`, and the file shows tests ran, not `skipping`.
- [ ] **Step 3: Commit** `test(spacetime): sign test instances with their own key`.

### Task 4: An in-crate throwaway store

The smoke tests live in-crate (`TuiRuntime::bootstrap` and `BootstrapInputs` are private), so they cannot reach `tests/common`.

**Files:**
- Create: `src/runtime/tests/live_store.rs` (declared `mod live_store;` in `src/runtime/tests/mod.rs`, `#[cfg(test)]` via its parent)

**Interfaces:**
- Produces:
  - `pub(super) fn live_store_tests_should_run() -> bool` — `spacetime` on `PATH` → true; absent and `CI` set → panic `"spacetime is required under CI"`; absent otherwise → `eprintln!("skipping: spacetime not available on PATH")`, false.
  - `pub(super) struct LiveStore { child: Child, port: u16 }` with `pub(super) fn start(data_dir: &Path, keys_dir: &Path, cli_config: &Path) -> LiveStore` (free port; waits until `GET /v1/identity` gets any HTTP status, deadline 30 s; panics if the child exited), `pub(super) fn address(&self) -> String` (`127.0.0.1:<port>`), `pub(super) fn url(&self) -> String`, and `Drop` that kills and waits the child.
  - `pub(super) fn spacetime(cli_config: &Path, args: &[&str]) -> std::process::Output` — runs the CLI with `--config-path` first.

- [ ] **Step 1: Write the failing test** `a_live_store_serves_and_frees_its_port_on_drop`: start in a tempdir; assert `TcpStream::connect(address)` succeeds; drop; assert a `TcpListener::bind(address)` succeeds within a deadline-bounded poll.
- [ ] **Step 2:** `cargo test --lib live_store` → FAIL (missing module).
- [ ] **Step 3: Implement** as specified. Copy the HTTP readiness check's approach from `tests/common/spacetime_instance.rs::Instance` (hand-written GET, any `HTTP/` reply counts).
- [ ] **Step 4:** `cargo test --lib live_store` → PASS.
- [ ] **Step 5: Commit** `test(runtime): in-crate throwaway spacetime store`.

### Task 5: Smoke test 1 — fresh install

**Files:**
- Create: `src/runtime/tests/startup_smoke.rs` (declared in `src/runtime/tests/mod.rs`)

**Interfaces:**
- Consumes: Task 2's `managed_address`; Task 4's `LiveStore`, `spacetime`, `live_store_tests_should_run`; `bootstrap_inputs` and `fixture()` from `src/runtime/tests/mod.rs` (temp `StartupPaths`).
- Produces: `pub(super) fn managed_target(layout_data_dir: &Path, log_dir: &Path, address: &str, cli_config: &Path) -> StoreTarget` — a `StoreTarget::Managed` over `ManagedStore::new(SpacetimeManagedStore::new(RealProcessRunner, SpacetimeStartSpawner, ManagedStoreLayout { address, data_dir, log_path }, EMBEDDED_MODULE).with_config_path(cli_config), FileModuleHashRecord::in_data_dir(layout_data_dir), embedded_module_hash())`. `pub(super) async fn boot(data_dir, paths, target) -> Result<Bootstrap>` — `TuiRuntime::bootstrap(BootstrapInputs { target, build_store: StoreParts::build, ..bootstrap_inputs(..) })`.

- [ ] **Step 1: Write the test** `a_fresh_install_starts_against_a_real_store`:
  - return early unless `live_store_tests_should_run()`;
  - `LiveStore::start` in a tempdir; `boot` with `managed_target` pointing at that store's address and data dir;
  - `let b = boot(..).await.unwrap_or_else(|e| panic!("fresh-install smoke: bootstrap aborted: {e}"))`;
  - `assert_eq!(b.store_server, store.url())`;
  - create one task through the runtime's task service (the mutation boundary, `TaskServiceApi`), then wait for it in `b.runtime.board_reads.list_all()` (deadline-bounded poll on `revision()`);
  - drop `b`; `boot` again over the same data dir and target; assert `Ok`, and that `list_all()` contains the task's id;
  - assert the module hash file in the store data dir equals `embedded_module_hash()`.
- [ ] **Step 2:** `cargo test --lib startup_smoke > <scratchpad>/t.txt 2>&1; echo $?`. Expected before Task 2 lands: the `store_server` assertion fails. After: PASS. (If Task 2 is already in, temporarily point `managed_address` back to the constant to see it fail, then restore.)
- [ ] **Step 3:** Fix whatever the real wiring reveals (spec first if it is behaviour). Re-run → PASS.
- [ ] **Step 4: Commit** `test(startup): fresh-install smoke test against a real store`.

### Task 6: The previous-store fixture and its generator

**Files:**
- Create: `src/runtime/tests/startup_smoke.rs::generate_previous_store_fixture` (`#[ignore]`)
- Create: `tests/fixtures/previous-store/` — `data/`, `keys/id_ecdsa`, `keys/id_ecdsa.pub`, `cli.toml` (*user*, the owner), `host.json` (*user*'s board), `expected.json`, `README.md`

**Interfaces:**
- Produces: `expected.json` = `{"tasks": [<TaskId>…], "epics": [<EpicId>…]}`, ids ascending. README fields: `main commit`, `spacetime version`, `module hash` (same string as `data/<MODULE_HASH_FILE>`).

- [ ] **Step 1: Write the generator** (run by hand: `cargo test --lib generate_previous_store_fixture -- --ignored`). Into a tempdir:
  1. `LiveStore::start` with a fresh `keys/` and `cli.toml`.
  2. Publish `EMBEDDED_MODULE` once by CLI: write it to a temp `.wasm`, `spacetime(cli.toml, ["publish", "-b", <wasm>, "-s", <url>, "-y", "--delete-data=never", "dispatch"])`. A fresh `cli.toml` gets a server-issued login on that first call (seen in the design's spike), and that login becomes the database owner. Read its token (`spacetimedb_token` in `cli.toml`) and identity (`spacetime login show`); write *user*'s board `host.json` with `src/host_file/mod.rs::set_credential` and `adopt_user_identity_once`. *other* needs no CLI login: its board starts from an empty data dir and mints its own identity on connect.
  3. `boot` *user*'s board (publishes `EMBEDDED_MODULE`, which on this branch is `main`'s — this task does not change the module). Through the task and epic services create: one epic with three subtasks, one task in each status column, one standalone task.
  4. `boot` *other*'s board; create one epic and one task.
  5. From *user*'s `board_reads`, write `expected.json`.
  6. Drop the boards and the store; copy `data/` (minus `cache/`, `logs/`, `spacetime.pid`), `keys/`, `cli.toml` and *user*'s `host.json` to `tests/fixtures/previous-store/`. Write the README: what this is, that the logins are throwaway and only valid under the committed key, the three fields, and how to rotate (Task 8).
- [ ] **Step 2: Run it.** `du -sh tests/fixtures/previous-store` → under 10 MB, else stop and ask the operator.
- [ ] **Step 3:** Inspect `expected.json`: 1 *user* epic, its 3 subtasks plus the other *user* tasks; nothing *other* created.
- [ ] **Step 4: Commit** `test(startup): previous-store fixture and its generator`.

### Task 7: Smoke test 2 — upgrade from the previous store

**Files:**
- Modify: `src/runtime/tests/startup_smoke.rs`

**Interfaces:**
- Consumes: Task 5's `managed_target`, `boot`; Task 6's fixture layout.

- [ ] **Step 1: Write the test** `an_upgrade_from_the_previous_store_keeps_every_task_and_epic`:
  - skip rule; assert `keys/id_ecdsa` exists (a fixture without its key cannot verify its logins);
  - copy `tests/fixtures/previous-store/` to a tempdir; `LiveStore::start` on the copy's `data/`, `keys/`, `cli.toml`;
  - board data dir gets the copied `host.json`;
  - `eprintln!` whether the recorded hash differs from `embedded_module_hash()` ("upgrade smoke: publishing over the previous module" / "upgrade smoke: module unchanged, connect only");
  - `boot` with `managed_target` over the copy; panic message `"upgrade smoke: bootstrap aborted: {e}"`;
  - read `expected.json`; compare to `board_reads.list_all()` and `list_epics()` id sets with `assert_eq!` on sorted vectors, message listing missing and extra ids.
- [ ] **Step 2:** Run it → PASS. Then prove it can fail: edit the copied `expected.json` in the test temporarily to add id `999999`, run → FAIL naming `999999`; revert.
- [ ] **Step 3: Commit** `test(startup): upgrade smoke test from the previous store`.

### Task 8: Rotation script, gate and docs

**Files:**
- Create: `scripts/rotate-previous-store.sh`
- Modify: `scripts/check-spacetime-module.sh`, `docs/testing.md`, `tests/fixtures/previous-store/README.md`

- [ ] **Step 1: Write the rotation script.** Steps, each failing loudly: copy the fixture to a temp dir; `spacetime start` on the copy with its keys on a free port; `git show main:src/spacetime/module.wasm > <tmp>/main.wasm`; `spacetime --config-path=<copy>/cli.toml publish -b <tmp>/main.wasm -s <url> -y --delete-data=never dispatch`; write `sha256:<hex>` of `main.wasm` into the copy's module hash record; stop the store; drop `cache/ logs/ spacetime.pid`; replace the fixture's `data/`; update the README's three fields. It does not touch `expected.json` (rows must not change across an upgrade). Uses local `main`, never `origin/main`.
- [ ] **Step 2: Add the gate** to `scripts/check-spacetime-module.sh`: if `git rev-parse --verify -q main` succeeds, the fixture's `data/<MODULE_HASH_FILE>` must equal `sha256:` of `git show main:src/spacetime/module.wasm`; else print `skipping previous-store check: no local main` and pass. Failure text names `scripts/rotate-previous-store.sh`.
- [ ] **Step 3:** Run `./scripts/check-spacetime-module.sh; echo $?` → `0` (fixture was made from this branch's module, equal to `main`'s). Then put a wrong hash in a scratch copy and point the check at it (or temporarily edit the record) → non-zero with the rotate hint; revert.
- [ ] **Step 4: Docs.** `docs/testing.md`: the two smoke tests, their skip rule, what a green upgrade run does and does not prove, and when to run `scripts/rotate-previous-store.sh`.
- [ ] **Step 5:** Run every hook check: `for s in scripts/check-*.sh; do $s || echo "FAIL $s"; done`, then `cargo fmt`, then `cargo test --no-fail-fast > <scratchpad>/t.txt 2>&1; echo $?` (timeout 600 s) → `0`.
- [ ] **Step 6: Commit** `feat(scripts): rotate and gate the previous-store fixture`.
