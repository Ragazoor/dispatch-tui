//! The per-caller views against a live store (`docs/architecture.md`: "Read
//! enforcement: per-caller views"). The scenario and its expected rows are the
//! ones `src/sync/memory_caller/tests/views.rs` runs over the in-memory store,
//! so the module and its double are held to one answer. The views narrow by
//! follows and ownership only: delivery, not access
//! (`docs/plans/2026-11-23-remove-workspaces.md`, R4).
//!
//! Run as the publisher identity: views do not exempt the operator.

use super::*;
use crate::common::spacetime_instance::column_values;

fn epic_by(id: i64, parent: i64, creator: &str) -> serde_json::Value {
    let mut e = epic_json(id, &format!("epic {id}"), "backlog", parent);
    e["created_by"] = serde_json::json!(creator);
    e
}

fn ids(instance: &Instance, view: &str) -> Vec<String> {
    column_values(instance, &format!("SELECT id FROM {view}"))
}

/// I follow epic 1, which is mine; epic 2 under it is `bob`'s, epic 3 under
/// that is `bob`'s too, epic 4 is `bob`'s child of 1, and epic 5 is a root of
/// `bob`'s that nobody follows. Task 10 ("visible") is mine in epic 1, task 11
/// ("hidden") `bob`'s in epic 2, task 12 ("elsewhere") `bob`'s in epic 5.
pub(super) fn seed_scenario(instance: &Instance, me: &str, bob: &str) {
    call_ok(
        instance,
        "seed_epics",
        serde_json::json!([
            epic_by(1, 0, me),
            epic_by(2, 1, bob),
            epic_by(3, 2, bob),
            epic_by(4, 1, bob),
            epic_by(5, 0, bob),
        ]),
    );
    let mut visible = task_json(10, "visible", "backlog", 1, "");
    visible["created_by"] = serde_json::json!(me);
    let mut hidden = task_json(11, "hidden", "backlog", 2, "");
    hidden["created_by"] = serde_json::json!(bob);
    let mut elsewhere = task_json(12, "elsewhere", "backlog", 5, "");
    elsewhere["created_by"] = serde_json::json!(bob);
    call_ok(
        instance,
        "seed_tasks",
        serde_json::json!([visible, hidden, elsewhere]),
    );
    call_ok(
        instance,
        "seed_subscriptions",
        serde_json::json!([{"id": format!("{me}/1"), "epic_id": 1, "subscriber": me}]),
    );
}

#[test]
fn the_views_serve_each_caller_what_they_follow_or_made() {
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

    // covered_epics walks down from epic 1 at any depth, whoever made them.
    assert_eq!(ids(&instance, "covered_epics"), ["1", "2", "3", "4"]);
    assert_eq!(ids(&instance, "covered_tasks"), ["10", "11"]);
    // What I created.
    assert_eq!(ids(&instance, "own_epics"), ["1"]);
    assert_eq!(ids(&instance, "own_tasks"), ["10"]);
}
