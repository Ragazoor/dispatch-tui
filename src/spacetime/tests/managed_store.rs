//! The managed local store — `docs/specs/startup.allium`, from
//! `AbortWhenTheManagedStoreHasNoCli` through
//! `StopTheManagedStoreWhenStartupAborts`, and the invariants
//! `ANamedStoreIsNeverManaged`, `TheManagedStoreStopsWithItsBoard` and
//! `TheManagedStoreRunsTheEmbeddedModule`.
//!
//! The orchestration is driven through [`FakePorts`] and [`FakeHashes`], which
//! record every call, so each rule's "and nothing else happened" half is
//! asserted as well as its effect. The `spacetime publish` argv is asserted
//! through `MockProcessRunner`; the address probe against real loopback
//! sockets (no server needed, no sleeps).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use proptest::prelude::*;

use crate::process::MockProcessRunner;
use crate::spacetime::managed_store::{
    address_action, after_publish, module_action, module_hash, probe_address, select_store,
    AddressAction, FileModuleHashRecord, ManagedAddressState, ManagedStore, ManagedStorePorts,
    ManagedStoreReady, ModuleAction, ModuleHashRecord, ModulePublishOutcome, SpacetimeCliPublisher,
    StoreSelection, MANAGED_DATABASE_NAME, MANAGED_STORE_ADDRESS, MANAGED_STORE_START_TIMEOUT,
    MANAGED_STORE_STOP_TIMEOUT, MODULE_HASH_FILE,
};
use crate::startup::StartupAbort;

const EMBEDDED: &str = "sha256:embedded";
const OLDER: &str = "sha256:older";

// ---------------------------------------------------------------------------
// Fakes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Call {
    Probe,
    Start,
    DatabaseExists,
    Publish,
    Stop,
}

struct FakePorts {
    state: ManagedAddressState,
    start: Result<(), String>,
    database_exists: bool,
    publish: ModulePublishOutcome,
    stop_in_time: bool,
    calls: Mutex<Vec<Call>>,
}

impl FakePorts {
    fn new(state: ManagedAddressState) -> Self {
        Self {
            state,
            start: Ok(()),
            database_exists: true,
            publish: ModulePublishOutcome::Published,
            stop_in_time: true,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn count(&self, call: Call) -> usize {
        self.calls().iter().filter(|c| **c == call).count()
    }

    fn log(&self, call: Call) {
        self.calls.lock().unwrap().push(call);
    }
}

impl ManagedStorePorts for FakePorts {
    fn probe_managed_address(&self) -> ManagedAddressState {
        self.log(Call::Probe);
        self.state
    }
    fn start_managed_store(&self) -> Result<(), String> {
        self.log(Call::Start);
        self.start.clone()
    }
    fn managed_database_exists(&self) -> bool {
        self.log(Call::DatabaseExists);
        self.database_exists
    }
    fn publish_embedded_module(&self) -> ModulePublishOutcome {
        self.log(Call::Publish);
        self.publish.clone()
    }
    fn stop_managed_store(&self) -> bool {
        self.log(Call::Stop);
        self.stop_in_time
    }
}

struct FakeHashes {
    recorded: Mutex<Option<String>>,
    keeps_records: bool,
    writes: Mutex<Vec<String>>,
}

impl FakeHashes {
    fn new(recorded: Option<&str>) -> Self {
        Self {
            recorded: Mutex::new(recorded.map(str::to_string)),
            keeps_records: true,
            writes: Mutex::new(Vec::new()),
        }
    }
    fn writes(&self) -> Vec<String> {
        self.writes.lock().unwrap().clone()
    }
    fn current(&self) -> Option<String> {
        self.recorded.lock().unwrap().clone()
    }
}

impl ModuleHashRecord for FakeHashes {
    fn recorded_module_hash(&self) -> Option<String> {
        self.current()
    }
    fn record_module_hash(&self, hash: &str) -> bool {
        self.writes.lock().unwrap().push(hash.to_string());
        if self.keeps_records {
            *self.recorded.lock().unwrap() = Some(hash.to_string());
        }
        self.keeps_records
    }
}

fn store(ports: &Arc<FakePorts>, hashes: &Arc<FakeHashes>) -> ManagedStore {
    ManagedStore::new(ports.clone(), hashes.clone(), EMBEDDED)
}

fn ready() -> ManagedStoreReady {
    ManagedStoreReady {
        server: format!("http://{MANAGED_STORE_ADDRESS}"),
        database: MANAGED_DATABASE_NAME.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Config defaults
// ---------------------------------------------------------------------------

#[test]
fn managed_store_config_matches_the_spec_defaults() {
    assert_eq!(MANAGED_STORE_ADDRESS, "127.0.0.1:3000");
    assert_eq!(MANAGED_DATABASE_NAME, "dispatch");
    assert_eq!(MANAGED_STORE_START_TIMEOUT, Duration::from_secs(15));
    assert_eq!(MANAGED_STORE_STOP_TIMEOUT, Duration::from_secs(5));
    assert!(
        MANAGED_STORE_ADDRESS.starts_with("127.0.0.1:"),
        "the managed store is loopback only"
    );
}

// ---------------------------------------------------------------------------
// AbortWhenTheManagedStoreHasNoCli / ANamedStoreIsNeverManaged
// ---------------------------------------------------------------------------

#[test]
fn no_store_named_and_no_cli_aborts_with_spacetime_cli_missing() {
    assert_eq!(
        select_store(None, || false),
        Err(StartupAbort::SpacetimeCliMissing)
    );
}

#[test]
fn no_store_named_with_the_cli_selects_the_managed_store() {
    assert_eq!(select_store(None, || true), Ok(StoreSelection::Managed));
}

#[test]
fn a_named_store_is_used_as_is_and_the_cli_is_never_asked_for() {
    let named = "http://team-store.example:3000".to_string();
    assert_eq!(
        select_store(Some(named.clone()), || panic!(
            "a named store must not consult the CLI"
        )),
        Ok(StoreSelection::Named(named))
    );
}

#[test]
fn naming_the_managed_address_itself_opts_out_of_management() {
    let spelled_out = format!("http://{MANAGED_STORE_ADDRESS}");
    assert_eq!(
        select_store(Some(spelled_out.clone()), || false),
        Ok(StoreSelection::Named(spelled_out))
    );
}

#[test]
fn a_blank_named_store_counts_as_none() {
    assert_eq!(
        select_store(Some("  ".into()), || true),
        Ok(StoreSelection::Managed)
    );
    assert_eq!(
        select_store(Some(String::new()), || false),
        Err(StartupAbort::SpacetimeCliMissing)
    );
}

// ---------------------------------------------------------------------------
// The three ManagedStoreRequested rules, as a decision
// ---------------------------------------------------------------------------

#[test]
fn a_store_answering_on_the_managed_address_is_adopted() {
    assert_eq!(
        address_action(ManagedAddressState::StoreAnswering),
        AddressAction::Adopt
    );
}

#[test]
fn nothing_listening_on_the_managed_address_starts_a_store() {
    assert_eq!(
        address_action(ManagedAddressState::NothingListening),
        AddressAction::Start
    );
}

#[test]
fn something_else_on_the_managed_address_aborts_naming_it() {
    assert_eq!(
        address_action(ManagedAddressState::HeldBySomethingElse),
        AddressAction::Abort(StartupAbort::ManagedStorePortTaken {
            address: MANAGED_STORE_ADDRESS.to_string()
        })
    );
}

// ---------------------------------------------------------------------------
// ConnectWhenTheManagedModuleIsCurrent / PublishTheEmbeddedModuleWhenTheStoreDiffers
// ---------------------------------------------------------------------------

#[test]
fn a_recorded_hash_equal_to_the_embedded_one_connects_without_publishing() {
    assert_eq!(
        module_action(Some(EMBEDDED), EMBEDDED, true),
        ModuleAction::Connect
    );
}

#[test]
fn a_recorded_hash_that_differs_publishes() {
    assert_eq!(
        module_action(Some(OLDER), EMBEDDED, true),
        ModuleAction::Publish
    );
}

#[test]
fn no_recorded_hash_publishes() {
    assert_eq!(module_action(None, EMBEDDED, true), ModuleAction::Publish);
}

#[test]
fn a_missing_dispatch_database_publishes_even_when_the_hash_matches() {
    assert_eq!(
        module_action(Some(EMBEDDED), EMBEDDED, false),
        ModuleAction::Publish
    );
}

// ---------------------------------------------------------------------------
// The three ModulePublishFinished rules, as a decision
// ---------------------------------------------------------------------------

#[test]
fn a_published_module_goes_on_to_connect() {
    assert_eq!(after_publish(&ModulePublishOutcome::Published), Ok(()));
}

#[test]
fn a_module_needing_manual_migration_aborts_with_the_error_attached() {
    assert_eq!(
        after_publish(&ModulePublishOutcome::NeedsManualMigration {
            error: "column removed".into()
        }),
        Err(StartupAbort::ModuleNeedsManualMigration {
            reason: "column removed".into()
        })
    );
}

#[test]
fn any_other_publish_failure_aborts_with_the_error_attached() {
    assert_eq!(
        after_publish(&ModulePublishOutcome::PublishFailed {
            error: "upload refused".into()
        }),
        Err(StartupAbort::ModulePublishFailed {
            reason: "upload refused".into()
        })
    );
}

// ---------------------------------------------------------------------------
// ManagedStore::bring_up — the sequence
// ---------------------------------------------------------------------------

#[test]
fn a_fresh_orchestrator_holds_nothing() {
    let ports = Arc::new(FakePorts::new(ManagedAddressState::StoreAnswering));
    let hashes = Arc::new(FakeHashes::new(Some(EMBEDDED)));
    assert!(!store(&ports, &hashes).held_by_this_board());
    assert!(ports.calls().is_empty());
}

#[test]
fn adopting_a_current_store_connects_and_holds_it_without_starting_or_publishing() {
    let ports = Arc::new(FakePorts::new(ManagedAddressState::StoreAnswering));
    let hashes = Arc::new(FakeHashes::new(Some(EMBEDDED)));
    let managed = store(&ports, &hashes);

    assert_eq!(managed.bring_up(), Ok(ready()));
    assert!(
        managed.held_by_this_board(),
        "adopting makes it this board's"
    );
    assert_eq!(ports.count(Call::Start), 0);
    assert_eq!(ports.count(Call::Publish), 0);
    assert_eq!(ports.count(Call::Stop), 0);
    assert!(hashes.writes().is_empty());
}

#[test]
fn a_port_held_by_another_program_aborts_without_starting_holding_or_stopping() {
    let ports = Arc::new(FakePorts::new(ManagedAddressState::HeldBySomethingElse));
    let hashes = Arc::new(FakeHashes::new(Some(EMBEDDED)));
    let managed = store(&ports, &hashes);

    assert_eq!(
        managed.bring_up(),
        Err(StartupAbort::ManagedStorePortTaken {
            address: MANAGED_STORE_ADDRESS.to_string()
        })
    );
    assert!(!managed.held_by_this_board());
    assert_eq!(ports.calls(), vec![Call::Probe]);
}

#[test]
fn nothing_listening_starts_the_store_then_connects_holding_it() {
    let ports = Arc::new(FakePorts::new(ManagedAddressState::NothingListening));
    let hashes = Arc::new(FakeHashes::new(Some(EMBEDDED)));
    let managed = store(&ports, &hashes);

    assert_eq!(managed.bring_up(), Ok(ready()));
    assert!(managed.held_by_this_board());
    assert_eq!(ports.count(Call::Start), 1);
    assert_eq!(ports.count(Call::Stop), 0);
}

#[test]
fn a_start_abort_says_so_when_the_store_did_not_stop_in_time() {
    let mut fake = FakePorts::new(ManagedAddressState::NothingListening);
    fake.start = Err("did not serve HTTP within 15s".into());
    fake.stop_in_time = false;
    let ports = Arc::new(fake);
    let hashes = Arc::new(FakeHashes::new(Some(EMBEDDED)));
    let managed = store(&ports, &hashes);

    assert_eq!(
        managed.bring_up(),
        Err(StartupAbort::ManagedStoreDidNotStart {
            reason: "did not serve HTTP within 15s".into(),
            stopped: false,
        })
    );
}

#[test]
fn a_store_that_does_not_start_aborts_with_its_error_and_is_still_stopped() {
    let mut fake = FakePorts::new(ManagedAddressState::NothingListening);
    fake.start = Err("did not serve HTTP within 15s".into());
    let ports = Arc::new(fake);
    let hashes = Arc::new(FakeHashes::new(Some(EMBEDDED)));
    let managed = store(&ports, &hashes);

    assert_eq!(
        managed.bring_up(),
        Err(StartupAbort::ManagedStoreDidNotStart {
            reason: "did not serve HTTP within 15s".into(),
            stopped: true,
        })
    );
    assert!(
        managed.held_by_this_board(),
        "held from the moment the start is attempted"
    );
    assert_eq!(
        ports.count(Call::Stop),
        1,
        "a slow store must not come up unowned"
    );
    assert_eq!(ports.count(Call::Publish), 0);
}

#[test]
fn a_current_module_is_not_published() {
    let ports = Arc::new(FakePorts::new(ManagedAddressState::StoreAnswering));
    let hashes = Arc::new(FakeHashes::new(Some(EMBEDDED)));
    store(&ports, &hashes).bring_up().unwrap();
    assert_eq!(ports.count(Call::Publish), 0);
}

#[test]
fn a_differing_module_is_published_then_its_hash_recorded_then_connected() {
    let ports = Arc::new(FakePorts::new(ManagedAddressState::StoreAnswering));
    let hashes = Arc::new(FakeHashes::new(Some(OLDER)));
    let managed = store(&ports, &hashes);

    assert_eq!(managed.bring_up(), Ok(ready()));
    assert_eq!(ports.count(Call::Publish), 1);
    assert_eq!(hashes.writes(), vec![EMBEDDED.to_string()]);
    assert_eq!(hashes.current().as_deref(), Some(EMBEDDED));
}

#[test]
fn a_store_with_no_recorded_hash_is_published_to() {
    let ports = Arc::new(FakePorts::new(ManagedAddressState::NothingListening));
    let hashes = Arc::new(FakeHashes::new(None));
    assert_eq!(store(&ports, &hashes).bring_up(), Ok(ready()));
    assert_eq!(ports.count(Call::Publish), 1);
    assert_eq!(hashes.current().as_deref(), Some(EMBEDDED));
}

#[test]
fn a_store_missing_the_dispatch_database_is_published_to_despite_a_matching_hash() {
    let mut fake = FakePorts::new(ManagedAddressState::StoreAnswering);
    fake.database_exists = false;
    let ports = Arc::new(fake);
    let hashes = Arc::new(FakeHashes::new(Some(EMBEDDED)));
    assert_eq!(store(&ports, &hashes).bring_up(), Ok(ready()));
    assert_eq!(ports.count(Call::Publish), 1);
}

#[test]
fn a_hash_that_cannot_be_recorded_still_connects() {
    let ports = Arc::new(FakePorts::new(ManagedAddressState::StoreAnswering));
    let mut fake = FakeHashes::new(Some(OLDER));
    fake.keeps_records = false;
    let hashes = Arc::new(fake);
    let managed = store(&ports, &hashes);

    assert_eq!(managed.bring_up(), Ok(ready()));
    assert_eq!(hashes.writes(), vec![EMBEDDED.to_string()]);
    assert_eq!(ports.count(Call::Stop), 0);
}

#[test]
fn a_module_needing_manual_migration_aborts_records_nothing_and_stops_the_store() {
    let mut fake = FakePorts::new(ManagedAddressState::StoreAnswering);
    fake.publish = ModulePublishOutcome::NeedsManualMigration {
        error: "Aborting publish due to required manual migration.".into(),
    };
    let ports = Arc::new(fake);
    let hashes = Arc::new(FakeHashes::new(Some(OLDER)));
    let managed = store(&ports, &hashes);

    assert_eq!(
        managed.bring_up(),
        Err(StartupAbort::ModuleNeedsManualMigration {
            reason: "Aborting publish due to required manual migration.".into()
        })
    );
    assert!(
        hashes.writes().is_empty(),
        "a hash names only a module the store runs"
    );
    assert_eq!(hashes.current().as_deref(), Some(OLDER));
    assert_eq!(ports.count(Call::Stop), 1);
}

#[test]
fn a_failed_publish_aborts_records_nothing_and_stops_the_store() {
    let mut fake = FakePorts::new(ManagedAddressState::NothingListening);
    fake.publish = ModulePublishOutcome::PublishFailed {
        error: "module failed to initialise".into(),
    };
    let ports = Arc::new(fake);
    let hashes = Arc::new(FakeHashes::new(None));
    let managed = store(&ports, &hashes);

    assert_eq!(
        managed.bring_up(),
        Err(StartupAbort::ModulePublishFailed {
            reason: "module failed to initialise".into()
        })
    );
    assert!(hashes.writes().is_empty());
    assert_eq!(ports.count(Call::Stop), 1);
}

#[test]
fn publishing_happens_before_bring_up_returns_so_before_any_connection() {
    let ports = Arc::new(FakePorts::new(ManagedAddressState::NothingListening));
    let hashes = Arc::new(FakeHashes::new(None));
    store(&ports, &hashes).bring_up().unwrap();
    let calls = ports.calls();
    let start = calls.iter().position(|c| *c == Call::Start).unwrap();
    let publish = calls.iter().position(|c| *c == Call::Publish).unwrap();
    assert!(
        start < publish,
        "the store is serving before it is published to: {calls:?}"
    );
}

// ---------------------------------------------------------------------------
// StopTheManagedStoreWhenTheBoardExits / StopTheManagedStoreWhenStartupAborts
// ---------------------------------------------------------------------------

#[test]
fn the_board_exit_stops_a_store_it_started() {
    let ports = Arc::new(FakePorts::new(ManagedAddressState::NothingListening));
    let hashes = Arc::new(FakeHashes::new(Some(EMBEDDED)));
    let managed = store(&ports, &hashes);
    managed.bring_up().unwrap();

    assert!(managed.stop_on_exit());
    assert_eq!(ports.count(Call::Stop), 1);
}

#[test]
fn the_board_exit_stops_a_store_it_adopted() {
    let ports = Arc::new(FakePorts::new(ManagedAddressState::StoreAnswering));
    let hashes = Arc::new(FakeHashes::new(Some(EMBEDDED)));
    let managed = store(&ports, &hashes);
    managed.bring_up().unwrap();

    assert!(managed.stop_on_exit());
    assert_eq!(ports.count(Call::Stop), 1);
}

#[test]
fn the_board_exit_stops_nothing_it_does_not_hold() {
    let ports = Arc::new(FakePorts::new(ManagedAddressState::StoreAnswering));
    let hashes = Arc::new(FakeHashes::new(Some(EMBEDDED)));
    let managed = store(&ports, &hashes);

    assert!(managed.stop_on_exit());
    assert_eq!(ports.count(Call::Stop), 0);
}

#[test]
fn a_stop_that_runs_out_its_timeout_is_reported() {
    let mut fake = FakePorts::new(ManagedAddressState::NothingListening);
    fake.stop_in_time = false;
    let ports = Arc::new(fake);
    let hashes = Arc::new(FakeHashes::new(Some(EMBEDDED)));
    let managed = store(&ports, &hashes);
    managed.bring_up().unwrap();

    assert!(!managed.stop_on_exit());
    assert_eq!(ports.count(Call::Stop), 1);
}

#[test]
fn a_later_startup_abort_stops_a_held_store_and_keeps_its_reason() {
    let ports = Arc::new(FakePorts::new(ManagedAddressState::StoreAnswering));
    let hashes = Arc::new(FakeHashes::new(Some(EMBEDDED)));
    let managed = store(&ports, &hashes);
    managed.bring_up().unwrap();

    let reason = StartupAbort::StoreUnavailable {
        reason: "subscription refused".into(),
    };
    assert_eq!(managed.abort_startup(reason.clone()), reason);
    assert_eq!(ports.count(Call::Stop), 1);
}

#[test]
fn a_startup_abort_before_anything_is_held_stops_nothing() {
    let ports = Arc::new(FakePorts::new(ManagedAddressState::StoreAnswering));
    let hashes = Arc::new(FakeHashes::new(Some(EMBEDDED)));
    let managed = store(&ports, &hashes);

    let reason = StartupAbort::AgentPortUnavailable { port: 8899 };
    assert_eq!(managed.abort_startup(reason.clone()), reason);
    assert_eq!(ports.count(Call::Stop), 0);
}

// ---------------------------------------------------------------------------
// Invariants, over every combination the ports can answer
// ---------------------------------------------------------------------------

fn any_state() -> impl Strategy<Value = ManagedAddressState> {
    prop_oneof![
        Just(ManagedAddressState::StoreAnswering),
        Just(ManagedAddressState::NothingListening),
        Just(ManagedAddressState::HeldBySomethingElse),
    ]
}

fn any_outcome() -> impl Strategy<Value = ModulePublishOutcome> {
    prop_oneof![
        Just(ModulePublishOutcome::Published),
        Just(ModulePublishOutcome::NeedsManualMigration {
            error: "manual".into()
        }),
        Just(ModulePublishOutcome::PublishFailed {
            error: "failed".into()
        }),
    ]
}

proptest! {
    /// TheManagedStoreRunsTheEmbeddedModule: bring_up hands back somewhere to
    /// connect only when the store already ran this binary's module, or the
    /// module was just published and its hash recorded.
    #[test]
    fn the_managed_store_runs_the_embedded_module(
        state in any_state(),
        start_ok in any::<bool>(),
        recorded in prop_oneof![Just(None), Just(Some(EMBEDDED)), Just(Some(OLDER))],
        database_exists in any::<bool>(),
        outcome in any_outcome(),
        keeps_records in any::<bool>(),
    ) {
        let mut fake = FakePorts::new(state);
        fake.start = if start_ok { Ok(()) } else { Err("no".into()) };
        fake.database_exists = database_exists;
        fake.publish = outcome.clone();
        let ports = Arc::new(fake);
        let mut h = FakeHashes::new(recorded);
        h.keeps_records = keeps_records;
        let hashes = Arc::new(h);

        if store(&ports, &hashes).bring_up().is_ok() {
            let published = ports.count(Call::Publish) == 1;
            let already_current = recorded == Some(EMBEDDED) && database_exists;
            prop_assert!(
                (already_current && !published)
                    || (published
                        && outcome == ModulePublishOutcome::Published
                        && hashes.writes() == vec![EMBEDDED.to_string()]),
                "connected without the embedded module: calls {:?}", ports.calls()
            );
        }
    }

    /// TheManagedStoreStopsWithItsBoard: every abort bring_up returns has
    /// stopped a store this board holds, exactly once, and a store it does not
    /// hold is never stopped.
    #[test]
    fn the_managed_store_stops_with_its_board(
        state in any_state(),
        start_ok in any::<bool>(),
        recorded in prop_oneof![Just(None), Just(Some(EMBEDDED)), Just(Some(OLDER))],
        database_exists in any::<bool>(),
        outcome in any_outcome(),
    ) {
        let mut fake = FakePorts::new(state);
        fake.start = if start_ok { Ok(()) } else { Err("no".into()) };
        fake.database_exists = database_exists;
        fake.publish = outcome;
        let ports = Arc::new(fake);
        let hashes = Arc::new(FakeHashes::new(recorded));
        let managed = store(&ports, &hashes);

        match managed.bring_up() {
            Err(_) if managed.held_by_this_board() => prop_assert_eq!(ports.count(Call::Stop), 1),
            Err(_) => prop_assert_eq!(ports.count(Call::Stop), 0),
            Ok(_) => {
                prop_assert!(managed.held_by_this_board());
                prop_assert_eq!(ports.count(Call::Stop), 0);
                managed.stop_on_exit();
                prop_assert_eq!(ports.count(Call::Stop), 1);
            }
        }
        prop_assert_eq!(
            managed.held_by_this_board(),
            state != ManagedAddressState::HeldBySomethingElse
        );
    }
}

// ---------------------------------------------------------------------------
// module_hash
// ---------------------------------------------------------------------------

#[test]
fn the_module_hash_is_a_prefixed_sha256_of_the_wasm_bytes() {
    assert_eq!(
        module_hash(b""),
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(module_hash(b"\0asm one"), module_hash(b"\0asm one"));
    assert_ne!(module_hash(b"\0asm one"), module_hash(b"\0asm two"));
}

// ---------------------------------------------------------------------------
// FileModuleHashRecord
// ---------------------------------------------------------------------------

#[test]
fn the_hash_record_lives_in_the_data_directory() {
    let dir = tempfile::tempdir().unwrap();
    let record = FileModuleHashRecord::in_data_dir(dir.path());
    assert_eq!(record.path(), dir.path().join(MODULE_HASH_FILE));
}

#[test]
fn no_hash_file_means_no_recorded_hash() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        FileModuleHashRecord::in_data_dir(dir.path()).recorded_module_hash(),
        None
    );
}

#[test]
fn a_blank_hash_file_means_no_recorded_hash() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(MODULE_HASH_FILE), "\n").unwrap();
    assert_eq!(
        FileModuleHashRecord::in_data_dir(dir.path()).recorded_module_hash(),
        None
    );
}

#[test]
fn a_recorded_hash_reads_back_across_instances() {
    let dir = tempfile::tempdir().unwrap();
    assert!(FileModuleHashRecord::in_data_dir(dir.path()).record_module_hash(EMBEDDED));
    assert_eq!(
        FileModuleHashRecord::in_data_dir(dir.path()).recorded_module_hash(),
        Some(EMBEDDED.to_string())
    );
    assert!(FileModuleHashRecord::in_data_dir(dir.path()).record_module_hash(OLDER));
    assert_eq!(
        FileModuleHashRecord::in_data_dir(dir.path()).recorded_module_hash(),
        Some(OLDER.to_string())
    );
}

#[test]
fn a_hash_that_cannot_be_written_reports_false_rather_than_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let not_a_dir = dir.path().join("a-file");
    std::fs::write(&not_a_dir, "x").unwrap();
    assert!(!FileModuleHashRecord::in_data_dir(&not_a_dir).record_module_hash(EMBEDDED));
}

// ---------------------------------------------------------------------------
// SpacetimeCliPublisher — PublishingNeverClearsData
// ---------------------------------------------------------------------------

fn publish_with(
    response: anyhow::Result<std::process::Output>,
) -> (ModulePublishOutcome, Vec<String>) {
    let runner = Arc::new(MockProcessRunner::new(vec![response]).with_queued_window_lookup());
    let outcome =
        SpacetimeCliPublisher::new(runner.clone(), "/data/dispatch_module.wasm").publish();
    let calls = runner.recorded_calls();
    assert_eq!(calls.len(), 1, "one publish, one shell-out: {calls:?}");
    assert_eq!(calls[0].0, "spacetime");
    (outcome, calls[0].1.clone())
}

#[test]
fn publish_targets_the_managed_database_with_the_embedded_wasm_and_never_clears_data() {
    let (_, argv) = publish_with(MockProcessRunner::ok());

    assert!(argv.contains(&"publish".to_string()), "{argv:?}");
    let bin = argv
        .iter()
        .position(|a| a == "-b" || a == "--bin-path")
        .expect("publishes the prebuilt wasm, not a build");
    assert_eq!(argv[bin + 1], "/data/dispatch_module.wasm");
    let server = argv
        .iter()
        .position(|a| a == "-s" || a == "--server")
        .expect("names the server");
    assert!(argv[server + 1].contains(MANAGED_STORE_ADDRESS), "{argv:?}");
    assert_eq!(argv.last().map(String::as_str), Some(MANAGED_DATABASE_NAME));

    assert!(
        argv.contains(&"--delete-data=never".to_string()),
        "{argv:?}"
    );
    for arg in &argv {
        let clears = arg == "-c" || (arg.contains("delete-data") && arg != "--delete-data=never");
        assert!(
            !clears,
            "a data-clearing option reached the publish: {arg} in {argv:?}"
        );
    }
}

#[test]
fn a_successful_publish_is_published() {
    assert_eq!(
        publish_with(MockProcessRunner::ok()).0,
        ModulePublishOutcome::Published
    );
}

#[test]
fn a_refused_automigration_is_needs_manual_migration() {
    for stderr in [
        "Aborting publish due to required manual migration.",
        "Aborting because publishing would require manual migration or deletion of data and --delete-data was not specified.",
    ] {
        match publish_with(MockProcessRunner::fail(stderr)).0 {
            ModulePublishOutcome::NeedsManualMigration { error } => {
                assert!(error.contains(stderr), "{error}")
            }
            other => panic!("{stderr:?} classified as {other:?}"),
        }
    }
}

#[test]
fn any_other_publish_error_is_publish_failed_with_the_error() {
    match publish_with(MockProcessRunner::fail("error: connection refused")).0 {
        ModulePublishOutcome::PublishFailed { error } => {
            assert!(error.contains("connection refused"), "{error}")
        }
        other => panic!("classified as {other:?}"),
    }
}

#[test]
fn a_publish_that_could_not_run_is_publish_failed() {
    let (outcome, _) = publish_with(Err(anyhow::anyhow!("spacetime: not found")));
    assert!(
        matches!(outcome, ModulePublishOutcome::PublishFailed { ref error } if error.contains("not found")),
        "{outcome:?}"
    );
}

// ---------------------------------------------------------------------------
// probe_address — against real loopback sockets
// ---------------------------------------------------------------------------

const PROBE: Duration = Duration::from_millis(500);

/// Accept connections forever on a background thread, answering each with
/// `reply` (or nothing, holding the socket, when `None`).
fn serve(reply: Option<&'static [u8]>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            match reply {
                Some(bytes) => {
                    let _ = stream.write_all(bytes);
                }
                None => held.push(stream),
            }
        }
    });
    address
}

#[test]
fn a_port_nothing_listens_on_probes_as_nothing_listening() {
    let address = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().to_string()
    };
    assert_eq!(
        probe_address(&address, PROBE),
        ManagedAddressState::NothingListening
    );
}

#[test]
fn a_server_answering_the_store_ping_probes_as_a_store() {
    let address = serve(Some(
        b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
    ));
    assert_eq!(
        probe_address(&address, PROBE),
        ManagedAddressState::StoreAnswering
    );
}

#[test]
fn an_http_server_that_is_not_a_store_probes_as_something_else() {
    let address = serve(Some(
        b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
    ));
    assert_eq!(
        probe_address(&address, PROBE),
        ManagedAddressState::HeldBySomethingElse
    );
}

#[test]
fn a_non_http_listener_probes_as_something_else() {
    let address = serve(Some(b"SSH-2.0-OpenSSH_9.6\r\n"));
    assert_eq!(
        probe_address(&address, PROBE),
        ManagedAddressState::HeldBySomethingElse
    );
}

#[test]
fn a_listener_that_never_answers_probes_as_something_else_within_the_timeout() {
    let address = serve(None);
    assert_eq!(
        probe_address(&address, PROBE),
        ManagedAddressState::HeldBySomethingElse
    );
}

// ---------------------------------------------------------------------------
// The managed store's abort messages
// ---------------------------------------------------------------------------

#[test]
fn the_managed_store_abort_messages_name_their_remedy() {
    let cli = StartupAbort::SpacetimeCliMissing.message();
    assert!(cli.contains("spacetime"), "{cli}");
    assert!(
        cli.contains("--spacetime-server") || cli.contains("DISPATCH_SPACETIME_SERVER"),
        "{cli}"
    );

    let port = StartupAbort::ManagedStorePortTaken {
        address: MANAGED_STORE_ADDRESS.into(),
    }
    .message();
    assert!(port.contains(MANAGED_STORE_ADDRESS), "{port}");

    let start = StartupAbort::ManagedStoreDidNotStart {
        reason: "exited with status 1".into(),
        stopped: true,
    }
    .message();
    assert!(start.contains("exited with status 1"), "{start}");
    assert!(start.contains("Nothing was left running"), "{start}");
    let unstopped = StartupAbort::ManagedStoreDidNotStart {
        reason: "exited with status 1".into(),
        stopped: false,
    }
    .message();
    assert!(
        !unstopped.contains("Nothing was left running"),
        "a store that did not stop in time may still be running: {unstopped}"
    );
    assert!(unstopped.contains("may still be running"), "{unstopped}");

    let manual = StartupAbort::ModuleNeedsManualMigration {
        reason: "column removed".into(),
    }
    .message();
    assert!(manual.contains("column removed"), "{manual}");
    assert!(manual.contains(MANAGED_DATABASE_NAME), "{manual}");

    let failed = StartupAbort::ModulePublishFailed {
        reason: "upload refused".into(),
    }
    .message();
    assert!(failed.contains("upload refused"), "{failed}");

    let all = [cli, port, start, manual, failed];
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert_ne!(a, b, "each abort is repaired by a different act");
        }
    }
}

#[test]
fn the_connection_and_the_managed_store_name_the_same_database() {
    // The board connects to `sync::SHARED_DATABASE_NAME`; the managed store
    // publishes `MANAGED_DATABASE_NAME`. Two constants that drift apart would
    // have every board connect to a database nothing published.
    assert_eq!(crate::sync::SHARED_DATABASE_NAME, MANAGED_DATABASE_NAME);
}

#[test]
fn normalize_server_trims_and_treats_blank_as_none() {
    use crate::spacetime::managed_store::normalize_server;
    assert_eq!(normalize_server(None), None);
    assert_eq!(normalize_server(Some(String::new())), None);
    assert_eq!(normalize_server(Some("  \t".into())), None);
    assert_eq!(
        normalize_server(Some(" http://h:1 ".into())),
        Some("http://h:1".to_string())
    );
}
