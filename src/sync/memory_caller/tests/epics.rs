use super::*;

// -- Epics ------------------------------------------------------------------

#[tokio::test]
async fn create_epic_generates_sequential_ids() {
    let (caller, rows) = caller();
    let id1 = caller.create_epic(blank_epic()).await.unwrap();
    let id2 = caller.create_epic(blank_epic()).await.unwrap();
    assert_eq!(id1, EpicId(1));
    assert_eq!(id2, EpicId(2));
    assert_eq!(rows.epics().len(), 2);
}

#[tokio::test]
async fn epic_flips_to_done_when_every_child_task_is_done_and_back_when_one_regresses() {
    let (caller, rows) = caller();
    let epic_id = caller.create_epic(blank_epic()).await.unwrap();
    let task_id = caller
        .create_task(bindings::Task {
            epic_id: epic_id.0,
            owner: String::new(),
            ..blank_task()
        })
        .await
        .unwrap();

    caller
        .patch_task(
            task_id,
            bindings::TaskPatch {
                status: Some("done".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();
    let epic = rows.epic(epic_id).unwrap();
    assert_eq!(epic.status, crate::models::TaskStatus::Done);
    let completed_at = epic.completed_at;
    assert!(completed_at.is_some());

    caller
        .patch_task(
            task_id,
            bindings::TaskPatch {
                status: Some("backlog".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();
    let epic = rows.epic(epic_id).unwrap();
    assert_eq!(epic.status, crate::models::TaskStatus::Backlog);
    // `completed_at` records the last completion and does not clear on a
    // regression out of done — `stamps_completion`'s own rule.
    assert_eq!(epic.completed_at, completed_at);
}

#[tokio::test]
async fn set_task_epic_recalculates_both_the_source_and_destination_epic() {
    let (caller, rows) = caller();
    let epic_a = caller.create_epic(blank_epic()).await.unwrap();
    let epic_b = caller.create_epic(blank_epic()).await.unwrap();
    let task_id = caller
        .create_task(bindings::Task {
            epic_id: epic_a.0,
            owner: String::new(),
            status: "done".into(),
            ..blank_task()
        })
        .await
        .unwrap();
    // epic_a is now all-done.
    assert_eq!(
        rows.epic(epic_a).unwrap().status,
        crate::models::TaskStatus::Done
    );

    caller
        .set_task_epic(task_id, Some(epic_b), String::new())
        .await
        .unwrap();

    // epic_a lost its only child. `derive_epic_status`'s "no active
    // children is NOT all-done" rule only stops a childless epic being
    // BORN done — with none left to derive from, it returns `None` (no
    // write) rather than forcing a regression, so epic_a stays exactly
    // where it was.
    assert_eq!(
        rows.epic(epic_a).unwrap().status,
        crate::models::TaskStatus::Done
    );
    // epic_b gained a done child and is now all-done.
    assert_eq!(
        rows.epic(epic_b).unwrap().status,
        crate::models::TaskStatus::Done
    );
}

#[tokio::test]
async fn delete_epic_cascades_to_sub_epics_and_their_tasks() {
    let (caller, rows) = caller();
    let parent = caller.create_epic(blank_epic()).await.unwrap();
    let child = caller
        .create_epic(bindings::Epic {
            parent_epic_id: parent.0,
            ..blank_epic()
        })
        .await
        .unwrap();
    let task_id = caller
        .create_task(bindings::Task {
            epic_id: child.0,
            owner: String::new(),
            ..blank_task()
        })
        .await
        .unwrap();
    // `delete_epic` refuses while any task in the subtree is not `done`
    // (task #4971).
    caller
        .patch_task(
            task_id,
            bindings::TaskPatch {
                status: Some("done".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();

    caller.delete_epic(parent).await.unwrap();

    assert!(rows.epic(parent).is_none());
    assert!(rows.epic(child).is_none());
    assert!(rows.task(task_id).is_none());
}

#[tokio::test]
async fn delete_epic_refuses_while_a_subtree_task_is_not_done() {
    let (caller, rows) = caller();
    let parent = caller.create_epic(blank_epic()).await.unwrap();
    let child = caller
        .create_epic(bindings::Epic {
            parent_epic_id: parent.0,
            ..blank_epic()
        })
        .await
        .unwrap();
    let task_id = caller
        .create_task(bindings::Task {
            epic_id: child.0,
            owner: String::new(),
            ..blank_task()
        })
        .await
        .unwrap();

    let outcome = caller.delete_epic(parent).await.unwrap();

    assert!(!outcome.won());
    assert!(rows.epic(parent).is_some());
    assert!(rows.epic(child).is_some());
    assert!(rows.task(task_id).is_some());
}

#[tokio::test]
async fn batch_delete_removes_a_plain_task_and_an_epic_subtree_together() {
    let (caller, rows) = caller();
    let plain_epic = caller.create_epic(blank_epic()).await.unwrap();
    let plain_task = caller
        .create_task(bindings::Task {
            epic_id: plain_epic.0,
            owner: String::new(),
            ..blank_task()
        })
        .await
        .unwrap();
    caller
        .patch_task(
            plain_task,
            bindings::TaskPatch {
                status: Some("done".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();

    let doomed_epic = caller.create_epic(blank_epic()).await.unwrap();
    let doomed_task = caller
        .create_task(bindings::Task {
            epic_id: doomed_epic.0,
            owner: String::new(),
            ..blank_task()
        })
        .await
        .unwrap();
    caller
        .patch_task(
            doomed_task,
            bindings::TaskPatch {
                status: Some("done".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();

    let outcome = caller
        .batch_delete(vec![plain_task], vec![doomed_epic])
        .await
        .unwrap();

    assert!(outcome.won());
    assert!(rows.task(plain_task).is_none());
    assert!(rows.epic(doomed_epic).is_none());
    assert!(rows.task(doomed_task).is_none());
    // Untouched: neither selected.
    assert!(rows.epic(plain_epic).is_some());
}

#[tokio::test]
async fn batch_delete_refuses_and_deletes_nothing_when_one_item_is_not_done() {
    let (caller, rows) = caller();
    let done_epic = caller.create_epic(blank_epic()).await.unwrap();
    let done_task = caller
        .create_task(bindings::Task {
            epic_id: done_epic.0,
            owner: String::new(),
            ..blank_task()
        })
        .await
        .unwrap();
    caller
        .patch_task(
            done_task,
            bindings::TaskPatch {
                status: Some("done".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();

    let not_done_epic = caller.create_epic(blank_epic()).await.unwrap();
    let not_done_task = caller
        .create_task(bindings::Task {
            epic_id: not_done_epic.0,
            owner: String::new(),
            ..blank_task()
        })
        .await
        .unwrap();

    let outcome = caller
        .batch_delete(vec![], vec![done_epic, not_done_epic])
        .await
        .unwrap();

    assert!(!outcome.won());
    // "one operation, or nothing at all": the done epic survives too.
    assert!(rows.epic(done_epic).is_some());
    assert!(rows.task(done_task).is_some());
    assert!(rows.epic(not_done_epic).is_some());
    assert!(rows.task(not_done_task).is_some());
}

#[tokio::test]
async fn patch_epic_applies_the_epic_patch_helper() {
    let (caller, rows) = caller();
    let epic_id = caller.create_epic(blank_epic()).await.unwrap();
    caller
        .patch_epic(
            epic_id,
            bindings::EpicPatch {
                title: Some("renamed".into()),
                ..blank_epic_patch()
            },
        )
        .await
        .unwrap();
    assert_eq!(rows.epic(epic_id).unwrap().title, "renamed");
}
