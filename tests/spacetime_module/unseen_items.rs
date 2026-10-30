//! The `unseen_items` view against a live store
//! (`admin-surfaces.allium: UnseenItemListingContents`): a super-admin's
//! listing holds a private epic whose creator is no longer a member, an
//! ordinary caller's is empty, and no row carries a title.

use super::*;
use crate::common::spacetime_instance::no_rows;

fn call_ok(instance: &Instance, reducer: &str, rows: serde_json::Value) {
    let out = instance.call(reducer, &[&rows.to_string()]);
    assert!(out.status.success(), "{reducer}: {}", describe(&out));
}

fn profile(identity: &str, super_admin: bool) -> serde_json::Value {
    serde_json::json!({
        "identity": identity,
        "display_name": "",
        "is_super_admin": super_admin,
        "created_at": "2026-10-30 10:00:00",
    })
}

#[test]
fn the_unseen_items_view_lists_private_epics_nobody_can_see_for_a_super_admin_only() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let me = instance.identity();

    call_ok(
        &instance,
        "seed_workspaces",
        serde_json::json!([{
            "id": 1, "name": "Core", "created_by": me, "created_at": "2026-10-30 10:00:00"
        }]),
    );
    // `gone` created a private epic in workspace 1 and is not a member of it.
    let mut hidden = epic_json(5, "Secret title", "backlog", 0);
    hidden["created_by"] = serde_json::json!("gone");
    hidden["workspace_id"] = serde_json::json!(1);
    hidden["visibility"] = serde_json::json!("private");
    // A private epic whose creator is still a member is not unseen.
    let mut kept = epic_json(6, "Kept title", "backlog", 0);
    kept["created_by"] = serde_json::json!("here");
    kept["workspace_id"] = serde_json::json!(1);
    kept["visibility"] = serde_json::json!("private");
    call_ok(&instance, "seed_epics", serde_json::json!([hidden, kept]));
    call_ok(
        &instance,
        "seed_memberships",
        serde_json::json!([{
            "id": 1, "workspace_id": 1, "user": "here", "role": "member",
            "created_at": "2026-10-30 10:00:00"
        }]),
    );

    // Not a super-admin yet: the view is empty.
    call_ok(
        &instance,
        "seed_user_profiles",
        serde_json::json!([profile(&me, false)]),
    );
    assert!(no_rows(&instance, "SELECT * FROM unseen_items"));

    // A super-admin: the one unseen epic, by id, and no title anywhere.
    call_ok(
        &instance,
        "seed_user_profiles",
        serde_json::json!([profile(&me, true)]),
    );
    // `column` asserts exactly one row: epic 6 is not listed.
    assert_eq!(column(&instance, "SELECT item_id FROM unseen_items"), "5");
    assert_eq!(column(&instance, "SELECT kind FROM unseen_items"), "epic");
    assert_eq!(
        column(&instance, "SELECT creator FROM unseen_items"),
        "gone"
    );
    let everything = instance.sql("SELECT * FROM unseen_items");
    assert!(
        !everything.contains("Secret title") && !everything.contains("Kept title"),
        "a row carries a title: {everything}"
    );
}
