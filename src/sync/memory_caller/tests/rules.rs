//! The data rules of store-split M2 (task #53255), as the in-memory reducer
//! twin enforces them. `tests/memory_caller_conformance.rs` holds the same
//! scenario against the real module.

use super::*;
use crate::models::EpicOrigin;

async fn grouped_root(caller: &MemoryReducerCaller) -> EpicId {
    caller
        .create_epic(bindings::Epic {
            group_by_repo: true,
            ..blank_epic()
        })
        .await
        .unwrap()
}

async fn task_in(caller: &MemoryReducerCaller, epic: EpicId, repo: &str) -> TaskId {
    caller
        .create_task(bindings::Task {
            epic_id: epic.0,
            owner: String::new(),
            repo_path: repo.into(),
            ..blank_task()
        })
        .await
        .unwrap()
}

fn refusal(outcome: ReducerOutcome) -> String {
    match outcome {
        ReducerOutcome::Refused(why) => why,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

// -- sub-status legality ------------------------------------------------------

#[tokio::test]
async fn patch_task_refuses_a_sub_status_that_is_illegal_for_the_status() {
    let (caller, rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();
    let outcome = caller
        .patch_task(
            id,
            bindings::TaskPatch {
                sub_status: Some("active".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        refusal(outcome),
        "sub_status 'active' is not valid for status 'backlog'"
    );
    assert_eq!(
        rows.task(id).unwrap().sub_status,
        crate::models::SubStatus::None,
        "a refused patch leaves the row as it was"
    );
}

#[tokio::test]
async fn patch_task_judges_the_sub_status_against_the_status_it_moves_to() {
    let (caller, rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();
    let outcome = caller
        .patch_task(
            id,
            bindings::TaskPatch {
                status: Some("running".into()),
                sub_status: Some("active".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();
    assert!(outcome.won());
    assert_eq!(
        rows.task(id).unwrap().sub_status,
        crate::models::SubStatus::Active
    );
}

/// A task that is already running, as a dispatch claim leaves it.
async fn running_task(caller: &MemoryReducerCaller) -> TaskId {
    caller
        .create_task(bindings::Task {
            status: "running".into(),
            sub_status: "active".into(),
            ..blank_task()
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn batch_patch_sub_status_skips_an_entry_that_is_illegal_for_the_status() {
    let (caller, rows) = caller();
    let backlog = caller.create_task(blank_task()).await.unwrap();
    let running = running_task(&caller).await;
    caller
        .batch_patch_sub_status(vec![
            (backlog, crate::models::SubStatus::Approved),
            (running, crate::models::SubStatus::Stale),
        ])
        .await
        .unwrap();
    assert_eq!(
        rows.task(backlog).unwrap().sub_status,
        crate::models::SubStatus::None,
        "backlog never holds approved"
    );
    assert_eq!(
        rows.task(running).unwrap().sub_status,
        crate::models::SubStatus::Stale,
        "a legal entry in the same batch still applies"
    );
}

#[tokio::test]
async fn record_pre_tool_use_refuses_a_sub_status_that_is_illegal_for_running() {
    let (caller, rows) = caller();
    let id = running_task(&caller).await;
    let outcome = caller
        .record_pre_tool_use(id, crate::models::SubStatus::Approved, at(TEST_STAMP))
        .await
        .unwrap();
    assert_eq!(
        refusal(outcome),
        "sub_status 'approved' is not valid for status 'running'"
    );
    assert_eq!(
        rows.task(id).unwrap().sub_status,
        crate::models::SubStatus::Active
    );
}

// -- epic_id through patch_task -------------------------------------------------

#[tokio::test]
async fn patch_task_refuses_epic_id_because_set_task_epic_owns_the_move() {
    let (caller, rows) = caller();
    let epic = caller.create_epic(blank_epic()).await.unwrap();
    let id = caller.create_task(blank_task()).await.unwrap();
    let outcome = caller
        .patch_task(
            id,
            bindings::TaskPatch {
                epic_id: Some(epic.0),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();
    assert!(refusal(outcome).contains("set_task_epic"));
    assert_eq!(rows.task(id).unwrap().epic_id, None);
}

// -- epic patch checks --------------------------------------------------------

#[tokio::test]
async fn patch_epic_refuses_a_parent_cycle() {
    let (caller, rows) = caller();
    let a = caller.create_epic(blank_epic()).await.unwrap();
    let b = caller
        .create_epic(bindings::Epic {
            parent_epic_id: a.0,
            ..blank_epic()
        })
        .await
        .unwrap();
    let outcome = caller
        .patch_epic(
            a,
            bindings::EpicPatch {
                parent_epic_id: Some(b.0),
                ..blank_epic_patch()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        refusal(outcome),
        "Setting this parent would create a cycle in the epic hierarchy"
    );
    assert_eq!(rows.epic(a).unwrap().parent_epic_id, None);
}

#[tokio::test]
async fn patch_epic_refuses_to_move_a_repo_group_sub_epic() {
    let (caller, rows) = caller();
    let root = grouped_root(&caller).await;
    task_in(&caller, root, "/x/alpha").await;
    let sub = rows
        .epics_with_parent(Some(root))
        .into_iter()
        .find(|e| e.origin == EpicOrigin::RepoGroup)
        .expect("the task was routed into a repo-group sub-epic")
        .id;
    for new_parent in [0, root.0] {
        let outcome = caller
            .patch_epic(
                sub,
                bindings::EpicPatch {
                    parent_epic_id: Some(new_parent),
                    ..blank_epic_patch()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            refusal(outcome),
            "Cannot reparent an auto-created repo-group sub-epic"
        );
    }
}

#[tokio::test]
async fn patch_epic_refuses_append_only_with_grouping_whichever_flag_comes_second() {
    let (caller, _rows) = caller();
    let flat = caller.create_epic(blank_epic()).await.unwrap();
    let appends = caller
        .patch_epic(
            flat,
            bindings::EpicPatch {
                feed_append_only: Some(true),
                ..blank_epic_patch()
            },
        )
        .await
        .unwrap();
    assert!(appends.won());
    let groups = caller
        .patch_epic(
            flat,
            bindings::EpicPatch {
                group_by_repo: Some(true),
                ..blank_epic_patch()
            },
        )
        .await
        .unwrap();
    assert!(refusal(groups).starts_with("feed_append_only cannot be combined"));
}

#[tokio::test]
async fn patch_epic_leaves_an_unrelated_edit_to_an_already_invalid_epic_alone() {
    // A restore or an older board can leave the combination in place; a title
    // edit must not be blocked by it.
    let (caller, _rows) = caller();
    let id = caller
        .create_epic(bindings::Epic {
            feed_append_only: true,
            group_by_repo: true,
            ..blank_epic()
        })
        .await
        .unwrap();
    let outcome = caller
        .patch_epic(
            id,
            bindings::EpicPatch {
                title: Some("renamed".into()),
                ..blank_epic_patch()
            },
        )
        .await
        .unwrap();
    assert!(outcome.won());
}

// -- repo grouping ------------------------------------------------------------

#[tokio::test]
async fn create_task_in_a_grouped_root_lands_in_its_repo_sub_epic() {
    let (caller, rows) = caller();
    let root = grouped_root(&caller).await;
    let id = task_in(&caller, root, "/x/alpha").await;
    let epic = rows.task(id).unwrap().epic_id.unwrap();
    assert_ne!(epic, root);
    let sub = rows.epic(epic).unwrap();
    assert_eq!(sub.title, "alpha");
    assert_eq!(sub.origin, EpicOrigin::RepoGroup);
    assert_eq!(sub.parent_epic_id, Some(root));
}

#[tokio::test]
async fn two_tasks_of_one_repo_share_a_sub_epic() {
    let (caller, rows) = caller();
    let root = grouped_root(&caller).await;
    let a = task_in(&caller, root, "/x/alpha").await;
    let b = task_in(&caller, root, "/y/alpha").await;
    assert_eq!(rows.task(a).unwrap().epic_id, rows.task(b).unwrap().epic_id);
    assert_eq!(rows.epics_with_parent(Some(root)).len(), 1);
}

#[tokio::test]
async fn create_task_in_a_plain_or_feed_epic_is_not_routed() {
    let (caller, rows) = caller();
    let plain = caller.create_epic(blank_epic()).await.unwrap();
    let feed = caller
        .create_epic(bindings::Epic {
            group_by_repo: true,
            feed_command: "gh ...".into(),
            ..blank_epic()
        })
        .await
        .unwrap();
    for epic in [plain, feed] {
        let id = task_in(&caller, epic, "/x/alpha").await;
        assert_eq!(rows.task(id).unwrap().epic_id, Some(epic));
    }
}

#[tokio::test]
async fn set_task_epic_routes_into_the_grouped_roots_sub_epic() {
    let (caller, rows) = caller();
    let root = grouped_root(&caller).await;
    let plain = caller.create_epic(blank_epic()).await.unwrap();
    let id = task_in(&caller, plain, "/x/alpha").await;
    caller
        .set_task_epic(id, Some(root), String::new())
        .await
        .unwrap();
    let landed = rows.task(id).unwrap().epic_id.unwrap();
    assert_ne!(landed, root);
    assert_eq!(rows.epic(landed).unwrap().title, "alpha");
}

#[tokio::test]
async fn set_task_epic_onto_the_root_it_is_already_grouped_under_is_a_no_op() {
    let (caller, rows) = caller();
    let root = grouped_root(&caller).await;
    let id = task_in(&caller, root, "/x/alpha").await;
    let before = rows.task(id).unwrap().epic_id;
    caller
        .set_task_epic(id, Some(root), String::new())
        .await
        .unwrap();
    assert_eq!(rows.task(id).unwrap().epic_id, before);
    assert_eq!(rows.epics_with_parent(Some(root)).len(), 1);
}

#[tokio::test]
async fn a_repo_change_moves_the_task_and_removes_the_emptied_sub_epic() {
    let (caller, rows) = caller();
    let root = grouped_root(&caller).await;
    let id = task_in(&caller, root, "/x/alpha").await;
    let old_sub = rows.task(id).unwrap().epic_id.unwrap();
    caller
        .patch_task(
            id,
            bindings::TaskPatch {
                repo_path: Some("/x/beta".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();
    let new_sub = rows.task(id).unwrap().epic_id.unwrap();
    assert_ne!(new_sub, old_sub);
    assert_eq!(rows.epic(new_sub).unwrap().title, "beta");
    assert!(rows.epic(old_sub).is_none(), "the emptied sub-epic is gone");
}

#[tokio::test]
async fn a_repo_change_keeps_a_sub_epic_that_still_has_tasks() {
    let (caller, rows) = caller();
    let root = grouped_root(&caller).await;
    let stays = task_in(&caller, root, "/x/alpha").await;
    let moves = task_in(&caller, root, "/x/alpha").await;
    let old_sub = rows.task(stays).unwrap().epic_id.unwrap();
    caller
        .patch_task(
            moves,
            bindings::TaskPatch {
                repo_path: Some("/x/beta".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();
    assert!(rows.epic(old_sub).is_some());
}

#[tokio::test]
async fn a_repo_change_in_a_hand_made_sub_epic_moves_nothing() {
    let (caller, rows) = caller();
    let root = grouped_root(&caller).await;
    let manual = caller
        .create_epic(bindings::Epic {
            parent_epic_id: root.0,
            ..blank_epic()
        })
        .await
        .unwrap();
    let id = task_in(&caller, manual, "/x/alpha").await;
    caller
        .patch_task(
            id,
            bindings::TaskPatch {
                repo_path: Some("/x/beta".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();
    assert_eq!(rows.task(id).unwrap().epic_id, Some(manual));
}

#[tokio::test]
async fn regroup_moves_every_direct_task_into_a_repo_sub_epic() {
    let (caller, rows) = caller();
    let root = caller.create_epic(blank_epic()).await.unwrap();
    task_in(&caller, root, "/x/alpha").await;
    task_in(&caller, root, "/x/beta").await;
    caller
        .patch_epic(
            root,
            bindings::EpicPatch {
                group_by_repo: Some(true),
                ..blank_epic_patch()
            },
        )
        .await
        .unwrap();
    assert!(caller.regroup_epic(root).await.unwrap().won());
    assert!(rows.tasks_for_epic(root).is_empty());
    assert_eq!(rows.epics_with_parent(Some(root)).len(), 2);
    // Idempotent.
    assert!(caller.regroup_epic(root).await.unwrap().won());
    assert_eq!(rows.epics_with_parent(Some(root)).len(), 2);
}

#[tokio::test]
async fn flatten_rehomes_tasks_deletes_empty_repo_groups_and_keeps_hand_made_ones() {
    let (caller, rows) = caller();
    let root = grouped_root(&caller).await;
    let id = task_in(&caller, root, "/x/alpha").await;
    let manual = caller
        .create_epic(bindings::Epic {
            parent_epic_id: root.0,
            ..blank_epic()
        })
        .await
        .unwrap();
    caller
        .patch_epic(
            root,
            bindings::EpicPatch {
                group_by_repo: Some(false),
                ..blank_epic_patch()
            },
        )
        .await
        .unwrap();

    assert!(caller.flatten_epic(root).await.unwrap().won());

    assert_eq!(
        rows.task(id).unwrap().epic_id,
        Some(root),
        "re-homed, not deleted"
    );
    let subs = rows.epics_with_parent(Some(root));
    assert_eq!(subs.len(), 1, "the repo-group sub-epic is gone");
    assert_eq!(subs[0].id, manual);
}

#[tokio::test]
async fn flatten_re_scopes_the_deleted_sub_epics_learnings_to_the_root() {
    let (caller, rows) = caller();
    let root = grouped_root(&caller).await;
    let id = task_in(&caller, root, "/x/alpha").await;
    let sub = rows.task(id).unwrap().epic_id.unwrap();
    let learning = caller
        .create_learning(bindings::Learning {
            scope: "epic".into(),
            scope_ref: Some(sub.0.to_string()),
            ..super::learnings::blank_learning()
        })
        .await
        .unwrap();
    caller
        .patch_epic(
            root,
            bindings::EpicPatch {
                group_by_repo: Some(false),
                ..blank_epic_patch()
            },
        )
        .await
        .unwrap();
    caller.flatten_epic(root).await.unwrap();
    let kept = rows.learning(learning).expect("the learning survives");
    assert_eq!(kept.scope_ref.as_deref(), Some(root.0.to_string().as_str()));
}

// -- a refused patch leaves nothing behind ---------------------------------------

#[tokio::test]
async fn a_refused_patch_leaves_no_phoenix_successor_behind() {
    let (caller, rows) = caller();
    let epic = caller.create_epic(blank_epic()).await.unwrap();
    // A running phoenix task in an epic: the patch below would respawn it.
    let id = caller
        .create_task(bindings::Task {
            epic_id: epic.0,
            owner: String::new(),
            status: "running".into(),
            sub_status: "active".into(),
            phoenix: true,
            ..blank_task()
        })
        .await
        .unwrap();
    let before = rows.tasks().len();
    // Done respawns, but an owner on an epic task is refused by the write.
    let outcome = caller
        .patch_task(
            id,
            bindings::TaskPatch {
                status: Some("done".into()),
                sub_status: Some("none".into()),
                owner: Some("bob".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();
    assert!(matches!(outcome, ReducerOutcome::Refused(_)), "{outcome:?}");
    assert_eq!(rows.tasks().len(), before, "no successor was inserted");
    assert!(
        rows.task(id).unwrap().phoenix,
        "the flag is still the receipt"
    );
}
