use super::*;

/// `MigrateStoreToWorkspaces` over a store that already holds rows: one
/// default workspace, every epic public in it, a profile and a member row for
/// each known person.
#[tokio::test]
async fn migrating_places_existing_rows_in_the_default_workspace() {
    let (caller, rows) = caller();
    caller.create_epic(blank_epic()).await.unwrap();
    caller.create_epic(blank_epic()).await.unwrap();
    caller.migrate_store_to_workspaces();
    caller.migrate_store_to_workspaces(); // once

    let workspaces = rows.workspaces();
    assert_eq!(workspaces.len(), 1);
    assert_eq!(workspaces[0].name, "Default");
    let tables = caller.lock();
    assert!(tables
        .epics
        .values()
        .all(|e| e.workspace_id == workspaces[0].id && e.visibility == "public"));
    // `tester` created both epics; one member row, no admin.
    assert_eq!(tables.memberships.len(), 1);
    let member = tables.memberships.values().next().unwrap();
    assert_eq!(
        (member.user.as_str(), member.role.as_str()),
        ("tester", "member")
    );
    assert!(tables.user_profiles.values().all(|p| !p.is_super_admin));
}

#[tokio::test]
async fn deleting_a_workspace_that_holds_an_epic_is_refused() {
    let (caller, _rows) = caller();
    let epic = caller.create_epic(blank_epic()).await.unwrap();
    caller.register_user_profile();
    caller.migrate_store_to_workspaces();
    caller.apply_claim_super_admin().unwrap();
    let workspace = {
        let mut tables = caller.lock();
        let id = *tables.workspaces.keys().next().unwrap();
        tables.epics.get_mut(&epic.0).unwrap().workspace_id = id;
        id
    };
    let outcome = caller.apply_delete_workspace(workspace).unwrap();
    assert!(matches!(outcome, ReducerOutcome::Refused(_)), "{outcome:?}");
    assert!(caller.lock().workspaces.contains_key(&workspace));
}

#[tokio::test]
async fn removing_a_member_ends_their_subscriptions_in_that_workspace_only() {
    let (caller, rows) = caller();
    let here = caller.create_epic(blank_epic()).await.unwrap();
    let there = caller.create_epic(blank_epic()).await.unwrap();
    caller.register_user_profile();
    caller.migrate_store_to_workspaces();
    caller.apply_claim_super_admin().unwrap();
    caller.lock().epics.get_mut(&there.0).unwrap().workspace_id = 0;
    for epic in [here, there] {
        caller
            .subscribe_to_epic("tester".into(), epic)
            .await
            .unwrap();
    }
    let membership = caller.lock().memberships.values().next().unwrap().id;
    caller.apply_remove_member(membership).unwrap();
    assert_eq!(rows.subscribed_epics("tester"), vec![there]);
}
