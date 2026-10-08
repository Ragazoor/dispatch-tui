use super::*;
use crate::models::MIN_FEED_INTERVAL_SECS;
use crate::store::{EpicCrud, EpicRead, Store};

#[test]
fn for_epic_sets_only_the_epic_id() {
    let params = UpdateEpicParams::for_epic(EpicId(7));
    assert_eq!(params.epic_id, EpicId(7));
    assert!(params.updated_field_names().is_empty());
}

#[test]
fn update_epic_params_has_any_field_consistent_with_updated_field_names() {
    let with_field = UpdateEpicParams {
        title: Some("x".to_string()),
        ..UpdateEpicParams::for_epic(EpicId(1))
    };
    assert!(
        with_field.has_any_field(),
        "has_any_field should be true when title is set"
    );
    assert!(
        !with_field.updated_field_names().is_empty(),
        "updated_field_names should be non-empty when title is set"
    );

    let empty = UpdateEpicParams::for_epic(EpicId(1));
    assert!(
        !empty.has_any_field(),
        "has_any_field should be false when no fields are set"
    );
    assert!(
        empty.updated_field_names().is_empty(),
        "updated_field_names should be empty when no fields are set"
    );
}

#[test]
fn update_epic_params_every_field_covered() {
    // The exhaustive destructuring in updated_field_names() already makes an
    // unhandled field a compile error; what this test uniquely covers is
    // that each field reports its *own* name.
    let cases: Vec<(&str, UpdateEpicParams)> = vec![
        (
            "title",
            UpdateEpicParams {
                title: Some("t".to_string()),
                ..UpdateEpicParams::for_epic(EpicId(1))
            },
        ),
        (
            "description",
            UpdateEpicParams {
                description: Some("d".to_string()),
                ..UpdateEpicParams::for_epic(EpicId(1))
            },
        ),
        (
            "status",
            UpdateEpicParams {
                status: Some(TaskStatus::Backlog),
                ..UpdateEpicParams::for_epic(EpicId(1))
            },
        ),
        (
            "plan_path",
            UpdateEpicParams {
                plan_path: Some("p".to_string()),
                ..UpdateEpicParams::for_epic(EpicId(1))
            },
        ),
        (
            "sort_order",
            UpdateEpicParams {
                sort_order: Some(0),
                ..UpdateEpicParams::for_epic(EpicId(1))
            },
        ),
        (
            "auto_dispatch",
            UpdateEpicParams {
                auto_dispatch: Some(true),
                ..UpdateEpicParams::for_epic(EpicId(1))
            },
        ),
        (
            "feed_command",
            UpdateEpicParams {
                feed_command: Some(FieldUpdate::Set("cmd".to_string())),
                ..UpdateEpicParams::for_epic(EpicId(1))
            },
        ),
        (
            "feed_interval_secs",
            UpdateEpicParams {
                feed_interval_secs: Some(Some(300)),
                ..UpdateEpicParams::for_epic(EpicId(1))
            },
        ),
        (
            "group_by_repo",
            UpdateEpicParams {
                group_by_repo: Some(true),
                ..UpdateEpicParams::for_epic(EpicId(1))
            },
        ),
        (
            "parent_epic_id",
            UpdateEpicParams {
                parent_epic_id: Some(Some(EpicId(2))),
                ..UpdateEpicParams::for_epic(EpicId(1))
            },
        ),
    ];
    for (expected, params) in &cases {
        assert!(
            params.has_any_field(),
            "has_any_field() should be true when {expected} is set"
        );
        assert_eq!(
            params.updated_field_names(),
            vec![*expected],
            "setting {expected} should report exactly that field name"
        );
    }
}

#[tokio::test]
async fn create_epic_returns_the_post_patch_epic() {
    // sort_order / feed_command / feed_interval_secs are applied in a
    // second write; the returned Epic must carry them, not the pre-patch
    // insert result.
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let svc = EpicService::new(db.clone());

    let epic = svc
        .create_epic(CreateEpicParams {
            sort_order: Some(42),
            feed_command: Some("gh api repos/x/pulls".to_string()),
            feed_interval_secs: Some(300),
            ..CreateEpicParams::fixture("E")
        })
        .await
        .unwrap();

    assert_eq!(epic.sort_order, Some(42));
    assert_eq!(epic.feed_command.as_deref(), Some("gh api repos/x/pulls"));
    assert_eq!(epic.feed_interval_secs, Some(300));
}

// --- the feed-cadence floor (core.allium: "Interval literals", CLAIM 2) ---

fn create_params_with_interval(interval: Option<i64>) -> CreateEpicParams {
    CreateEpicParams {
        feed_command: Some("true".to_string()),
        feed_interval_secs: interval,
        ..CreateEpicParams::fixture("E")
    }
}

/// Creation is bound as tightly as update: validating only the update path
/// would leave an epic able to be *born* busy-looping.
#[tokio::test]
async fn create_epic_rejects_a_sub_floor_interval() {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let svc = EpicService::new(db.clone());

    // 0 busy-loops the runner; a negative used to wrap to a near-infinite
    // interval and silence the feed; 59 is the off-by-one at the boundary.
    for bad in [0, -5, MIN_FEED_INTERVAL_SECS - 1] {
        let err = svc
            .create_epic(create_params_with_interval(Some(bad)))
            .await;
        assert!(
            matches!(err, Err(ServiceError::Validation(_))),
            "creating with feed_interval_secs = {bad} should be rejected, got {err:?}"
        );
    }
}

#[tokio::test]
async fn create_epic_accepts_the_floor_itself_and_an_unset_interval() {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let svc = EpicService::new(db.clone());

    let at_floor = svc
        .create_epic(create_params_with_interval(Some(MIN_FEED_INTERVAL_SECS)))
        .await
        .unwrap();
    assert_eq!(at_floor.feed_interval_secs, Some(MIN_FEED_INTERVAL_SECS));

    // Unset means "inherit the default", which itself clears the floor.
    let unset = svc
        .create_epic(create_params_with_interval(None))
        .await
        .unwrap();
    assert_eq!(unset.feed_interval_secs, None);
}

#[tokio::test]
async fn update_epic_rejects_a_sub_floor_interval() {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let epic = db.create_epic("Test", "", None).await.unwrap();
    let svc = EpicService::new(db.clone());

    for bad in [0, -5, MIN_FEED_INTERVAL_SECS - 1] {
        let err = svc
            .update_epic(UpdateEpicParams {
                feed_interval_secs: Some(Some(bad)),
                ..UpdateEpicParams::for_epic(epic.id)
            })
            .await;
        assert!(
            matches!(err, Err(ServiceError::Validation(_))),
            "updating to feed_interval_secs = {bad} should be rejected, got {err:?}"
        );
    }
}

/// A rejected update must not have written anything — including the other
/// fields in the same call. The editor sends title and interval together,
/// so a partial apply would save a title against a refused cadence.
#[tokio::test]
async fn update_epic_rejecting_the_interval_writes_no_other_field() {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let epic = db.create_epic("Original", "", None).await.unwrap();
    let svc = EpicService::new(db.clone());

    let err = svc
        .update_epic(UpdateEpicParams {
            title: Some("Renamed".to_string()),
            feed_interval_secs: Some(Some(10)),
            ..UpdateEpicParams::for_epic(epic.id)
        })
        .await;
    assert!(matches!(err, Err(ServiceError::Validation(_))), "{err:?}");

    let after = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(
        after.title, "Original",
        "the title must not survive a rejected update"
    );
}

#[tokio::test]
async fn update_epic_accepts_the_floor_itself_and_clearing_the_interval() {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let epic = db.create_epic("Test", "", None).await.unwrap();
    let svc = EpicService::new(db.clone());

    svc.update_epic(UpdateEpicParams {
        feed_interval_secs: Some(Some(MIN_FEED_INTERVAL_SECS)),
        ..UpdateEpicParams::for_epic(epic.id)
    })
    .await
    .unwrap();
    let after = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(after.feed_interval_secs, Some(MIN_FEED_INTERVAL_SECS));

    // Explicit null clears to "inherit the default", never below the floor.
    svc.update_epic(UpdateEpicParams {
        feed_interval_secs: Some(None),
        ..UpdateEpicParams::for_epic(epic.id)
    })
    .await
    .unwrap();
    let cleared = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(cleared.feed_interval_secs, None);
}

/// feeds.allium AppendOnlyFeed: grouping keys on the repo an item belongs
/// to, which is a MIRRORING feed's axis. An append-only feed's items are
/// events keyed by where in the code they fired, so grouping one would put
/// every item in a single sub-epic. Permanently refused; several repos are
/// covered by one flat append-only epic each under a common parent.
#[tokio::test]
async fn update_epic_refuses_append_only_together_with_group_by_repo() {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let epic = db.create_epic("Log warnings", "", None).await.unwrap();
    let svc = EpicService::new(db.clone());

    svc.update_epic(UpdateEpicParams {
        feed_append_only: Some(true),
        ..UpdateEpicParams::for_epic(epic.id)
    })
    .await
    .unwrap();

    // Turning grouping on afterwards must be refused...
    let err = svc
        .update_epic(UpdateEpicParams {
            group_by_repo: Some(true),
            ..UpdateEpicParams::for_epic(epic.id)
        })
        .await;
    assert!(
        matches!(err, Err(ServiceError::Validation(_))),
        "append-only + group_by_repo must be refused, got {err:?}"
    );
    let unchanged = db.get_epic(epic.id).await.unwrap().unwrap();
    assert!(
        !unchanged.group_by_repo,
        "a refused update must write nothing"
    );

    // ...and so must the same pair arriving in the other order.
    let grouped = db.create_epic("Grouped", "", None).await.unwrap();
    svc.update_epic(UpdateEpicParams {
        group_by_repo: Some(true),
        ..UpdateEpicParams::for_epic(grouped.id)
    })
    .await
    .unwrap();
    let reversed = svc
        .update_epic(UpdateEpicParams {
            feed_append_only: Some(true),
            ..UpdateEpicParams::for_epic(grouped.id)
        })
        .await;
    assert!(
        matches!(reversed, Err(ServiceError::Validation(_))),
        "the same pair in the other order must be refused too, got {reversed:?}"
    );

    // Both flags in ONE call is the same refusal, not a way around it.
    let both = db.create_epic("Both at once", "", None).await.unwrap();
    let together = svc
        .update_epic(UpdateEpicParams {
            group_by_repo: Some(true),
            feed_append_only: Some(true),
            ..UpdateEpicParams::for_epic(both.id)
        })
        .await;
    assert!(
        matches!(together, Err(ServiceError::Validation(_))),
        "setting both in one call is the same refusal, not a way around it, got {together:?}"
    );
}

/// The boundary: each flag alone is fine, and turning append-only OFF on a
/// grouped epic must not be caught by the guard.
#[tokio::test]
async fn update_epic_allows_either_flag_alone() {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let svc = EpicService::new(db.clone());

    let grouped = db.create_epic("Grouped", "", None).await.unwrap();
    svc.update_epic(UpdateEpicParams {
        group_by_repo: Some(true),
        feed_append_only: Some(false),
        ..UpdateEpicParams::for_epic(grouped.id)
    })
    .await
    .unwrap();

    let log = db.create_epic("Log warnings", "", None).await.unwrap();
    // Append-only on an ungrouped epic is the shipped case.
    svc.update_epic(UpdateEpicParams {
        feed_append_only: Some(true),
        ..UpdateEpicParams::for_epic(log.id)
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn update_epic_sets_group_by_repo() {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let epic = db.create_epic("Test", "", None).await.unwrap();
    assert!(!epic.group_by_repo);
    let svc = EpicService::new(db.clone());
    svc.update_epic(UpdateEpicParams {
        group_by_repo: Some(true),
        ..UpdateEpicParams::for_epic(epic.id)
    })
    .await
    .unwrap();
    let updated = db.get_epic(epic.id).await.unwrap().unwrap();
    assert!(updated.group_by_repo);
}

fn epic_svc_with_clock(db: Arc<Store>, clock: Arc<dyn crate::clock::Clock>) -> EpicService {
    EpicService::new(db.clone()).with_clock(clock)
}

#[tokio::test]
async fn update_epic_entering_done_stamps_completed_at() {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let epic = db.create_epic("Test", "", None).await.unwrap();
    let now = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    let clock = Arc::new(crate::clock::FixedClock::new(now));
    let svc = epic_svc_with_clock(db.clone(), clock);

    svc.update_epic(UpdateEpicParams {
        status: Some(TaskStatus::Done),
        ..UpdateEpicParams::for_epic(epic.id)
    })
    .await
    .unwrap();

    let updated = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(updated.completed_at, Some(now));
    assert_eq!(
        updated.sort_order, None,
        "the status transition must not touch sort_order"
    );
}

/// Leaving Done writes nothing: `completed_at` records the last completion,
/// not the current status (tasks.allium, ConfirmDone).
#[tokio::test]
async fn update_epic_leaving_done_keeps_completed_at() {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let epic = db.create_epic("Test", "", None).await.unwrap();
    let svc = EpicService::new(db.clone());

    svc.update_epic(UpdateEpicParams {
        status: Some(TaskStatus::Done),
        ..UpdateEpicParams::for_epic(epic.id)
    })
    .await
    .unwrap();
    let finished = db.get_epic(epic.id).await.unwrap().unwrap().completed_at;
    assert!(finished.is_some());

    svc.update_epic(UpdateEpicParams {
        status: Some(TaskStatus::Backlog),
        ..UpdateEpicParams::for_epic(epic.id)
    })
    .await
    .unwrap();

    let updated = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(updated.completed_at, finished);
}

/// An explicit `sort_order` in the same call as a Done status is no longer
/// overridden: the two write different fields now.
#[tokio::test]
async fn update_epic_entering_done_keeps_an_explicit_sort_order() {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let epic = db.create_epic("Test", "", None).await.unwrap();
    let svc = EpicService::new(db.clone());

    svc.update_epic(UpdateEpicParams {
        status: Some(TaskStatus::Done),
        sort_order: Some(7),
        ..UpdateEpicParams::for_epic(epic.id)
    })
    .await
    .unwrap();

    let updated = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(updated.sort_order, Some(7));
    assert!(updated.completed_at.is_some());
}

#[tokio::test]
async fn update_epic_unrelated_field_edit_while_done_leaves_completed_at_untouched() {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let epic = db.create_epic("Test", "", None).await.unwrap();
    let svc = EpicService::new(db.clone());

    svc.update_epic(UpdateEpicParams {
        status: Some(TaskStatus::Done),
        ..UpdateEpicParams::for_epic(epic.id)
    })
    .await
    .unwrap();
    let after_entry = db.get_epic(epic.id).await.unwrap().unwrap().completed_at;

    svc.update_epic(UpdateEpicParams {
        title: Some("Renamed".to_string()),
        ..UpdateEpicParams::for_epic(epic.id)
    })
    .await
    .unwrap();

    let updated = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(updated.completed_at, after_entry);
}

#[tokio::test]
async fn create_sub_epic_succeeds() {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let svc = EpicService::new(db.clone());
    let parent = db.create_epic("Parent", "", None).await.unwrap();
    let sub = svc
        .create_epic(CreateEpicParams {
            parent_epic_id: Some(parent.id),
            ..CreateEpicParams::fixture("Sub")
        })
        .await
        .unwrap();
    assert_eq!(sub.parent_epic_id, Some(parent.id));
}

#[tokio::test]
async fn create_sub_epic_recalculates_done_parent() {
    // Regression guard: attaching a new (backlog) sub-epic to a Done
    // parent must regress the parent to Backlog immediately, not wait
    // for some unrelated task write to trigger a recalc.
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let svc = EpicService::new(db.clone());
    let parent = db.create_epic("Parent", "", None).await.unwrap();
    db.patch_epic(parent.id, &EpicPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();

    svc.create_epic(CreateEpicParams {
        parent_epic_id: Some(parent.id),
        ..CreateEpicParams::fixture("Sub")
    })
    .await
    .unwrap();

    let parent = db.get_epic(parent.id).await.unwrap().unwrap();
    assert_eq!(parent.status, TaskStatus::Backlog);
}

#[tokio::test]
async fn create_sub_epic_missing_parent_returns_not_found() {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let svc = EpicService::new(db.clone());
    let result = svc
        .create_epic(CreateEpicParams {
            parent_epic_id: Some(EpicId(9999)),
            ..CreateEpicParams::fixture("Sub")
        })
        .await;
    assert!(
        matches!(result, Err(ServiceError::NotFound(_))),
        "expected NotFound for missing parent, got: {result:?}"
    );
}

#[tokio::test]
async fn update_epic_sets_parent() {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let svc = EpicService::new(db.clone());
    let parent = db.create_epic("Parent", "", None).await.unwrap();
    let child = db.create_epic("Child", "", None).await.unwrap();
    assert!(child.parent_epic_id.is_none());
    svc.update_epic(UpdateEpicParams {
        parent_epic_id: Some(Some(parent.id)),
        ..UpdateEpicParams::for_epic(child.id)
    })
    .await
    .unwrap();
    let updated = db.get_epic(child.id).await.unwrap().unwrap();
    assert_eq!(updated.parent_epic_id, Some(parent.id));
}

#[tokio::test]
async fn update_epic_clears_parent() {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let svc = EpicService::new(db.clone());
    let parent = db.create_epic("Parent", "", None).await.unwrap();
    let child = db.create_epic("Child", "", Some(parent.id)).await.unwrap();
    assert_eq!(child.parent_epic_id, Some(parent.id));
    svc.update_epic(UpdateEpicParams {
        parent_epic_id: Some(None),
        ..UpdateEpicParams::for_epic(child.id)
    })
    .await
    .unwrap();
    let updated = db.get_epic(child.id).await.unwrap().unwrap();
    assert!(updated.parent_epic_id.is_none());
}

#[tokio::test]
async fn update_epic_parent_id_absent_is_noop() {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let svc = EpicService::new(db.clone());
    let parent = db.create_epic("Parent", "", None).await.unwrap();
    let child = db.create_epic("Child", "", Some(parent.id)).await.unwrap();
    svc.update_epic(UpdateEpicParams {
        title: Some("New Title".to_string()),
        ..UpdateEpicParams::for_epic(child.id)
    })
    .await
    .unwrap();
    let updated = db.get_epic(child.id).await.unwrap().unwrap();
    assert_eq!(updated.parent_epic_id, Some(parent.id), "parent unchanged");
}

#[tokio::test]
async fn update_epic_reparent_recalculates_old_and_new_parent() {
    // Regression guard: reparenting a sub-epic changes both parents'
    // active_sub_epics set, so both must be recalculated immediately.
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let svc = EpicService::new(db.clone());
    let old_parent = db.create_epic("Old", "", None).await.unwrap();
    let new_parent = db.create_epic("New", "", None).await.unwrap();
    let child = db
        .create_epic("Child", "", Some(old_parent.id))
        .await
        .unwrap();
    // A second, still-Running child stays behind on old_parent after the
    // reparent below, so a correct recalc must regress old_parent from
    // its manually-forced Done — proving recalc actually ran rather than
    // old_parent merely keeping an unrelated status.
    let sibling = db
        .create_epic("Sibling", "", Some(old_parent.id))
        .await
        .unwrap();
    db.patch_epic(sibling.id, &EpicPatch::new().status(TaskStatus::Running))
        .await
        .unwrap();
    db.patch_epic(old_parent.id, &EpicPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();
    db.patch_epic(new_parent.id, &EpicPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();

    svc.update_epic(UpdateEpicParams {
        parent_epic_id: Some(Some(new_parent.id)),
        ..UpdateEpicParams::for_epic(child.id)
    })
    .await
    .unwrap();

    let old_parent = db.get_epic(old_parent.id).await.unwrap().unwrap();
    let new_parent = db.get_epic(new_parent.id).await.unwrap().unwrap();
    assert_eq!(
        old_parent.status,
        TaskStatus::Backlog,
        "old parent still has a Running child and should regress from its stale Done"
    );
    assert_eq!(
        new_parent.status,
        TaskStatus::Backlog,
        "new parent gains a backlog child and should regress from done"
    );
}

#[tokio::test]
async fn update_epic_status_change_recalculates_parent() {
    // Regression guard: explicitly setting a sub-epic's status changes
    // its parent's active_sub_epics rollup and must recalculate it.
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let svc = EpicService::new(db.clone());
    let parent = db.create_epic("Parent", "", None).await.unwrap();
    let child = db.create_epic("Child", "", Some(parent.id)).await.unwrap();

    svc.update_epic(UpdateEpicParams {
        status: Some(TaskStatus::Done),
        ..UpdateEpicParams::for_epic(child.id)
    })
    .await
    .unwrap();
    let parent_after_child_done = db.get_epic(parent.id).await.unwrap().unwrap();
    assert_eq!(parent_after_child_done.status, TaskStatus::Done);

    // Regress the child back to Running — parent must be recalculated
    // immediately, not left stale at Done.
    svc.update_epic(UpdateEpicParams {
        status: Some(TaskStatus::Running),
        ..UpdateEpicParams::for_epic(child.id)
    })
    .await
    .unwrap();

    let parent = db.get_epic(parent.id).await.unwrap().unwrap();
    assert_eq!(parent.status, TaskStatus::Backlog);
}

#[tokio::test]
async fn update_epic_cycle_detection() {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let svc = EpicService::new(db.clone());
    let a = db.create_epic("A", "", None).await.unwrap();
    let b = db.create_epic("B", "", Some(a.id)).await.unwrap();
    // Trying to set A's parent to B would create a cycle: A → B → A
    let result = svc
        .update_epic(UpdateEpicParams {
            parent_epic_id: Some(Some(b.id)),
            ..UpdateEpicParams::for_epic(a.id)
        })
        .await;
    assert!(
        matches!(result, Err(ServiceError::Validation(_))),
        "expected Validation error for cycle, got: {:?}",
        result
    );
    // DB must be unchanged
    let a_after = db.get_epic(a.id).await.unwrap().unwrap();
    assert!(a_after.parent_epic_id.is_none());
}

#[tokio::test]
async fn update_epic_self_parent_rejected() {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let svc = EpicService::new(db.clone());
    let epic = db.create_epic("Epic", "", None).await.unwrap();
    let result = svc
        .update_epic(UpdateEpicParams {
            parent_epic_id: Some(Some(epic.id)),
            ..UpdateEpicParams::for_epic(epic.id)
        })
        .await;
    assert!(
        matches!(result, Err(ServiceError::Validation(_))),
        "expected Validation error for self-parent, got: {:?}",
        result
    );
}

#[tokio::test]
async fn reparent_repo_group_sub_epic_is_rejected() {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let svc = EpicService::new(db.clone());
    let root = db.create_epic("root", "", None).await.unwrap();
    let other = db.create_epic("other", "", None).await.unwrap();
    let sub = db
        .create_repo_group_sub_epic(root.id, "alpha")
        .await
        .unwrap();

    let err = svc
        .update_epic(UpdateEpicParams {
            parent_epic_id: Some(Some(other.id)),
            ..UpdateEpicParams::for_epic(sub)
        })
        .await;
    assert!(
        matches!(err, Err(ServiceError::Validation(_))),
        "expected Validation error for reparenting a RepoGroup sub-epic, got: {:?}",
        err
    );
}

#[tokio::test]
async fn detach_repo_group_sub_epic_is_rejected() {
    // Nice-to-have guard: detaching (Some(None)) a RepoGroup sub-epic to root
    // must be rejected, just like reparenting it to another epic.
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let svc = EpicService::new(db.clone());
    let root = db.create_epic("root", "", None).await.unwrap();
    let sub = db
        .create_repo_group_sub_epic(root.id, "alpha")
        .await
        .unwrap();

    let err = svc
        .update_epic(UpdateEpicParams {
            parent_epic_id: Some(None), // detach to root
            ..UpdateEpicParams::for_epic(sub)
        })
        .await;
    assert!(
        matches!(err, Err(ServiceError::Validation(_))),
        "expected Validation error for detaching a RepoGroup sub-epic, got: {:?}",
        err
    );
}

#[tokio::test]
async fn detach_manual_sub_epic_is_allowed() {
    // Regression guard: detaching a Manual sub-epic to root must still work.
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let svc = EpicService::new(db.clone());
    let parent = db.create_epic("parent", "", None).await.unwrap();
    let child = db.create_epic("child", "", Some(parent.id)).await.unwrap();
    assert_eq!(child.parent_epic_id, Some(parent.id));

    svc.update_epic(UpdateEpicParams {
        parent_epic_id: Some(None),
        ..UpdateEpicParams::for_epic(child.id)
    })
    .await
    .unwrap();

    let updated = db.get_epic(child.id).await.unwrap().unwrap();
    assert!(
        updated.parent_epic_id.is_none(),
        "Manual sub-epic can be detached"
    );
}

#[tokio::test]
async fn progress_aggregates_descendants_for_grouped_epic() {
    use crate::store::{EpicCrud as _, TaskCrud as _};
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let svc = EpicService::new(db.clone());
    let root = db.create_epic("root", "", None).await.unwrap();
    db.patch_epic(root.id, &crate::store::EpicPatch::new().group_by_repo(true))
        .await
        .unwrap();
    let sub = db
        .create_repo_group_sub_epic(root.id, "alpha")
        .await
        .unwrap();
    db.create_task(crate::store::CreateTaskRequest {
        epic_id: Some(sub),
        ..crate::store::CreateTaskRequest::fixture("t", "/x/alpha")
    })
    .await
    .unwrap();

    let rows = svc.list_epics_with_progress().await.unwrap();
    let (_, _done, total) = rows.iter().find(|(e, _, _)| e.id == root.id).unwrap();
    assert_eq!(*total, 1, "grouped root aggregates descendant task counts");
}
