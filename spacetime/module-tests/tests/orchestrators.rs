//! The orchestrator switch on an epic and the ready mark on a backlog task
//! (`core.allium: EpicOrchestration`, `ReadyMark`; `orchestrators.allium:
//! SwitchOrchestratorOn`, `ChangeOrchestratorSettings`,
//! `SwitchOrchestratorOff`; `epics.allium:
//! ReparentingKeepsOneOrchestratorOnAPath`, `DeleteEpic`; `tasks.allium:
//! MarkTaskReady`, `ClearReadyMark`, `LeavingBacklogDropsTheReadyMark`),
//! run against the in-memory store.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

fn on(store: &Store, epic: i64, rule: Option<&str>, max: Option<i64>) -> Result<(), String> {
    store.call("alice", |ctx| {
        switch_orchestrator_on(ctx, epic, rule.map(str::to_string), max)
    })
}

fn off(store: &Store, epic: i64) -> Result<(), String> {
    store.call("alice", |ctx| switch_orchestrator_off(ctx, epic))
}

fn orchestration(store: &Store, epic: i64) -> Option<EpicOrchestration> {
    store.db().epic_orchestrations().epic_id().find(epic)
}

fn mark(store: &Store, task: i64, by: &str, agent: &str) -> Result<(), String> {
    store.call("alice", |ctx| {
        mark_task_ready(ctx, task, by.to_string(), agent.to_string())
    })
}

fn ready(store: &Store, task: i64) -> Option<ReadyMark> {
    store.db().ready_marks().task_id().find(task)
}

fn reparent(store: &Store, epic: i64, to: i64) {
    store
        .call("alice", |ctx| {
            patch_epic(
                ctx,
                epic,
                EpicPatch {
                    parent_epic_id: Some(to),
                    ..EpicPatch::default()
                },
            )
        })
        .expect("reparent");
}

#[test]
fn switching_on_writes_the_switch_with_its_defaults() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E");

    on(&store, epic, None, None).expect("switch on");

    let row = orchestration(&store, epic).expect("the switch");
    assert_eq!(row.start_rule, "marked_only");
    assert_eq!(row.max_parallel, 0, "no limit of its own");
    assert!(!row.switched_on_at.is_empty());
}

#[test]
fn switching_on_takes_a_start_rule_and_a_limit() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E");

    on(&store, epic, Some("everything"), Some(5)).expect("switch on");

    let row = orchestration(&store, epic).expect("the switch");
    assert_eq!(row.start_rule, "everything");
    assert_eq!(row.max_parallel, 5);
}

#[test]
fn switching_on_refuses_a_bad_rule_a_bad_limit_and_a_missing_epic() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E");

    assert!(on(&store, epic, Some("sometimes"), None).is_err());
    assert_eq!(
        on(&store, epic, None, Some(0)).unwrap_err(),
        "the limit must be at least 1"
    );
    assert!(on(&store, 9_999, None, None).is_err());
    assert!(orchestration(&store, epic).is_none());
}

#[test]
fn one_switch_per_epic() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E");
    on(&store, epic, None, None).expect("switch on");

    assert_eq!(
        on(&store, epic, Some("everything"), None).unwrap_err(),
        format!("epic {epic} already has an orchestrator")
    );
    assert_eq!(
        orchestration(&store, epic).unwrap().start_rule,
        "marked_only"
    );
}

#[test]
fn one_orchestrator_on_a_path() {
    let store = Store::new();
    let root = store.create_epic("alice", "Root");
    let mid = store.create_sub_epic("alice", root);
    let leaf = store.create_sub_epic("alice", mid);
    on(&store, mid, None, None).expect("switch on the middle");

    assert_eq!(
        on(&store, leaf, None, None).unwrap_err(),
        format!("epic {mid} (sub of {root}) above this one has an orchestrator")
    );
    assert_eq!(
        on(&store, root, None, None).unwrap_err(),
        format!("epic {mid} (sub of {root}) below this one has an orchestrator")
    );
    assert!(orchestration(&store, root).is_none());
    assert!(orchestration(&store, leaf).is_none());
}

#[test]
fn changing_settings_changes_only_what_is_named() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E");
    on(&store, epic, None, Some(2)).expect("switch on");

    store
        .call("alice", |ctx| {
            change_orchestrator_settings(ctx, epic, Some("everything".into()), None)
        })
        .expect("change the rule");
    let row = orchestration(&store, epic).unwrap();
    assert_eq!(
        (row.start_rule.as_str(), row.max_parallel),
        ("everything", 2)
    );

    store
        .call("alice", |ctx| {
            change_orchestrator_settings(ctx, epic, None, Some(4))
        })
        .expect("change the limit");
    let row = orchestration(&store, epic).unwrap();
    assert_eq!(
        (row.start_rule.as_str(), row.max_parallel),
        ("everything", 4)
    );

    store
        .call("alice", |ctx| change_orchestrator_settings(ctx, epic, None, Some(0)))
        .expect("back to the default");
    let row = orchestration(&store, epic).unwrap();
    assert_eq!(row.max_parallel, 0);
    assert_eq!(orchestration_limit(&row), DEFAULT_ORCHESTRATOR_LIMIT);

    assert_eq!(
        store
            .call("alice", |ctx| change_orchestrator_settings(
                ctx,
                epic,
                None,
                Some(-1)
            ))
            .unwrap_err(),
        "the limit must be at least 1"
    );
}

#[test]
fn changing_or_switching_off_an_unorchestrated_epic_is_refused() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E");

    assert_eq!(
        store
            .call("alice", |ctx| change_orchestrator_settings(
                ctx,
                epic,
                None,
                Some(2)
            ))
            .unwrap_err(),
        format!("epic {epic} has no orchestrator")
    );
    assert_eq!(
        off(&store, epic).unwrap_err(),
        format!("epic {epic} has no orchestrator")
    );
}

#[test]
fn switching_off_removes_the_switch_and_keeps_the_marks() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E");
    let task = store.create("alice", epic_task("t", epic, "backlog"));
    on(&store, epic, None, None).expect("switch on");
    mark(&store, task, "operator", "").expect("mark");

    off(&store, epic).expect("switch off");

    assert!(orchestration(&store, epic).is_none());
    assert!(ready(&store, task).is_some(), "ready marks stay");
    on(&store, epic, None, None).expect("switch on again");
}

#[test]
fn reparenting_under_an_orchestrated_epic_switches_the_subtree_off() {
    let store = Store::new();
    let top = store.create_epic("alice", "Top");
    let under_top = store.create_sub_epic("alice", top);
    let moved = store.create_epic("alice", "Moved");
    let inside = store.create_sub_epic("alice", moved);
    on(&store, top, None, None).expect("switch on top");
    on(&store, inside, None, None).expect("switch on inside");

    reparent(&store, moved, under_top);

    assert!(
        orchestration(&store, inside).is_none(),
        "covered from above now"
    );
    assert!(orchestration(&store, top).is_some(), "the one above stays");
}

#[test]
fn reparenting_where_nothing_orchestrates_switches_nothing_off() {
    let store = Store::new();
    let plain = store.create_epic("alice", "Plain");
    let moved = store.create_epic("alice", "Moved");
    on(&store, moved, None, None).expect("switch on");

    reparent(&store, moved, plain);
    assert!(orchestration(&store, moved).is_some());

    reparent(&store, moved, 0);
    assert!(orchestration(&store, moved).is_some());
}

#[test]
fn deleting_an_epic_removes_every_switch_in_its_subtree() {
    let store = Store::new();
    let root = store.create_epic("alice", "Root");
    let sub = store.create_sub_epic("alice", root);
    let other = store.create_epic("alice", "Other");
    on(&store, sub, None, None).expect("switch on sub");
    on(&store, other, None, None).expect("switch on other");

    store
        .call("alice", |ctx| delete_epic(ctx, root))
        .expect("delete");

    assert!(orchestration(&store, sub).is_none());
    assert!(orchestration(&store, other).is_some());
}

#[test]
fn marking_ready_records_the_first_marker() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E");
    let task = store.create("alice", epic_task("t", epic, "backlog"));

    mark(&store, task, "agent", "folke3").expect("mark");
    mark(&store, task, "operator", "").expect("mark again is no error");

    let row = ready(&store, task).expect("ready");
    assert_eq!(row.marked_by, "agent");
    assert_eq!(row.agent_name, "folke3");
    assert!(!row.marked_at.is_empty());
}

#[test]
fn only_a_backlog_task_can_be_marked_ready() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E");
    let task = store.create("alice", epic_task("t", epic, "review"));

    assert_eq!(
        mark(&store, task, "operator", "").unwrap_err(),
        "only a backlog task can be marked ready"
    );
    assert_eq!(
        mark(&store, 9_999, "operator", "").unwrap_err(),
        "task 9999 not found"
    );
    assert!(ready(&store, task).is_none());
}

#[test]
fn the_marker_and_the_agent_name_go_together() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E");
    let task = store.create("alice", epic_task("t", epic, "backlog"));

    assert_eq!(
        mark(&store, task, "agent", "").unwrap_err(),
        "an agent's ready mark names the agent"
    );
    assert_eq!(
        mark(&store, task, "operator", "folke3").unwrap_err(),
        "the operator's ready mark names no agent"
    );
    assert!(mark(&store, task, "someone", "").is_err());
    assert!(ready(&store, task).is_none());
}

#[test]
fn clearing_is_idempotent() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E");
    let task = store.create("alice", epic_task("t", epic, "backlog"));
    mark(&store, task, "operator", "").expect("mark");

    store
        .call("alice", |ctx| clear_ready_mark(ctx, task))
        .expect("clear");
    store
        .call("alice", |ctx| clear_ready_mark(ctx, task))
        .expect("clear again");
    assert!(ready(&store, task).is_none());
}

#[test]
fn leaving_backlog_drops_the_mark_and_coming_back_does_not_restore_it() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E");
    let task = store.create("alice", epic_task("t", epic, "backlog"));
    let untouched = store.create("alice", epic_task("u", epic, "backlog"));
    mark(&store, task, "operator", "").expect("mark");
    mark(&store, untouched, "operator", "").expect("mark");

    store
        .call("alice", |ctx| {
            patch_task(
                ctx,
                task,
                TaskPatch {
                    title: Some("renamed".into()),
                    ..TaskPatch::default()
                },
            )
        })
        .expect("a patch that stays in backlog");
    assert!(ready(&store, task).is_some(), "staying in backlog keeps it");

    store
        .call("alice", |ctx| patch_task(ctx, task, status_patch("review")))
        .expect("move to review");
    assert!(ready(&store, task).is_none());

    store
        .call("alice", |ctx| {
            patch_task(ctx, task, status_patch("backlog"))
        })
        .expect("move back");
    assert!(
        ready(&store, task).is_none(),
        "not ready until marked again"
    );
    assert!(ready(&store, untouched).is_some());
}

#[test]
fn a_feed_dropping_a_ready_task_takes_its_mark_with_it() {
    let store = Store::new();
    let row = Epic {
        title: "Feed".into(),
        feed_command: "echo []".into(),
        ..blank_epic()
    };
    store
        .call("alice", |ctx| create_epic(ctx, row))
        .expect("create_epic");
    let epic = store.db().epics().iter().map(|e| e.id).max().unwrap();
    store.register_host_for(HOST_A, "alice");
    let ingest = |items: Vec<FeedIngestItem>| {
        store
            .call("alice", |ctx| {
                ingest_feed(ctx, epic, items, false, "creator".into(), HOST_A.into())
            })
            .expect("ingest_feed");
    };
    ingest(vec![FeedIngestItem {
        item: FeedTaskUpsertItem {
            external_id: "pr-1".into(),
            title: "pr 1".into(),
            description: String::new(),
            repo_path: "/repo".into(),
            status: "backlog".into(),
            sub_status: "none".into(),
            base_branch: "main".into(),
            tag: "chore".into(),
            labels: "[]".into(),
            sort_order: None,
            url: String::new(),
            url_type: String::new(),
            wrap_up_mode: String::new(),
        },
        signals: vec![],
    }]);
    let task = store.max_task_id();
    mark(&store, task, "operator", "").expect("mark");

    ingest(vec![]);

    assert!(store.db().tasks().id().find(task).is_none(), "the stale task is gone");
    assert!(ready(&store, task).is_none());
}
