//! An orchestrated epic's memory: its log, its notes and the items that wait
//! for the operator (`core.allium: OrchestratorLogEntry`, `EpicNote`,
//! `WaitingItem`; `orchestrators.allium: OrchestratorStartsATask`,
//! `OrchestratorAnswersAnAgent`, `OrchestratorPassesOnToTheOperator`,
//! `WaitingItemResolved`, `FinishedTaskResolvesItsItems`,
//! `DeletedTaskResolvesItsItems`, `OrchestratorRecordsItsSummary`,
//! `OrchestratorThatKeepsDyingWaitsForTheOperator`, `WriteEpicNote`,
//! `ChangeEpicNote`, `RemoveEpicNote`, `SwitchOrchestratorOff`,
//! `DeletingAnEpicRemovesItsOrchestratorMemory`, the session entries),
//! run against the in-memory store.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

const HOST: &str = "host-a";

/// An orchestrated epic with one backlog task and its orchestrator on HOST:
/// (epic, task, orchestrator).
fn setup(store: &Store) -> (i64, i64, Orchestrator) {
    store.register_host_for(HOST, "alice");
    let epic = store.create_epic("alice", "E", false);
    let task = store.create("alice", epic_task("t", epic, "backlog"));
    store
        .call("alice", |ctx| switch_orchestrator_on(ctx, epic, None, None))
        .expect("switch on");
    store
        .call("alice", |ctx| ensure_orchestrator(ctx, epic, HOST.to_string()))
        .expect("ensure");
    let o = store
        .db()
        .orchestrators()
        .epic_id()
        .filter(epic)
        .next()
        .expect("the orchestrator");
    (epic, task, o)
}

fn log(store: &Store, epic: i64) -> Vec<OrchestratorLogEntry> {
    let mut all: Vec<_> = store.db().orchestrator_log().epic_id().filter(epic).collect();
    all.sort_by_key(|e| e.id);
    all
}

fn items(store: &Store, epic: i64) -> Vec<WaitingItem> {
    store.db().waiting_items().epic_id().filter(epic).collect()
}

fn notes(store: &Store, epic: i64) -> Vec<EpicNote> {
    store.db().epic_notes().epic_id().filter(epic).collect()
}

fn raise(store: &Store, name: &str, task: i64) -> Result<(), String> {
    store.call("alice", |ctx| {
        raise_waiting_item(
            ctx,
            name.into(),
            task,
            "may I drop the column?".into(),
            "read the migration".into(),
            "keep it one more release".into(),
        )
    })
}

fn done(store: &Store, task: i64) {
    store
        .call("alice", |ctx| patch_task(ctx, task, status_patch("done")))
        .expect("done");
}

#[test]
fn an_orchestrators_start_writes_its_reason_in_the_same_write() {
    let store = Store::new();
    let (epic, task, o) = setup(&store);

    store
        .call("alice", |ctx| {
            orchestrator_request_dispatch(ctx, task, HOST.into(), o.name.clone(), "first step".into())
        })
        .expect("start");

    let entries = log(&store, epic);
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].kind, LOG_STARTED_TASK);
    assert_eq!(entries[0].task_id, task);
    assert_eq!(entries[0].text, "first step");
    assert_eq!(entries[0].host, HOST);
    assert!(!entries[0].at.is_empty());
    assert_eq!(store.task(task).status, "running");
}

#[test]
fn a_refused_start_leaves_no_entry() {
    let store = Store::new();
    let (epic, task, o) = setup(&store);
    done(&store, task);

    let why = store
        .call("alice", |ctx| {
            orchestrator_request_dispatch(ctx, task, HOST.into(), o.name.clone(), "again".into())
        })
        .unwrap_err();

    assert!(!why.is_empty());
    assert!(log(&store, epic).is_empty());
}

#[test]
fn a_start_needs_a_reason_and_a_known_orchestrator() {
    let store = Store::new();
    let (_, task, o) = setup(&store);

    let blank = store
        .call("alice", |ctx| {
            orchestrator_request_dispatch(ctx, task, HOST.into(), o.name.clone(), " ".into())
        })
        .unwrap_err();
    assert!(blank.contains("the reason must not be empty"), "{blank}");

    let unknown = store
        .call("alice", |ctx| {
            orchestrator_request_dispatch(ctx, task, HOST.into(), "nobody".into(), "r".into())
        })
        .unwrap_err();
    assert!(unknown.contains("no orchestrator is named nobody"), "{unknown}");
    assert_eq!(store.task(task).status, "backlog");
}

#[test]
fn an_answer_is_logged_on_its_task() {
    let store = Store::new();
    let (epic, task, o) = setup(&store);

    store
        .call("alice", |ctx| {
            record_orchestrator_answer(
                ctx,
                o.name.clone(),
                task,
                "rebase or pr?".into(),
                "rebase".into(),
                "the epic says rebase".into(),
            )
        })
        .expect("answer");

    let entry = log(&store, epic).pop().expect("an entry");
    assert_eq!(entry.kind, LOG_ANSWERED);
    assert_eq!(entry.task_id, task);
    assert_eq!(entry.question, "rebase or pr?");
    assert_eq!(entry.text, "rebase — the epic says rebase");
}

#[test]
fn an_answer_about_a_task_outside_the_epic_is_refused() {
    let store = Store::new();
    let (epic, _, o) = setup(&store);
    let other_epic = store.create_epic("alice", "Other", false);
    let outside = store.create("alice", epic_task("x", other_epic, "backlog"));

    let why = store
        .call("alice", |ctx| {
            record_orchestrator_answer(ctx, o.name.clone(), outside, "q".into(), "a".into(), "r".into())
        })
        .unwrap_err();

    assert!(why.contains(&format!("task {outside} is not in epic {epic}")), "{why}");
}

#[test]
fn a_raised_item_writes_its_passed_on_entry() {
    let store = Store::new();
    let (epic, task, o) = setup(&store);

    raise(&store, &o.name, task).expect("raise");

    let item = items(&store, epic).pop().expect("an item");
    assert_eq!(item.task_id, task);
    assert_eq!(item.host, HOST);
    assert!(item.resolved_at.is_empty() && !item.raised_at.is_empty());
    let entry = log(&store, epic).pop().expect("an entry");
    assert_eq!(entry.kind, LOG_PASSED_ON);
    assert_eq!(entry.question, "may I drop the column?");
    assert_eq!(entry.text, "keep it one more release");
}

#[test]
fn resolving_an_item_is_idempotent() {
    let store = Store::new();
    let (epic, task, o) = setup(&store);
    raise(&store, &o.name, task).expect("raise");
    let id = items(&store, epic)[0].id;

    store
        .call("alice", |ctx| resolve_waiting_item(ctx, id))
        .expect("resolve");
    let first = items(&store, epic)[0].resolved_at.clone();
    assert!(!first.is_empty());
    store.advance_secs(5);
    store
        .call("alice", |ctx| resolve_waiting_item(ctx, id))
        .expect("resolve again");

    assert_eq!(items(&store, epic)[0].resolved_at, first);
}

#[test]
fn a_finished_task_resolves_its_items() {
    let store = Store::new();
    let (epic, task, o) = setup(&store);
    raise(&store, &o.name, task).expect("raise");

    done(&store, task);

    assert!(!items(&store, epic)[0].resolved_at.is_empty());
}

#[test]
fn a_deleted_task_resolves_its_items() {
    let store = Store::new();
    let (epic, task, o) = setup(&store);
    raise(&store, &o.name, task).expect("raise");
    let item = items(&store, epic)[0].id;
    // Resolved by the done move already; reopen the bout with a second task.
    let second = store.create("alice", epic_task("t2", epic, "backlog"));
    raise(&store, &o.name, second).expect("raise");
    done(&store, second);
    store
        .call("alice", |ctx| delete_task(ctx, second))
        .expect("delete");

    assert!(items(&store, epic)
        .iter()
        .filter(|i| i.id != item)
        .all(|i| !i.resolved_at.is_empty()));
}

#[test]
fn switching_off_resolves_the_epics_items_and_keeps_its_memory() {
    let store = Store::new();
    let (epic, task, o) = setup(&store);
    raise(&store, &o.name, task).expect("raise");
    store
        .call("alice", |ctx| write_epic_note(ctx, o.name.clone(), "v3 next".into()))
        .expect("note");

    store
        .call("alice", |ctx| switch_orchestrator_off(ctx, epic))
        .expect("off");

    assert!(items(&store, epic).iter().all(|i| !i.resolved_at.is_empty()));
    assert_eq!(notes(&store, epic).len(), 1);
    assert!(!log(&store, epic).is_empty());
}

#[test]
fn deleting_the_epic_removes_its_memory() {
    let store = Store::new();
    let (epic, task, o) = setup(&store);
    raise(&store, &o.name, task).expect("raise");
    store
        .call("alice", |ctx| write_epic_note(ctx, o.name.clone(), "v3 next".into()))
        .expect("note");
    done(&store, task);

    store
        .call("alice", |ctx| delete_epic(ctx, epic))
        .expect("delete epic");

    assert!(items(&store, epic).is_empty());
    assert!(notes(&store, epic).is_empty());
    assert!(log(&store, epic).is_empty());
}

#[test]
fn notes_are_written_changed_and_removed_by_their_orchestrator() {
    let store = Store::new();
    let (epic, _, o) = setup(&store);

    store
        .call("alice", |ctx| write_epic_note(ctx, o.name.clone(), "use v3".into()))
        .expect("write");
    let note = notes(&store, epic).pop().expect("a note");
    assert!(!note.done && !note.written_at.is_empty());

    store
        .call("alice", |ctx| {
            change_epic_note(ctx, o.name.clone(), note.id, Some("use v4".into()), Some(true))
        })
        .expect("change");
    let changed = notes(&store, epic).pop().expect("the note");
    assert_eq!((changed.text.as_str(), changed.done), ("use v4", true));

    let nothing = store
        .call("alice", |ctx| change_epic_note(ctx, o.name.clone(), note.id, None, None))
        .unwrap_err();
    assert!(nothing.contains("needs its text, its done tick, or both"), "{nothing}");

    store
        .call("alice", |ctx| remove_epic_note(ctx, o.name.clone(), note.id))
        .expect("remove");
    assert!(notes(&store, epic).is_empty());
}

#[test]
fn another_epics_orchestrator_cannot_touch_a_note() {
    let store = Store::new();
    let (_, _, o) = setup(&store);
    let other = store.create_epic("alice", "Other", false);
    store.create("alice", epic_task("y", other, "backlog"));
    store
        .call("alice", |ctx| switch_orchestrator_on(ctx, other, None, None))
        .expect("switch on");
    store
        .call("alice", |ctx| ensure_orchestrator(ctx, other, HOST.to_string()))
        .expect("ensure");
    let theirs = store.db().orchestrators().epic_id().filter(other).next().unwrap();
    store
        .call("alice", |ctx| write_epic_note(ctx, o.name.clone(), "mine".into()))
        .expect("write");
    let note = store.db().epic_notes().iter().next().unwrap().id;

    let why = store
        .call("alice", |ctx| remove_epic_note(ctx, theirs.name.clone(), note))
        .unwrap_err();

    assert!(why.contains(&format!("note {note} belongs to epic")), "{why}");
}

#[test]
fn the_summary_waits_for_a_done_epic() {
    let store = Store::new();
    let (epic, task, o) = setup(&store);

    let early = store
        .call("alice", |ctx| record_orchestrator_summary(ctx, o.name.clone(), "all done".into()))
        .unwrap_err();
    assert!(early.contains("is not done"), "{early}");

    done(&store, task);
    store
        .call("alice", |ctx| record_orchestrator_summary(ctx, o.name.clone(), "all done".into()))
        .expect("summary");

    let entry = log(&store, epic).pop().expect("an entry");
    assert_eq!((entry.kind.as_str(), entry.text.as_str()), (LOG_SUMMARY, "all done"));
}

#[test]
fn the_session_writes_its_start_and_end() {
    let store = Store::new();
    let (epic, _, o) = setup(&store);

    store
        .call("alice", |ctx| start_orchestrator_session(ctx, o.id, HOST.into(), false))
        .expect("start");
    store
        .call("alice", |ctx| end_orchestrator_session(ctx, o.id, HOST.into(), true))
        .expect("died");
    store
        .call("alice", |ctx| start_orchestrator_session(ctx, o.id, HOST.into(), true))
        .expect("restart");
    store
        .call("alice", |ctx| end_orchestrator_session(ctx, o.id, HOST.into(), false))
        .expect("end");

    let texts: Vec<_> = log(&store, epic)
        .into_iter()
        .map(|e| (e.kind, e.text))
        .collect();
    let session = |t: &str| (LOG_SESSION.to_string(), t.to_string());
    assert_eq!(
        texts,
        vec![
            session(SESSION_STARTED),
            session(SESSION_DIED),
            session(SESSION_RESUMED),
            session(SESSION_ENDED)
        ]
    );
}

#[test]
fn an_orchestrator_that_keeps_dying_raises_one_item() {
    let store = Store::new();
    let (epic, _, o) = setup(&store);
    let down = |store: &Store| {
        store
            .call("alice", |ctx| raise_orchestrator_down_item(ctx, o.id, HOST.into()))
            .expect("raise down item")
    };

    down(&store);
    assert!(items(&store, epic).is_empty(), "not down yet");

    for _ in 0..2 {
        store
            .call("alice", |ctx| start_orchestrator_session(ctx, o.id, HOST.into(), false))
            .expect("start");
        store
            .call("alice", |ctx| end_orchestrator_session(ctx, o.id, HOST.into(), true))
            .expect("died");
    }
    down(&store);
    down(&store);

    let raised = items(&store, epic);
    assert_eq!(raised.len(), 1, "{raised:?}");
    assert_eq!(raised[0].task_id, 0);
    assert_eq!(raised[0].about, DOWN_ABOUT);
}

#[test]
fn only_an_orchestrator_writes_its_memory() {
    let store = Store::new();
    let (_, task, _) = setup(&store);

    let why = raise(&store, "plain-session", task).unwrap_err();
    assert!(why.contains("no orchestrator is named plain-session"), "{why}");
    let why = store
        .call("alice", |ctx| write_epic_note(ctx, "plain-session".into(), "x".into()))
        .unwrap_err();
    assert!(why.contains("no orchestrator is named plain-session"), "{why}");
}
