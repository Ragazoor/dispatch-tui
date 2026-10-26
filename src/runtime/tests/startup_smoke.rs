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
//! config and JWT key pair of the test's own. The board's `StartupPaths` are
//! temp paths too, so the configuration check never reaches `$HOME`.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::live_store::{live_store_tests_should_run, LiveStore};
use super::*;
use crate::models::TaskId;
use crate::spacetime::managed_store::{
    embedded_module_hash, FileModuleHashRecord, ManagedStore, ManagedStoreLayout, ModuleHashRecord,
    SpacetimeManagedStore, SpacetimeStartSpawner, EMBEDDED_MODULE,
};

/// How long a write may take to come back through the subscription.
const ROW_DEADLINE: Duration = Duration::from_secs(20);

/// A managed store laid out on `store`'s address, over `store_data_dir` (where
/// the module hash is recorded), publishing with `cli_config`'s login.
pub(super) fn managed_target(
    store: &LiveStore,
    store_data_dir: &Path,
    log_dir: &Path,
    cli_config: &Path,
) -> StoreTarget {
    let layout = ManagedStoreLayout {
        address: store.address(),
        data_dir: store_data_dir.to_path_buf(),
        log_path: log_dir.join("managed-store.log"),
    };
    let hashes = Arc::new(FileModuleHashRecord::in_data_dir(store_data_dir));
    let ports = SpacetimeManagedStore::new(
        Arc::new(crate::process::RealProcessRunner::default()),
        Arc::new(SpacetimeStartSpawner::new().with_config_path(cli_config)),
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
pub(super) fn board_dir(root: &Path) -> (std::path::PathBuf, StartupPaths) {
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
pub(super) async fn boot(
    data_dir: &Path,
    paths: &StartupPaths,
    target: StoreTarget,
) -> Result<Bootstrap> {
    TuiRuntime::bootstrap(BootstrapInputs {
        target,
        build_store: StoreParts::build,
        ..bootstrap_inputs(data_dir, paths, StoreParts::build)
    })
    .await
}

/// Wait until the board's reads hold `task`, or panic on the deadline.
pub(super) async fn await_task(b: &Bootstrap, task: TaskId) {
    let deadline = Instant::now() + ROW_DEADLINE;
    while !b
        .runtime
        .board_reads
        .list_all()
        .await
        .iter()
        .any(|t| t.id == task)
    {
        assert!(
            Instant::now() < deadline,
            "task {task:?} never reached the board's reads"
        );
        // allow-test-sleep: deadline-bounded poll on the subscription.
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub(super) fn new_task(
    title: &str,
    epic_id: Option<crate::models::EpicId>,
) -> crate::service::CreateTaskParams {
    crate::service::CreateTaskParams {
        title: title.to_string(),
        description: String::new(),
        repo_path: "/tmp/smoke-repo".to_string(),
        plan_path: None,
        epic_id,
        sort_order: None,
        tag: None,
        base_branch: None,
        wrap_up_mode: None,
        model: None,
        auto_run_plan: false,
        phoenix: false,
        schedule: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fresh_install_starts_against_a_real_store() {
    if !live_store_tests_should_run() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let store_data = dir.path().join("store");
    let cli_config = dir.path().join("cli.toml");
    let store = LiveStore::start(&store_data, &dir.path().join("keys"), &cli_config);
    let (data_dir, paths) = board_dir(dir.path());
    let target = || managed_target(&store, &store_data, dir.path(), &cli_config);

    let first = boot(&data_dir, &paths, target())
        .await
        .unwrap_or_else(|e| panic!("fresh-install smoke: bootstrap aborted: {e}"));
    assert_eq!(
        first.store_server,
        store.url(),
        "the board connected somewhere other than the store it brought up"
    );
    assert_eq!(
        FileModuleHashRecord::in_data_dir(&store_data).recorded_module_hash(),
        Some(embedded_module_hash()),
        "the embedded module was not recorded as published"
    );

    let task = first
        .runtime
        .task_svc
        .create_task(new_task("smoke", None))
        .await
        .expect("create a task through the service");
    await_task(&first, task).await;
    drop(first);

    let second = boot(&data_dir, &paths, target())
        .await
        .unwrap_or_else(|e| panic!("fresh-install smoke: second bootstrap aborted: {e}"));
    await_task(&second, task).await;
}
