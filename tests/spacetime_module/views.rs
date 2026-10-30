//! The per-caller views against a live store (`workspaces.allium`: "THE
//! PER-CALLER VIEWS"). The scenario and its expected rows are the ones
//! `src/sync/memory_caller/tests/views.rs` runs over the in-memory store, so
//! the module and its double are held to one answer.
//!
//! Run as the publisher identity, who holds a membership like anyone: views do
//! not exempt the operator.

use super::*;
use crate::common::spacetime_instance::column_values;

fn call_ok(instance: &Instance, reducer: &str, rows: serde_json::Value) {
    let out = instance.call(reducer, &[&rows.to_string()]);
    assert!(out.status.success(), "{reducer}: {}", describe(&out));
}

fn epic_in(
    id: i64,
    parent: i64,
    visibility: &str,
    creator: &str,
    workspace: i64,
) -> serde_json::Value {
    let mut e = epic_json(id, &format!("epic {id}"), "backlog", parent);
    e["created_by"] = serde_json::json!(creator);
    e["workspace_id"] = serde_json::json!(workspace);
    e["visibility"] = serde_json::json!(visibility);
    e
}

fn ids(instance: &Instance, view: &str) -> Vec<String> {
    column_values(instance, &format!("SELECT id FROM {view}"))
}

/// Me and `bob` in workspace 1. Epic 1 is public and followed by me; epic 2
/// under it is bob's private epic; epic 3 under 2 is public by itself; epic 4
/// is a public child of 1. Task 10 is in epic 1, task 11 in private epic 2.
fn seed_scenario(instance: &Instance, me: &str, bob: &str) {
    call_ok(
        instance,
        "seed_workspaces",
        serde_json::json!([{
            "id": 1, "name": "Core", "created_by": me, "created_at": "2026-10-30 10:00:00"
        }]),
    );
    call_ok(
        instance,
        "seed_memberships",
        serde_json::json!([
            {"id": 1, "workspace_id": 1, "user": me, "role": "member", "created_at": "2026-10-30 10:00:00"},
            {"id": 2, "workspace_id": 1, "user": bob, "role": "member", "created_at": "2026-10-30 10:00:00"},
        ]),
    );
    call_ok(
        instance,
        "seed_epics",
        serde_json::json!([
            epic_in(1, 0, "public", me, 1),
            epic_in(2, 1, "private", bob, 1),
            epic_in(3, 2, "public", bob, 1),
            epic_in(4, 1, "public", bob, 1),
        ]),
    );
    let mut visible = task_json(10, "visible", "backlog", 1, "");
    visible["created_by"] = serde_json::json!(me);
    let mut hidden = task_json(11, "hidden", "backlog", 2, "");
    hidden["created_by"] = serde_json::json!(bob);
    call_ok(instance, "seed_tasks", serde_json::json!([visible, hidden]));
    call_ok(
        instance,
        "seed_subscriptions",
        serde_json::json!([{"id": format!("{me}/1"), "epic_id": 1, "subscriber": me}]),
    );
}

#[test]
fn the_views_serve_each_caller_only_what_they_can_see() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let me = instance.identity();
    // The second identity exists once it has made a call.
    let named = instance.call_as("bob", "set_display_name", &["\"Bob\""]);
    assert!(named.status.success(), "{}", describe(&named));
    let bob = instance.identity_as("bob");
    seed_scenario(&instance, &me, &bob);

    // covered_epics walks down from epic 1 and stops at bob's private epic.
    assert_eq!(ids(&instance, "covered_epics"), ["1", "4"]);
    assert_eq!(ids(&instance, "covered_tasks"), ["10"]);
    // What I created, while I can still see it.
    assert_eq!(ids(&instance, "own_epics"), ["1"]);
    assert_eq!(ids(&instance, "own_tasks"), ["10"]);
    // Everything visible in my workspace: not the private epic or its subtree.
    assert_eq!(ids(&instance, "visible_epics"), ["1", "4"]);
    assert_eq!(ids(&instance, "visible_tasks"), ["10"]);
    assert_eq!(ids(&instance, "visible_workspaces"), ["1"]);
    assert_eq!(ids(&instance, "visible_memberships"), ["1", "2"]);
}
