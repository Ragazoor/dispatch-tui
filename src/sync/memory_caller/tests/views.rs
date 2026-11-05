//! The per-caller views over the in-memory store (`workspaces.allium`: "THE
//! PER-CALLER VIEWS"). The scenario and the expected rows are the ones
//! `tests/spacetime_module/views.rs` checks against a live store.

use dispatch_spacetime_module as module;

use super::*;

fn epic(id: i64, parent: i64, visibility: &str, creator: &str) -> module::Epic {
    module::Epic {
        id,
        parent_epic_id: parent,
        visibility: visibility.into(),
        created_by: creator.into(),
        workspace_id: 1,
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

fn member(id: i64, user: &str) -> module::Membership {
    module::Membership {
        id,
        workspace_id: 1,
        user: user.into(),
        role: "member".into(),
        ..module::blank_membership()
    }
}

/// `me` follows public epic 1; epic 2 under it is `bob`'s private epic, epic 3
/// under that is public by itself, epic 4 is a public child of 1.
fn scenario() -> MemoryReducerCaller {
    let (caller, _rows) = caller();
    let caller = caller.with_sender("me");
    {
        let mut t = caller.lock();
        t.workspaces.insert(
            1,
            module::Workspace {
                id: 1,
                name: "Core".into(),
                ..module::blank_workspace()
            },
        );
        t.memberships.insert(1, member(1, "me"));
        t.memberships.insert(2, member(2, "bob"));
        for e in [
            epic(1, 0, "public", "me"),
            epic(2, 1, "private", "bob"),
            epic(3, 2, "public", "bob"),
            epic(4, 1, "public", "bob"),
        ] {
            t.epics.insert(e.id, e);
        }
        for task in [task(10, 1, "me"), task(11, 2, "bob")] {
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
fn the_views_serve_each_caller_only_what_they_can_see() {
    let caller = scenario();
    assert_eq!(keys(&caller, "covered_epics"), ["1", "4"]);
    assert_eq!(keys(&caller, "covered_tasks"), ["10"]);
    assert_eq!(keys(&caller, "own_epics"), ["1"]);
    assert_eq!(keys(&caller, "own_tasks"), ["10"]);
    assert_eq!(keys(&caller, "visible_epics"), ["1", "4"]);
    assert_eq!(keys(&caller, "visible_tasks"), ["10"]);
    assert_eq!(keys(&caller, "visible_workspaces"), ["1"]);
    assert_eq!(keys(&caller, "visible_memberships"), ["1", "2"]);
}

#[test]
fn the_other_member_sees_the_private_epic_and_its_subtree() {
    let caller = scenario().with_sender("bob");
    caller.lock().subscriptions.insert(
        "bob/1".into(),
        module::Subscription {
            id: "bob/1".into(),
            epic_id: 1,
            subscriber: "bob".into(),
        },
    );
    assert_eq!(keys(&caller, "covered_epics"), ["1", "2", "3", "4"]);
    assert_eq!(keys(&caller, "covered_tasks"), ["10", "11"]);
    assert_eq!(keys(&caller, "own_epics"), ["2", "3", "4"]);
}

#[test]
fn a_super_admin_sees_every_workspace_without_belonging_to_one() {
    let caller = scenario().with_sender("carol");
    assert!(keys(&caller, "visible_workspaces").is_empty());
    assert!(keys(&caller, "visible_memberships").is_empty());

    caller.lock().user_profiles.insert(
        "carol".into(),
        module::UserProfile {
            identity: "carol".into(),
            display_name: String::new(),
            is_super_admin: true,
            created_at: String::new(),
        },
    );
    assert_eq!(keys(&caller, "visible_workspaces"), ["1"]);
    assert_eq!(keys(&caller, "visible_memberships"), ["1", "2"]);
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
