//! The per-caller views over the in-memory store (`docs/architecture.md`: "Read
//! enforcement: per-caller views"). The views narrow by follows and ownership
//! only: delivery, not access (`docs/plans/2026-11-23-remove-workspaces.md`, R4).

use dispatch_spacetime_module as module;

use super::*;

fn epic(id: i64, parent: i64, creator: &str) -> module::Epic {
    module::Epic {
        id,
        parent_epic_id: parent,
        created_by: creator.into(),
        ..module::blank_epic()
    }
}

fn task(id: i64, epic_id: i64, creator: &str) -> module::Task {
    module::Task {
        id,
        epic_id,
        created_by: creator.into(),
        ..module::blank_task()
    }
}

/// `me` follows epic 1; epic 2 under it is `bob`'s, epic 3 under that is
/// `bob`'s too, epic 4 is `bob`'s child of 1, and epic 5 is a root of `bob`'s
/// that nobody follows.
fn scenario() -> MemoryReducerCaller {
    let (caller, _rows) = caller();
    let caller = caller.with_sender("me");
    {
        let mut t = caller.lock();
        for e in [
            epic(1, 0, "me"),
            epic(2, 1, "bob"),
            epic(3, 2, "bob"),
            epic(4, 1, "bob"),
            epic(5, 0, "bob"),
        ] {
            t.epics.insert(e.id, e);
        }
        for task in [task(10, 1, "me"), task(11, 2, "bob"), task(12, 5, "bob")] {
            t.tasks.insert(task.id, task);
        }
        t.subscriptions.insert(
            "me/1".into(),
            module::Subscription {
                id: "me/1".into(),
                epic_id: 1,
                subscriber: "me".into(),
            },
        );
    }
    caller
}

fn keys(caller: &MemoryReducerCaller, view: &str) -> Vec<String> {
    caller.view_keys(view).unwrap()
}

#[test]
fn the_views_serve_each_caller_what_they_follow_or_made() {
    let caller = scenario();
    assert_eq!(keys(&caller, "covered_epics"), ["1", "2", "3", "4"]);
    assert_eq!(keys(&caller, "covered_tasks"), ["10", "11"]);
    assert_eq!(keys(&caller, "own_epics"), ["1"]);
    assert_eq!(keys(&caller, "own_tasks"), ["10"]);
}

#[test]
fn another_person_receives_what_they_made_and_nothing_they_do_not_follow() {
    let caller = scenario().with_sender("bob");
    assert!(keys(&caller, "covered_epics").is_empty());
    assert_eq!(keys(&caller, "own_epics"), ["2", "3", "4", "5"]);
    assert_eq!(keys(&caller, "own_tasks"), ["11", "12"]);
}

#[test]
fn a_name_that_is_not_a_view_has_no_keys() {
    assert!(scenario().view_keys("epics").is_none());
}

/// M12: `me` dispatched bob's task 12 (epic 4, public) and then unfollowed
/// epic 1. The agent runs on `me`'s host, so the task, and the subagent rows
/// of it, stay served; with no agent or request there, they do not.
#[test]
fn a_task_running_on_the_callers_host_stays_served_after_an_unfollow() {
    let caller = scenario();
    {
        let mut t = caller.lock();
        let task12 = task(12, 4, "bob");
        t.tasks.insert(12, task12);
        t.subscriptions.clear();
        t.hosts.insert(
            "h-me".into(),
            module::Host {
                id: "h-me".into(),
                owner: "me".into(),
                label: String::new(),
            },
        );
        t.task_subagents.push(module::TaskSubagent {
            task_id: 12,
            agent_id: "sub".into(),
            session_id: "s".into(),
            started_at: String::new(),
            agent_row: 0,
        });
    }
    assert_eq!(keys(&caller, "own_tasks"), ["10"]);
    assert!(keys(&caller, "visible_task_subagents").is_empty());
    caller.lock().agents.insert(
        1,
        module::Agent {
            id: 1,
            task_id: 12,
            host: "h-me".into(),
            state: module::AGENT_CURRENT.into(),
            ..module::blank_agent()
        },
    );
    assert_eq!(keys(&caller, "own_tasks"), ["10", "12"]);
    assert_eq!(keys(&caller, "visible_task_subagents"), ["12/sub/s"]);
}
