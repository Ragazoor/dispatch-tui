//! An orchestrated epic's orchestrator on one machine (`core.allium:
//! Orchestrator`, `OneOrchestratorPerEpicAndHost`,
//! `OrchestratorNamesAreUnique`; `orchestrators.allium:
//! BoardRunsItsOrchestrators`, `OrchestratorWindowGone`,
//! `StartOrchestratorSession`, `OrchestratorTurnClearsTheRestartFlag`,
//! `EndOrchestratorSession`, `SwitchOrchestratorOff`), run against the
//! in-memory store.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

const HOST: &str = "host-a";
const OTHER: &str = "host-b";

fn orchestrated(store: &Store) -> i64 {
    store.register_host_for(HOST, "alice");
    store.register_host_for(OTHER, "alice");
    let epic = store.create_epic("alice", "E");
    store
        .call("alice", |ctx| switch_orchestrator_on(ctx, epic, None, None))
        .expect("switch on");
    epic
}

fn ensure(store: &Store, epic: i64, host: &str) -> Result<(), String> {
    store.call("alice", |ctx| ensure_orchestrator(ctx, epic, host.to_string()))
}

fn rows(store: &Store, epic: i64) -> Vec<Orchestrator> {
    store.db().orchestrators().epic_id().filter(epic).collect()
}

fn row_on(store: &Store, epic: i64, host: &str) -> Orchestrator {
    rows(store, epic)
        .into_iter()
        .find(|o| o.host == host)
        .expect("an orchestrator on the host")
}

fn start(store: &Store, id: i64) -> Result<(), String> {
    store.call("alice", |ctx| start_orchestrator_session(ctx, id, HOST.to_string(), false))
}

fn end(store: &Store, id: i64, died: bool) -> Result<(), String> {
    store.call("alice", |ctx| end_orchestrator_session(ctx, id, HOST.to_string(), died))
}

fn turn(store: &Store, name: &str) -> Result<(), String> {
    store.call("alice", |ctx| orchestrator_turn_completed(ctx, name.to_string()))
}

#[test]
fn ensuring_creates_one_row_per_epic_and_host() {
    let store = Store::new();
    let epic = orchestrated(&store);

    ensure(&store, epic, HOST).expect("ensure");
    ensure(&store, epic, HOST).expect("ensure again");
    ensure(&store, epic, OTHER).expect("ensure on another host");

    let all = rows(&store, epic);
    assert_eq!(all.len(), 2, "{all:?}");
    let here = row_on(&store, epic, HOST);
    assert!(here.tmux_window.is_empty());
    assert!(!here.restarted_once && !here.session_died);
    assert_ne!(here.name, row_on(&store, epic, OTHER).name);
}

#[test]
fn ensuring_without_a_switch_is_refused() {
    let store = Store::new();
    store.register_host_for(HOST, "alice");
    let epic = store.create_epic("alice", "E");

    let why = ensure(&store, epic, HOST).unwrap_err();
    assert!(why.contains("has no orchestrator"), "{why}");
    assert!(rows(&store, epic).is_empty());
}

#[test]
fn ensuring_for_a_host_the_caller_does_not_act_for_is_refused() {
    let store = Store::new();
    let epic = orchestrated(&store);
    store.register_host_for("host-z", "zed");

    assert!(ensure(&store, epic, "host-z").is_err());
    assert!(rows(&store, epic).is_empty());
}

#[test]
fn an_orchestrators_name_is_no_agents_and_no_agent_takes_it_later() {
    let store = Store::new();
    let epic = orchestrated(&store);
    ensure(&store, epic, HOST).expect("ensure");
    let name = row_on(&store, epic, HOST).name;

    for i in 0..20 {
        store.create("alice", epic_task(&format!("t{i}"), epic, "backlog"));
    }

    let agents: Vec<Agent> = store.db().agents().iter().collect();
    assert!(agents.iter().all(|a| a.name != name), "{name} also on an agent");
}

#[test]
fn a_start_records_the_window_and_the_time() {
    let store = Store::new();
    let epic = orchestrated(&store);
    ensure(&store, epic, HOST).unwrap();
    let id = row_on(&store, epic, HOST).id;

    start(&store, id).expect("start");

    let row = row_on(&store, epic, HOST);
    assert_eq!(row.tmux_window, format!("orch-{epic}"));
    assert!(!row.started_at.is_empty());
    assert!(!row.restarted_once, "a first start spends no restart");
}

#[test]
fn a_second_start_of_a_live_session_is_refused() {
    let store = Store::new();
    let epic = orchestrated(&store);
    ensure(&store, epic, HOST).unwrap();
    let id = row_on(&store, epic, HOST).id;
    start(&store, id).unwrap();

    let why = start(&store, id).unwrap_err();
    assert!(why.contains("already running"), "{why}");
}

#[test]
fn only_the_rows_own_host_starts_it() {
    let store = Store::new();
    let epic = orchestrated(&store);
    ensure(&store, epic, OTHER).unwrap();
    let id = row_on(&store, epic, OTHER).id;

    let why = start(&store, id).unwrap_err();
    assert!(why.contains("cannot start or end it"), "{why}");
}

#[test]
fn an_ended_session_is_not_a_death_and_starts_again_freely() {
    let store = Store::new();
    let epic = orchestrated(&store);
    ensure(&store, epic, HOST).unwrap();
    let id = row_on(&store, epic, HOST).id;
    start(&store, id).unwrap();

    end(&store, id, false).expect("end");
    let row = row_on(&store, epic, HOST);
    assert!(row.tmux_window.is_empty());
    assert!(!row.session_died);

    start(&store, id).expect("start again");
    assert!(!row_on(&store, epic, HOST).restarted_once);
}

#[test]
fn a_died_session_gets_one_restart_and_then_waits() {
    let store = Store::new();
    let epic = orchestrated(&store);
    ensure(&store, epic, HOST).unwrap();
    let id = row_on(&store, epic, HOST).id;
    start(&store, id).unwrap();

    end(&store, id, true).expect("window gone");
    assert!(row_on(&store, epic, HOST).session_died);

    start(&store, id).expect("the one restart");
    let row = row_on(&store, epic, HOST);
    assert!(row.restarted_once);
    assert!(!row.session_died);

    end(&store, id, true).expect("gone again");
    let why = start(&store, id).unwrap_err();
    assert!(why.contains("died twice in a row"), "{why}");
}

#[test]
fn a_completed_turn_gives_the_restart_back() {
    let store = Store::new();
    let epic = orchestrated(&store);
    ensure(&store, epic, HOST).unwrap();
    let row = row_on(&store, epic, HOST);
    start(&store, row.id).unwrap();
    end(&store, row.id, true).unwrap();
    start(&store, row.id).unwrap();

    turn(&store, &row.name).expect("turn");
    let after = row_on(&store, epic, HOST);
    assert!(!after.restarted_once);
    assert!(!after.last_turn_at.is_empty());

    end(&store, row.id, true).unwrap();
    start(&store, row.id).expect("a fresh bout gets its restart");
}

#[test]
fn the_operators_restart_gives_a_down_orchestrator_a_fresh_restart() {
    let store = Store::new();
    let epic = orchestrated(&store);
    ensure(&store, epic, HOST).unwrap();
    let id = row_on(&store, epic, HOST).id;
    start(&store, id).unwrap();
    end(&store, id, true).unwrap();
    start(&store, id).unwrap();
    end(&store, id, true).unwrap();
    assert!(start(&store, id).is_err(), "down");

    store
        .call("alice", |ctx| restart_orchestrator(ctx, id, HOST.to_string()))
        .expect("restart");
    assert!(!row_on(&store, epic, HOST).restarted_once);
    start(&store, id).expect("starts again");
    assert!(row_on(&store, epic, HOST).restarted_once, "the death still counts");

    let why = store
        .call("alice", |ctx| restart_orchestrator(ctx, id, HOST.to_string()))
        .unwrap_err();
    assert!(why.contains("already running"), "{why}");
    let why = store
        .call("alice", |ctx| restart_orchestrator(ctx, id, OTHER.to_string()))
        .unwrap_err();
    assert!(why.contains("cannot start or end it"), "{why}");
}

#[test]
fn a_turn_for_an_unknown_name_is_refused() {
    let store = Store::new();
    orchestrated(&store);
    let why = turn(&store, "nobody").unwrap_err();
    assert!(why.contains("no orchestrator is named nobody"), "{why}");
}

#[test]
fn ending_a_session_that_is_not_running_is_refused() {
    let store = Store::new();
    let epic = orchestrated(&store);
    ensure(&store, epic, HOST).unwrap();
    let id = row_on(&store, epic, HOST).id;

    let why = end(&store, id, false).unwrap_err();
    assert!(why.contains("is not running"), "{why}");
}

#[test]
fn switching_off_removes_every_hosts_orchestrator() {
    let store = Store::new();
    let epic = orchestrated(&store);
    ensure(&store, epic, HOST).unwrap();
    ensure(&store, epic, OTHER).unwrap();

    store
        .call("alice", |ctx| switch_orchestrator_off(ctx, epic))
        .expect("switch off");

    assert!(rows(&store, epic).is_empty());
}

#[test]
fn deleting_the_epic_removes_its_orchestrators() {
    let store = Store::new();
    let epic = orchestrated(&store);
    ensure(&store, epic, HOST).unwrap();

    store.call("alice", |ctx| delete_epic(ctx, epic)).expect("delete");

    assert!(rows(&store, epic).is_empty());
}

#[test]
fn a_reparent_under_an_orchestrated_epic_removes_the_moved_epics_orchestrators() {
    let store = Store::new();
    let epic = orchestrated(&store);
    ensure(&store, epic, HOST).unwrap();
    let top = store.create_epic("alice", "Top");
    store
        .call("alice", |ctx| switch_orchestrator_on(ctx, top, None, None))
        .unwrap();

    store
        .call("alice", |ctx| {
            patch_epic(
                ctx,
                epic,
                EpicPatch {
                    parent_epic_id: Some(top),
                    ..EpicPatch::default()
                },
            )
        })
        .expect("reparent");

    assert!(rows(&store, epic).is_empty());
}
