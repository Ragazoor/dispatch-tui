//! Startup smoke tests: the real composition root, `TuiRuntime::bootstrap`,
//! against a real throwaway store — `startup.allium`:
//! `StartupIsSmokeTestedAgainstARealStore`. Design:
//! `docs/superpowers/specs/2026-10-26-startup-smoke-test-design.md`.
//!
//! Every other bootstrap test runs over the in-memory store stand-in, so a
//! change that breaks the real publish, connection, identity handshake or
//! subscription passes them all and fails at the operator's next launch —
//! where dispatch, the tool they would fix it with, is the thing that is down.
//!
//! The store is never the operator's: a free port, and a data directory, CLI
//! config and JWT key pair of the test's own. The fresh-install test leaves
//! the port empty and lets bootstrap start the store; the upgrade test starts
//! it itself (`LiveStore`) so it can copy a committed directory in first. The board's `StartupPaths` are
//! temp paths too, so the configuration check never reaches `$HOME`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use super::live_store::{live_store_tests_should_run, LiveStore};
use super::*;
use crate::models::TaskId;
use crate::spacetime::managed_store::{
    embedded_module_hash, FileModuleHashRecord, ManagedStore, ManagedStoreLayout, ModuleHashRecord,
    SpacetimeManagedStore, SpacetimeStartSpawner, EMBEDDED_MODULE,
};

/// How long a write may take to come back through the subscription.
const ROW_DEADLINE: Duration = Duration::from_secs(20);

/// A free loopback port with nothing on it, as `host:port`.
fn free_address() -> String {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("pick a free port")
        .to_string()
}

/// A managed store laid out on `address`, over `store_data_dir` (where the
/// module hash is recorded). It starts the store itself when nothing answers
/// there, with `cli_config`'s login and `keys_dir`'s JWT key pair, so neither
/// reaches the operator's own.
fn managed_target(
    address: String,
    store_data_dir: &Path,
    log_dir: &Path,
    cli_config: &Path,
    keys_dir: &Path,
) -> StoreTarget {
    let layout = ManagedStoreLayout {
        address,
        data_dir: store_data_dir.to_path_buf(),
        log_path: log_dir.join("managed-store.log"),
    };
    let hashes = Arc::new(FileModuleHashRecord::in_data_dir(store_data_dir));
    let ports = SpacetimeManagedStore::new(
        Arc::new(crate::process::RealProcessRunner::default()),
        Arc::new(
            SpacetimeStartSpawner::new()
                .with_config_path(cli_config)
                .with_key_dir(keys_dir),
        ),
        layout,
        EMBEDDED_MODULE,
    )
    .with_config_path(cli_config);
    StoreTarget::Managed(Arc::new(ManagedStore::new(
        Arc::new(ports),
        hashes,
        embedded_module_hash(),
    )))
}

/// A board data directory and temp `StartupPaths`, with the host already
/// named (a launch with an unnamed host stops to ask).
fn board_dir(root: &Path) -> (std::path::PathBuf, StartupPaths) {
    let data_dir = root.join("board");
    std::fs::create_dir_all(&data_dir).unwrap();
    let paths = StartupPaths {
        claude_dir: root.join("claude"),
        claude_json_path: root.join(".claude.json"),
    };
    crate::host_file::resolve_for_launch(&data_dir).unwrap();
    crate::host_file::rename_host(&data_dir, "smoke-test-host").unwrap();
    (data_dir, paths)
}

/// The real composition root over the real store wiring.
async fn boot(data_dir: &Path, paths: &StartupPaths, target: StoreTarget) -> Result<Bootstrap> {
    TuiRuntime::bootstrap(BootstrapInputs {
        target,
        ..bootstrap_inputs(data_dir, paths, StoreParts::build)
    })
    .await
}

/// Wait until the board's reads hold `task`, or panic on the deadline. Woken
/// by the shared rows' change signal (which `dispatch_request_changes` hands
/// out for every row, not only requests), never by a timer.
async fn await_task(b: &Bootstrap, task: TaskId) {
    let deadline = tokio::time::Instant::now() + ROW_DEADLINE;
    let mut changes = b.runtime.board_reads.dispatch_request_changes();
    while !b
        .runtime
        .board_reads
        .list_all()
        .await
        .iter()
        .any(|t| t.id == task)
    {
        tokio::time::timeout_at(deadline, changes.changed())
            .await
            .unwrap_or_else(|_| panic!("task {task:?} never reached the board's reads"))
            .expect("the rows outlive the board");
    }
}

fn new_task(
    title: &str,
    epic_id: Option<crate::models::EpicId>,
) -> crate::service::CreateTaskParams {
    crate::service::CreateTaskParams {
        epic_id,
        ..crate::service::CreateTaskParams::fixture(title, "/tmp/smoke-repo")
    }
}

/// A real first launch: nothing is listening, so bootstrap's managed store
/// takes the start branch itself -- spawns `spacetime start`, waits until it
/// serves, publishes. Each launch's store is stopped by the board's own guard
/// when the launch ends, as on a real quit, so the second launch starts it
/// again over the data the first left.
#[tokio::test(flavor = "multi_thread")]
async fn a_fresh_install_starts_against_a_real_store() {
    if !live_store_tests_should_run() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let store_data = dir.path().join("store");
    let cli_config = dir.path().join("cli.toml");
    let keys = dir.path().join("keys");
    let address = free_address();
    let (data_dir, paths) = board_dir(dir.path());
    let target = || managed_target(address.clone(), &store_data, dir.path(), &cli_config, &keys);

    let first_target = target();
    // Declared before `first`, so the store outlives the board it serves and
    // is still stopped when an assertion panics.
    let first_guard = ManagedStoreGuard(first_target.managed().cloned());
    let first = boot(&data_dir, &paths, first_target)
        .await
        .unwrap_or_else(|e| panic!("fresh-install smoke: bootstrap aborted: {e}"));
    assert_eq!(
        first.store_server,
        format!("http://{address}"),
        "the board connected somewhere other than the store it brought up"
    );
    assert_eq!(
        FileModuleHashRecord::in_data_dir(&store_data).recorded_module_hash(),
        Some(embedded_module_hash()),
        "the embedded module was not recorded as published"
    );
    assert!(
        keys.join("id_ecdsa").is_file(),
        "the store signed with a key other than the test's own"
    );

    let task = first
        .runtime
        .task_svc
        .create_task(new_task("smoke", None))
        .await
        .expect("create a task through the service");
    await_task(&first, task).await;
    drop(first);
    drop(first_guard);
    assert!(
        std::net::TcpListener::bind(&address).is_ok(),
        "the store still holds {address} after the board's guard dropped"
    );

    // The design's "without publishing": a publish rewrites the hash record.
    let record = FileModuleHashRecord::in_data_dir(&store_data);
    let recorded_at = || {
        std::fs::metadata(record.path())
            .unwrap()
            .modified()
            .unwrap()
    };
    let before = recorded_at();
    let second_target = target();
    let _second_guard = ManagedStoreGuard(second_target.managed().cloned());
    let second = boot(&data_dir, &paths, second_target)
        .await
        .unwrap_or_else(|e| panic!("fresh-install smoke: second bootstrap aborted: {e}"));
    assert_eq!(
        recorded_at(),
        before,
        "a second launch on an unchanged module published again"
    );
    await_task(&second, task).await;
}

// ---------------------------------------------------------------------------
// The previous-store fixture
// ---------------------------------------------------------------------------

/// `tests/fixtures/previous-store/`, the committed store directory the
/// upgrade test starts from.
fn previous_store_fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/previous-store")
}

/// The ids *user* sees, in `expected.json`.
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct ExpectedRows {
    tasks: Vec<i64>,
    epics: Vec<i64>,
}

/// What `b`'s board shows, ids ascending.
async fn visible_rows(b: &Bootstrap) -> ExpectedRows {
    let mut tasks: Vec<i64> = b
        .runtime
        .board_reads
        .list_all()
        .await
        .iter()
        .map(|t| t.id.0)
        .collect();
    let mut epics: Vec<i64> = b
        .runtime
        .board_reads
        .list_epics()
        .await
        .iter()
        .map(|e| e.id.0)
        .collect();
    tasks.sort_unstable();
    epics.sort_unstable();
    ExpectedRows { tasks, epics }
}

/// The CLI login's token, read out of its config file.
fn cli_token(cli_config: &Path) -> String {
    std::fs::read_to_string(cli_config)
        .unwrap()
        .lines()
        .find_map(|l| l.trim().strip_prefix("spacetimedb_token = "))
        .map(|v| v.trim_matches('"').to_string())
        .expect("the CLI config holds no login token")
}

/// The CLI login's identity, in the hex the module stamps.
fn cli_identity(cli_config: &Path) -> String {
    let shown = super::live_store::spacetime(cli_config, &["login", "show"]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&shown.stdout),
        String::from_utf8_lossy(&shown.stderr)
    );
    text.split(|c: char| !c.is_ascii_hexdigit())
        .find(|w| w.len() == 64)
        .unwrap_or_else(|| panic!("no identity in `spacetime login show`: {text}"))
        .to_lowercase()
}

/// Copy `from` into `to`, leaving out any file or directory named in `skip`
/// at any depth.
fn copy_dir(from: &Path, to: &Path, skip: &[&str]) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if skip.iter().any(|s| name == **s) {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target, skip);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// What the store rebuilds on its own, or what only one run of it means.
const NOT_COMMITTED: &[&str] = &[
    "cache",
    "logs",
    "module_logs",
    "spacetime.pid",
    "db.lock",
    "dispatch_module.wasm",
];

/// Make `tests/fixtures/previous-store/` from this binary's embedded module,
/// with fresh keys and logins. Run through `scripts/rotate-previous-store.sh`,
/// which first checks the embedded module is `main`'s. Rebuilt fresh rather
/// than published over: every publish appends the module to the store's
/// commit log, so a fixture moved forward that way grows ~1.6 MB a time.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "writes the committed fixture; run by hand"]
async fn generate_previous_store_fixture() {
    assert!(
        live_store_tests_should_run(),
        "the generator needs spacetime"
    );
    let dir = tempfile::tempdir().unwrap();
    let store_data = dir.path().join("data");
    let keys = dir.path().join("keys");
    let cli_config = dir.path().join("cli.toml");
    let store = LiveStore::start(&store_data, &keys, &cli_config);

    // The first CLI call gets a server-issued login, and the login that
    // publishes owns the database: that is *user*.
    let wasm = dir.path().join("module.wasm");
    std::fs::write(&wasm, EMBEDDED_MODULE).unwrap();
    let published = crate::spacetime::managed_store::SpacetimeCliPublisher::new(
        Arc::new(crate::process::RealProcessRunner::default()),
        &wasm,
    )
    .with_server(store.url())
    .with_config_path(&cli_config)
    .publish();
    assert!(
        matches!(
            published,
            crate::spacetime::managed_store::ModulePublishOutcome::Published
        ),
        "{published:?}"
    );
    let user = cli_identity(&cli_config);
    let token = cli_token(&cli_config);

    let user_root = dir.path().join("user");
    let (user_dir, user_paths) = board_dir(&user_root);
    crate::host_file::adopt_user_identity(&user_dir, &user, &token).unwrap();
    let target = || managed_target(store.address(), &store_data, dir.path(), &cli_config, &keys);

    let ub = boot(&user_dir, &user_paths, target())
        .await
        .expect("user's board starts");
    let epic = ub
        .runtime
        .epic_svc
        .create_epic(crate::service::CreateEpicParams::fixture("smoke epic"))
        .await
        .unwrap();
    let mut created = Vec::new();
    for title in ["subtask one", "subtask two", "subtask three"] {
        created.push(
            ub.runtime
                .task_svc
                .create_task(new_task(title, Some(epic.id)))
                .await
                .unwrap(),
        );
    }
    created.push(
        ub.runtime
            .task_svc
            .create_task(new_task("standalone", None))
            .await
            .unwrap(),
    );
    for task in &created {
        await_task(&ub, *task).await;
    }

    // *other*: a second person on the same store, minting their own identity.
    let other_root = dir.path().join("other");
    let (other_dir, other_paths) = board_dir(&other_root);
    let ob = boot(&other_dir, &other_paths, target())
        .await
        .expect("other's board starts");
    let other_epic = ob
        .runtime
        .epic_svc
        .create_epic(crate::service::CreateEpicParams::fixture("other's epic"))
        .await
        .unwrap();
    let other_task = ob
        .runtime
        .task_svc
        .create_task(new_task("other's task", None))
        .await
        .unwrap();
    await_task(&ob, other_task).await;

    let expected = visible_rows(&ub).await;
    for task in &created {
        assert!(expected.tasks.contains(&task.0), "user cannot see {task:?}");
    }
    assert!(expected.epics.contains(&epic.id.0));
    assert!(
        !expected.tasks.contains(&other_task.0),
        "other's task leaked to user"
    );
    assert!(
        !expected.epics.contains(&other_epic.id.0),
        "other's epic leaked to user"
    );
    drop(ob);
    drop(ub);
    store.stop();

    let out = previous_store_fixture();
    // Everything but the hand-written README, which says how this was made.
    for generated in ["data", "keys"] {
        let _ = std::fs::remove_dir_all(out.join(generated));
    }
    copy_dir(&store_data, &out.join("data"), NOT_COMMITTED);
    copy_dir(&keys, &out.join("keys"), &[]);
    std::fs::copy(&cli_config, out.join("cli.toml")).unwrap();
    std::fs::copy(
        crate::host_file::host_file_path(&user_dir),
        out.join("host.json"),
    )
    .unwrap();
    std::fs::write(
        out.join("expected.json"),
        serde_json::to_string_pretty(&expected).unwrap() + "\n",
    )
    .unwrap();
}

/// The agreed bar: after an upgrade from the module `main` runs, the same
/// user sees exactly the same tasks and epics as before it.
///
/// Startup always publishes here, as the store's owner. On a branch that
/// changes the module that is a migration; on one that does not, it
/// republishes the same module, so a green run there proves the owner can
/// publish but nothing about automigration. Which of the two ran is printed.
#[tokio::test(flavor = "multi_thread")]
async fn an_upgrade_from_the_previous_store_keeps_every_task_and_epic() {
    if !live_store_tests_should_run() {
        return;
    }
    let fixture = previous_store_fixture();
    assert!(
        fixture.join("keys/id_ecdsa").is_file(),
        "the fixture has no signing key, so none of its logins can verify"
    );
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&fixture, dir.path(), &[]);
    let store_data = dir.path().join("data");
    let cli_config = dir.path().join("cli.toml");
    let keys = dir.path().join("keys");
    let store = LiveStore::start(&store_data, &keys, &cli_config);

    let (data_dir, paths) = board_dir(&dir.path().join("user"));
    std::fs::copy(
        dir.path().join("host.json"),
        crate::host_file::host_file_path(&data_dir),
    )
    .unwrap();
    let expected: ExpectedRows =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("expected.json")).unwrap())
            .unwrap();

    // Always publish. With the module unchanged the hashes would match and
    // startup would only connect, never proving that the store's owner can
    // still publish to it — the step that fails when the CLI login changed.
    let record = FileModuleHashRecord::in_data_dir(&store_data);
    if record.recorded_module_hash().as_deref() == Some(embedded_module_hash().as_str()) {
        eprintln!("upgrade smoke: module unchanged; republishing it, not migrating");
        std::fs::remove_file(record.path()).unwrap();
    } else {
        eprintln!("upgrade smoke: publishing over the previous module");
    }

    let b = boot(
        &data_dir,
        &paths,
        managed_target(store.address(), &store_data, dir.path(), &cli_config, &keys),
    )
    .await
    .unwrap_or_else(|e| panic!("upgrade smoke: bootstrap aborted: {e}"));
    // The record was removed above, so only a publish brings it back: this
    // is what proves the owner's publish ran, not just that startup finished.
    assert_eq!(
        record.recorded_module_hash(),
        Some(embedded_module_hash()),
        "upgrade smoke: startup did not publish the embedded module"
    );

    let seen = visible_rows(&b).await;
    assert_eq!(seen, expected, "upgrade smoke: the user's rows changed");
}

/// `AbortWhenTheCliDoesNotOwnTheManagedDatabase` against a real store: the
/// previous store's database is owned by *user*, and the CLI config the board
/// is given holds a different login, so startup stops before publishing
/// instead of failing on the store's 403.
#[tokio::test(flavor = "multi_thread")]
async fn a_cli_login_that_does_not_own_the_database_aborts_before_publishing() {
    if !live_store_tests_should_run() {
        return;
    }
    let fixture = previous_store_fixture();
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&fixture, dir.path(), &[]);
    let store_data = dir.path().join("data");
    let owner_config = dir.path().join("cli.toml");
    let keys = dir.path().join("keys");
    let store = LiveStore::start(&store_data, &keys, &owner_config);
    let owner = cli_identity(&owner_config);

    // A config of its own: its first call against this store gets a
    // server-issued login, which is not the owner's.
    let other_config = dir.path().join("other-cli.toml");
    super::live_store::spacetime(&other_config, &["list", "-s", &store.url()]);
    let other = cli_identity(&other_config);
    assert_ne!(other, owner, "the second login must be someone else");

    let (data_dir, paths) = board_dir(&dir.path().join("user"));
    let record = FileModuleHashRecord::in_data_dir(&store_data);
    // Make a publish due, so the abort is shown to come before it.
    let _ = std::fs::remove_file(record.path());

    let aborted = boot(
        &data_dir,
        &paths,
        managed_target(
            store.address(),
            &store_data,
            dir.path(),
            &other_config,
            &keys,
        ),
    )
    .await;
    let message = match aborted {
        Ok(_) => panic!("startup went on with a CLI login that does not own the database"),
        Err(e) => e.to_string(),
    };
    assert!(
        message.contains(&other) && message.contains(&owner),
        "the abort names both identities: {message}"
    );
    assert_eq!(record.recorded_module_hash(), None, "nothing was published");
}
