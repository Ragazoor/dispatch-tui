use super::*;

fn epic(id: i64, parent: i64, visibility: &str, creator: &str, workspace: i64) -> Epic {
    Epic {
        id,
        parent_epic_id: parent,
        visibility: visibility.to_string(),
        created_by: creator.to_string(),
        workspace_id: workspace,
        created_at: "2026-10-30T00:00:00Z".to_string(),
        ..blank_epic()
    }
}

fn package(id: i64, visibility: &str, creator: &str, workspace: i64) -> WorkPackage {
    WorkPackage {
        id,
        visibility: visibility.to_string(),
        created_by: creator.to_string(),
        workspace_id: workspace,
        created_at: "2026-10-30T00:00:00Z".to_string(),
        ..blank_work_package()
    }
}

/// Run the listing over `epics` (all of them; the private ones are what the
/// view would read through its index) and `packages`, with `members` as
/// (workspace, user) pairs.
fn listing(epics: &[Epic], packages: &[WorkPackage], members: &[(i64, &str)]) -> Vec<UnseenItem> {
    let private_epics: Vec<Epic> = epics
        .iter()
        .filter(|e| e.visibility == VISIBILITY_PRIVATE)
        .cloned()
        .collect();
    let private_packages: Vec<WorkPackage> = packages
        .iter()
        .filter(|p| p.visibility == VISIBILITY_PRIVATE)
        .cloned()
        .collect();
    unseen_items_of(
        &private_epics,
        &private_packages,
        |ws, user| members.contains(&(ws, user)),
    )
}

#[test]
fn a_private_epic_whose_creator_left_is_listed_without_a_title() {
    let rows = listing(&[epic(1, 0, "private", "ann", 7)], &[], &[(7, "bob")]);
    assert_eq!(
        rows,
        vec![UnseenItem {
            kind: "epic".into(),
            item_id: 1,
            workspace_id: 7,
            creator: "ann".into(),
            created_at: "2026-10-30T00:00:00Z".into(),
        }]
    );
}

#[test]
fn a_private_epic_whose_creator_is_still_a_member_is_not_listed() {
    assert!(listing(&[epic(1, 0, "private", "ann", 7)], &[], &[(7, "ann")]).is_empty());
}

#[test]
fn a_public_item_is_never_listed() {
    let rows = listing(
        &[epic(1, 0, "public", "ann", 7)],
        &[package(2, "public", "ann", 7)],
        &[],
    );
    assert!(rows.is_empty());
}

#[test]
fn every_private_epic_whose_creator_left_is_listed_even_under_a_listed_one() {
    // 1 (private) -> 2 (public) -> 3 (private), all ann's, who left: 1 and 3
    // are each listed (A5); 2 is public and goes with 1's subtree.
    let epics = [
        epic(1, 0, "private", "ann", 7),
        epic(2, 1, "public", "ann", 7),
        epic(3, 2, "private", "ann", 7),
    ];
    let ids: Vec<i64> = listing(&epics, &[], &[])
        .iter()
        .map(|r| r.item_id)
        .collect();
    assert_eq!(ids, vec![1, 3]);
}

#[test]
fn a_private_sub_epic_whose_creator_left_is_listed_while_its_ancestors_creator_stays() {
    // 1 (private, bob's, bob stays) -> 2 (private, ann's, ann left). The
    // delete accepts 2 (its holder is ann), so the listing shows it.
    let epics = [
        epic(1, 0, "private", "bob", 7),
        epic(2, 1, "private", "ann", 7),
    ];
    let ids: Vec<i64> = listing(&epics, &[], &[(7, "bob")])
        .iter()
        .map(|r| r.item_id)
        .collect();
    assert_eq!(ids, vec![2]);
}

#[test]
fn a_private_sub_epic_of_a_public_epic_is_listed_on_its_own() {
    let epics = [
        epic(1, 0, "public", "ann", 7),
        epic(2, 1, "private", "ann", 7),
    ];
    let ids: Vec<i64> = listing(&epics, &[], &[])
        .iter()
        .map(|r| r.item_id)
        .collect();
    assert_eq!(ids, vec![2]);
}

#[test]
fn a_private_item_with_no_creator_is_listed() {
    let rows = listing(&[epic(1, 0, "private", "", 7)], &[], &[(7, "ann")]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].creator, "");
}

#[test]
fn packages_and_epics_are_ordered_by_workspace_then_kind_then_id() {
    let rows = listing(
        &[
            epic(5, 0, "private", "ann", 2),
            epic(1, 0, "private", "ann", 9),
        ],
        &[package(3, "private", "ann", 2)],
        &[],
    );
    let keys: Vec<(i64, &str, i64)> = rows
        .iter()
        .map(|r| (r.workspace_id, r.kind.as_str(), r.item_id))
        .collect();
    assert_eq!(
        keys,
        vec![(2, "epic", 5), (2, "work_package", 3), (9, "epic", 1)]
    );
}
