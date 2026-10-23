use super::*;

/// A create sends one row and answers with the id the store generated.
///
/// The id matters because the caller dispatches the task it just made. A writer
/// that returned a placeholder would have the board open a detail view on a
/// task that is not the one it created.
#[tokio::test]
async fn a_create_sends_the_row_and_returns_the_generated_id() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    let id = writer.create_task(a_request()).await.unwrap();

    assert_eq!(id, TaskId(42));
    let Some(Sent::CreateTask(row)) = caller.sent().into_iter().next() else {
        panic!("expected one create, got {:?}", caller.sent());
    };
    assert_eq!(row.title, "t");
    assert_eq!(row.id, 0, "the store generates the id");
}

/// The board's clock stamps a create, and it stamps both columns the same.
///
/// `created_at` records when the person asked, which is a fact about this
/// machine rather than about the store. Everything the store DERIVES afterwards
/// uses the store's clock instead, so two boards' rows are ordered by one
/// clock — see `Store::now`.
#[tokio::test]
async fn a_create_is_stamped_with_this_boards_clock() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.create_task(a_request()).await.unwrap();

    let Some(Sent::CreateTask(row)) = caller.sent().into_iter().next() else {
        panic!("expected a create");
    };
    assert_eq!(row.created_at, AT_STORED);
    assert_eq!(row.updated_at, AT_STORED);
}

/// The owner comes from the writer's identity, not from the request — the
/// request has no field for it. `core.allium: OwnerTracksUserBoardTask`.
#[tokio::test]
async fn an_epicless_create_carries_this_users_identity() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.create_task(a_request()).await.unwrap();

    let Some(Sent::CreateTask(row)) = caller.sent().into_iter().next() else {
        panic!("expected a create");
    };
    assert_eq!(row.owner, "user-me");
}

/// ...and a task in an epic carries none. The other arm of the same rule, and
/// the one the module refuses outright.
#[tokio::test]
async fn a_create_in_an_epic_carries_no_owner() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer
        .create_task(CreateTaskRequest {
            epic_id: Some(EpicId(3)),
            ..a_request()
        })
        .await
        .unwrap();

    let Some(Sent::CreateTask(row)) = caller.sent().into_iter().next() else {
        panic!("expected a create");
    };
    assert_eq!(row.owner, "");
}

/// `created_by` is a DIFFERENT question from `owner` — it survives regardless
/// of epic membership, because it is how `sync.allium`'s `own_creations`
/// subscription finds a task this identity just created no matter which epic
/// it landed in. `core.allium: Task.created_by`.
#[tokio::test]
async fn a_task_create_carries_the_creator_regardless_of_epic() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.create_task(a_request()).await.unwrap();
    writer
        .create_task(CreateTaskRequest {
            epic_id: Some(EpicId(3)),
            ..a_request()
        })
        .await
        .unwrap();

    let sent = caller.sent();
    for entry in sent {
        let Sent::CreateTask(row) = entry else {
            panic!("expected two creates");
        };
        assert_eq!(row.created_by, "user-me");
    }
}

/// A patch travels as a patch, naming only what it changes.
#[tokio::test]
async fn a_patch_names_only_the_fields_it_changes() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer
        .patch_task(TaskId(7), &TaskPatch::new().status(TaskStatus::Running))
        .await
        .unwrap();

    let Some(Sent::PatchTask(id, patch)) = caller.sent().into_iter().next() else {
        panic!("expected a patch");
    };
    assert_eq!(id, TaskId(7));
    assert_eq!(patch.status.as_deref(), Some("running"));
    assert_eq!(patch.title, None);
    assert_eq!(patch.worktree, None);
}

/// `sync.allium: AWriteWithNoConnectionIsRefused` — the reason reaches the
/// caller, and nothing was sent.
#[tokio::test]
async fn a_refused_write_carries_the_stores_reason() {
    let (writer, caller) = writer_with(RecordingCaller::refusing("store unreachable"));

    let refused = writer.create_task(a_request()).await;

    let why = refused.expect_err("a refused write must fail");
    assert!(
        why.to_string().contains("store unreachable"),
        "expected the store's reason, got {why}"
    );
    assert!(caller.sent().is_empty());
}

/// And it is not retried. One ask, one answer, no second attempt hiding inside
/// the first — `sync.allium: NoWriteIsEverQueued`.
#[tokio::test]
async fn a_refusal_is_not_retried_inside_the_writer() {
    let (writer, caller) = writer_with(RecordingCaller::refusing("down"));

    for _ in 0..3 {
        assert!(writer.delete_task(TaskId(1)).await.is_err());
    }
    // Every attempt reached the caller and failed there. Nothing accumulated.
    assert!(caller.sent().is_empty());
}

/// A repo path is stamped by the board too, for the same reason a create is:
/// `last_used` is when this person used it, and the path is stamped with this
/// host (`core.allium: SavedRepoPath.host`).
#[tokio::test]
async fn a_repo_path_is_stamped_with_this_boards_clock() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.save_repo_path("/repo").await.unwrap();

    assert_eq!(
        caller.sent(),
        vec![Sent::SaveRepoPath(
            "/repo".into(),
            AT_STORED.into(),
            "host-me".into()
        )]
    );
}

/// A board that has never connected cannot create a task on its own user
/// board, because there is no board to put it on. Refused here rather than
/// sent, so the operator gets the actionable message instead of the module's.
#[tokio::test]
async fn a_board_with_no_identity_cannot_create_an_epicless_task() {
    let (writer, caller) = writer_with_no_identity(RecordingCaller::default());

    let refused = writer.create_task(a_request()).await;

    assert!(refused
        .expect_err("a user-board task needs a user")
        .to_string()
        .contains("no user identity"));
    assert!(caller.sent().is_empty());
}

/// ...and NEITHER can a task inside an epic, even though that task carries no
/// owner. `sync.allium: CreatesRequireASettledIdentity` — every create needs a
/// settled identity now, because `created_by` needs a name to stamp regardless
/// of whether `owner` does.
#[tokio::test]
async fn a_board_with_no_identity_cannot_create_a_task_in_an_epic_either() {
    let (writer, caller) = writer_with_no_identity(RecordingCaller::default());

    let refused = writer
        .create_task(CreateTaskRequest {
            epic_id: Some(EpicId(3)),
            ..a_request()
        })
        .await;

    assert!(refused.is_err(), "a create needs a name to stamp");
    assert!(caller.sent().is_empty());
}

/// A delete names its row and nothing else. The cascade — shells,
/// subagents — is the store's, inside the same transaction, because a client
/// doing it in several calls could be interrupted between them.
#[tokio::test]
async fn a_delete_sends_only_the_id() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.delete_task(TaskId(9)).await.unwrap();

    assert_eq!(caller.sent(), vec![Sent::DeleteTask(TaskId(9))]);
}

// -- A refusal is not an outage ---------------------------------------------

/// THE DISTINCTION THE CLAIM DEPENDS ON. A store that says no is a different
/// thing from a store that is not there, and only one of them is a failure.
///
/// Since store-split M4 the claim is the dispatch request
/// (`store-split.allium: RequestDispatch`). Losing a race is the ordinary
/// outcome of two entry points reaching for one task; the caller handles it
/// by provisioning nothing. A store that is DOWN is an outage the operator
/// has to see. Collapsing the two would make a board that lost its
/// connection look like a board losing every race.
#[tokio::test]
async fn a_refused_request_is_an_answer_and_an_outage_is_an_error() {
    use crate::store::{DispatchRequestStore, RequestDispatchAnswer};
    let ask = |db: Store| async move {
        db.request_dispatch(TaskId(1), crate::models::DispatchOrigin::User, None)
            .await
    };

    let (winner, _) = writer_with(RecordingCaller::returning(&[5]));
    assert_eq!(
        ask(winner).await.unwrap(),
        RequestDispatchAnswer::Requested(crate::models::DispatchRequestId(5)),
        "an applied request reads its row's id back"
    );

    let (loser, _) = writer_with(RecordingCaller::rejecting());
    assert!(
        matches!(ask(loser).await.unwrap(), RequestDispatchAnswer::Refused(_)),
        "a refused request is an answer — somebody else got there first"
    );

    let (offline, _) = writer_with(RecordingCaller::refusing("store unreachable"));
    assert!(
        ask(offline).await.is_err(),
        "a request with the store down must fail rather than report a lost race"
    );
}

/// ...and everywhere else a refusal IS an error. A patch the store declined did
/// not happen, and the caller has to know.
#[tokio::test]
async fn a_refused_patch_is_an_error() {
    let (writer, _) = writer_with(RecordingCaller::rejecting());

    let refused = writer
        .patch_task(TaskId(1), &TaskPatch::new().title("t"))
        .await;

    assert!(refused.is_err(), "a declined patch must not report success");
}

/// The same asymmetry on unsubscribing, where the spec asks for it explicitly:
/// unfollowing something unfollowed is a refusal at the store and `Ok(false)`
/// here (`sync.allium: UnsubscribeFromEpic`).
#[tokio::test]
async fn unsubscribing_from_something_unfollowed_is_false_not_an_error() {
    let (writer, _) = writer_with(RecordingCaller::rejecting());

    assert!(!writer
        .unsubscribe_from_epic("user-me", EpicId(3))
        .await
        .unwrap());
}

// -- The rest of the routed surface -----------------------------------------

/// Moving a task out of its epic gives it an owner; moving it in takes the
/// owner away. `core.allium: OwnerTracksUserBoardTask` — the store enforces
/// both arms, and this is the name it needs to do so.
#[tokio::test]
async fn leaving_an_epic_carries_the_owner_and_joining_one_does_not() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.set_task_epic_id(TaskId(1), None).await.unwrap();
    writer
        .set_task_epic_id(TaskId(2), Some(EpicId(4)))
        .await
        .unwrap();

    assert_eq!(
        caller.sent(),
        vec![
            Sent::SetTaskEpic(TaskId(1), 0, "user-me".into()),
            Sent::SetTaskEpic(TaskId(2), 4, String::new()),
        ]
    );
}

/// Clearing a verify command sends the store's absent sentinel, not a command
/// that happens to be empty.
#[tokio::test]
async fn clearing_a_verify_command_sends_the_absent_sentinel() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.set_verify_command("/repo", None).await.unwrap();
    writer
        .set_verify_command("/repo", Some("cargo test"))
        .await
        .unwrap();

    assert_eq!(
        caller.sent(),
        vec![
            Sent::SetVerifyCommand("/repo".into(), String::new()),
            Sent::SetVerifyCommand("/repo".into(), "cargo test".into()),
        ]
    );
}

/// A new epic is born in backlog, not auto-dispatching and manual —
/// `epics.allium: CreateEpic`. Those are the epic's birth state rather than
/// defaults somebody forgot, and they are set here so the store has no opinion
/// about what a new epic looks like.
#[tokio::test]
async fn a_new_epic_is_born_in_backlog() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    let created = writer.create_epic("New", "why", None).await.unwrap();

    let Some(Sent::CreateEpic(row)) = caller.sent().into_iter().next() else {
        panic!("expected a create");
    };
    assert_eq!(row.status, "backlog");
    assert!(!row.auto_dispatch);
    assert_eq!(row.origin, "manual");
    assert_eq!(row.parent_epic_id, 0);
    // And the caller gets back the row it sent, with the store's id on it.
    assert_eq!(created.id, EpicId(7));
    assert_eq!(created.title, "New");
    assert_eq!(created.status, TaskStatus::Backlog);
}

/// An epic born with a sort order and a feed is ONE write: the fields ride in
/// the create row, so a failure cannot leave a half-configured epic behind and
/// a retry cannot duplicate it.
#[tokio::test]
async fn an_epic_born_with_feed_fields_is_one_write() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    let created = writer
        .create_epic_configured(
            "Feed",
            "why",
            None,
            &crate::store::EpicBirth {
                sort_order: Some(42),
                feed_command: Some("gh api x"),
                feed_interval_secs: Some(300),
            },
        )
        .await
        .unwrap();

    let sent = caller.sent();
    assert_eq!(sent.len(), 1, "expected a single write, got {sent:?}");
    let Sent::CreateEpic(row) = &sent[0] else {
        panic!("expected a create");
    };
    assert_eq!(row.sort_order, Some(42));
    assert_eq!(row.feed_command, "gh api x");
    assert_eq!(row.feed_interval_secs, 300);
    assert_eq!(created.sort_order, Some(42));
    assert_eq!(created.feed_command.as_deref(), Some("gh api x"));
    assert_eq!(created.feed_interval_secs, Some(300));
}

/// An epic create carries the creator's identity, the same as a task's does —
/// `core.allium: Epic.created_by`. Unlike a task, an epic has no `owner` field
/// at all, so this is the ONLY way `sync.allium`'s `own_creations`
/// subscription can find an epic its creator just made.
#[tokio::test]
async fn an_epic_create_carries_the_creator() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.create_epic("New", "why", None).await.unwrap();

    let Some(Sent::CreateEpic(row)) = caller.sent().into_iter().next() else {
        panic!("expected a create");
    };
    assert_eq!(row.created_by, "user-me");
}

/// A board with no settled identity cannot create an epic either —
/// `sync.allium: CreatesRequireASettledIdentity`. An epic has no `owner` to
/// fall back on the way a task does, so this is the only guard standing
/// between a create and a `created_by` nobody can name.
#[tokio::test]
async fn a_board_with_no_identity_cannot_create_an_epic() {
    let (writer, caller) = writer_with_no_identity(RecordingCaller::default());

    let refused = writer.create_epic("New", "why", None).await;

    assert!(refused.is_err(), "an epic create needs a name to stamp");
    assert!(caller.sent().is_empty());
}

/// A work package create carries the creator, a trimmed title and this
/// board's clock — `work-packages.allium: CreateWorkPackage`. The board
/// subscribes to packages by `created_by`, so the name is what makes the new
/// package visible at all.
#[tokio::test]
async fn a_work_package_create_carries_the_creator_and_a_trimmed_title() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    let id = writer
        .create_work_package(
            "  Window 1 ",
            "the plan",
            &[WorkPackageItem::Task(TaskId(4))],
        )
        .await
        .unwrap();

    let Some(Sent::CreateWorkPackageWithLinks(row, links)) = caller.sent().into_iter().next()
    else {
        panic!("expected a create");
    };
    assert_eq!(links, vec![("task".to_string(), 4)]);
    assert_eq!(id, WorkPackageId(9));
    assert_eq!(row.title, "Window 1");
    assert_eq!(row.description, "the plan");
    assert_eq!(row.created_by, "user-me");
    assert!(!row.created_at.is_empty());
    assert_eq!(row.created_at, row.updated_at);
}

/// `sync.allium: CreatesRequireASettledIdentity`, for packages.
#[tokio::test]
async fn a_board_with_no_identity_cannot_create_a_work_package() {
    let (writer, caller) = writer_with_no_identity(RecordingCaller::default());

    assert!(writer.create_work_package("W", "", &[]).await.is_err());
    assert!(caller.sent().is_empty());
}

/// A patch naming nothing never reaches the transport.
#[tokio::test]
async fn an_empty_work_package_patch_sends_nothing() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer
        .patch_work_package(WorkPackageId(1), None, None, None)
        .await
        .unwrap();

    assert!(caller.sent().is_empty());
}
