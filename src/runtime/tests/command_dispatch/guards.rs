use super::*;

// -----------------------------------------------------------------------
// Every variant reaches the dispatcher
// -----------------------------------------------------------------------

/// The dispatcher's top-level match has one arm per `Command` sub-enum, and
/// a mis-wire between two of them compiles. The tests above drive every
/// variant through `commands::dispatch`; this one is the *inventory* that
/// keeps that true — a new sub-enum added to `Command` fails to compile
/// here until it is listed, which is the prompt to write its test.
///
/// Deliberately not an assertion on the match's shape: it builds one value
/// per sub-enum and lets exhaustiveness do the work.
#[test]
fn every_command_sub_enum_is_named_by_this_module() {
    fn sub_enum_name(cmd: &Command) -> &'static str {
        match cmd {
            Command::Task(_) => "Task",
            Command::Editor(_) => "Editor",
            Command::Feed(_) => "Feed",
            Command::Settings(_) => "Settings",
            Command::Epic(_) => "Epic",
            Command::System(_) => "System",
            Command::RepoFilter(_) => "RepoFilter",
            Command::Pr(_) => "Pr",
            Command::Split(_) => "Split",
            Command::Learning(_) => "Learning",
            Command::Usage(_) => "Usage",
            Command::Budget(_) => "Budget",
            Command::KeepAwake(_) => "KeepAwake",
            Command::WorkPackage(_) => "WorkPackage",
            Command::People(_) => "People",
        }
    }

    // One representative per sub-enum. Every name below is driven through
    // `commands::dispatch` by a test in this module.
    let covered = [
        Command::Task(TaskCommand::RefreshFromDb),
        Command::Editor(EditorCommand::PopOut(crate::tui::EditKind::Description {
            is_epic: false,
        })),
        Command::Feed(FeedCommand::TriggerEpic {
            epic_id: crate::models::EpicId(1),
            epic_title: String::new(),
        }),
        Command::Settings(SettingsCommand::SaveRepoPath(String::new())),
        Command::Epic(crate::tui::commands::EpicCommand::RefreshFromDb),
        Command::System(SystemCommand::OpenInBrowser { url: String::new() }),
        Command::RepoFilter(crate::tui::commands::RepoFilterCommand::DeleteRepoPath(
            String::new(),
        )),
        Command::Pr(PrCommand::CheckStatus {
            id: TaskId(1),
            url: String::new(),
        }),
        Command::Split(SplitCommand::Enter),
        Command::Learning(crate::tui::commands::LearningCommand::ArchiveStale),
        Command::Usage(crate::tui::commands::UsageCommand::Record(
            crate::models::UsageEvent {
                category: crate::models::UsageCategory::Keybinding,
                action: String::new(),
                detail: None,
                actor: crate::models::UsageActor::Human,
            },
        )),
        Command::Budget(crate::tui::commands::BudgetCommand::Refresh),
        Command::KeepAwake(crate::tui::commands::KeepAwakeCommand::Release),
        Command::WorkPackage(crate::tui::commands::WorkPackageCommand::Delete(
            models::WorkPackageId(1),
        )),
        Command::People(crate::tui::commands::PeopleCommand::SetDisplayName(
            String::new(),
        )),
    ];

    let mut names: Vec<&str> = covered.iter().map(sub_enum_name).collect();
    names.sort_unstable();
    let before = names.len();
    names.dedup();
    assert_eq!(
        names.len(),
        before,
        "each sub-enum needs exactly one representative, got {names:?}"
    );
}

#[tokio::test]
async fn a_guarded_cleanup_of_a_task_not_done_in_the_store_runs_no_teardown() {
    // The board believed the task was done; the store knows it is Running.
    let mut h = quiet_harness().await;
    let task = seed(&h.rt, "Reopened", models::TaskStatus::Running).await;

    h.dispatch(Command::Task(TaskCommand::Cleanup {
        id: task.id,
        repo_path: "/repo".into(),
        worktree: Some("/repo/.worktrees/1-reopened".into()),
        tmux_window: None,
        follow_up: crate::tui::commands::CleanupFollowUp::DeleteRow,
        guard: Some(crate::tui::commands::DeleteGuard::Task(task.id)),
    }))
    .await;

    let msg = h.next_msg().await;
    assert!(refused(&msg), "got: {msg:?}");
    assert!(
        h.mock.recorded_calls().is_empty(),
        "no git or tmux command may run: {:?}",
        h.mock.recorded_calls()
    );
    assert!(h.rt.database.get_task(task.id).await.is_some());
}

#[tokio::test]
async fn a_guarded_cleanup_of_a_done_task_still_tears_down() {
    let mut h = harness(MockProcessRunner::new(vec![
        MockProcessRunner::ok(),
        MockProcessRunner::ok(),
    ]))
    .await;
    let task = seed(&h.rt, "Done", models::TaskStatus::Done).await;

    h.dispatch(Command::Task(TaskCommand::Cleanup {
        id: task.id,
        repo_path: "/repo".into(),
        worktree: Some("/repo/.worktrees/1-done".into()),
        tmux_window: None,
        follow_up: crate::tui::commands::CleanupFollowUp::DeleteRow,
        guard: Some(crate::tui::commands::DeleteGuard::Task(task.id)),
    }))
    .await;

    let msg = h.next_msg().await;
    assert!(
        matches!(
            msg,
            Message::Task(crate::tui::messages::TaskMessage::CleanupSucceeded { .. })
        ),
        "got: {msg:?}"
    );
}

#[tokio::test]
async fn a_guarded_epic_teardown_is_refused_when_a_subtree_task_is_not_done() {
    let mut h = quiet_harness().await;
    let epic = h.rt.db_write().create_epic("E", "", None).await.unwrap();
    let task = seed(&h.rt, "Live", models::TaskStatus::Running).await;
    h.rt.db_write()
        .set_task_epic_id(task.id, Some(epic.id))
        .await
        .unwrap();

    h.dispatch(Command::Task(TaskCommand::Cleanup {
        id: task.id,
        repo_path: "/repo".into(),
        worktree: Some("/repo/.worktrees/1-live".into()),
        tmux_window: None,
        follow_up: crate::tui::commands::CleanupFollowUp::Nothing,
        guard: Some(crate::tui::commands::DeleteGuard::Epic(epic.id)),
    }))
    .await;

    assert!(refused(&h.next_msg().await));
    assert!(h.mock.recorded_calls().is_empty());
}

#[tokio::test]
async fn deleting_a_task_that_is_not_done_in_the_store_changes_nothing() {
    let mut h = quiet_harness().await;
    let task = seed(&h.rt, "Reopened", models::TaskStatus::Backlog).await;

    h.dispatch(Command::Task(TaskCommand::Delete(task.id)))
        .await;

    assert!(refused(&h.next_msg().await));
    assert!(h.rt.database.get_task(task.id).await.is_some());
}

#[tokio::test]
async fn batch_deleting_with_one_item_not_done_in_the_store_changes_nothing() {
    let mut h = quiet_harness().await;
    let done = seed(&h.rt, "Done", models::TaskStatus::Done).await;
    let live = seed(&h.rt, "Live", models::TaskStatus::Backlog).await;

    h.dispatch(Command::Task(TaskCommand::BatchDelete {
        task_ids: vec![done.id, live.id],
        epic_ids: vec![],
        teardowns: vec![],
    }))
    .await;

    assert!(refused(&h.next_msg().await));
    assert!(h.rt.database.get_task(done.id).await.is_some());
    assert!(h.rt.database.get_task(live.id).await.is_some());
}

// -----------------------------------------------------------------------
// The pre-check reads the store, not the board's subscription replica
// -----------------------------------------------------------------------
//
// `tasks.allium: DeleteTask` ("ENFORCED THREE TIMES"): a task reopened on
// another board still reads as done here until the row reaches this board.
// Every case below plants that disagreement: the board's own rows say `done`,
// the store's say not.

/// A harness whose board copy lags the store.
async fn lagging_harness(script: Vec<anyhow::Result<std::process::Output>>) -> Harness {
    let db = Store::open_with_lagging_board().unwrap();
    harness_over(Arc::new(db), MockProcessRunner::new(script)).await
}

fn guarded_cleanup(id: models::TaskId, guard: crate::tui::commands::DeleteGuard) -> Command {
    Command::Task(TaskCommand::Cleanup {
        id,
        repo_path: "/repo".into(),
        worktree: Some(format!("/repo/.worktrees/{}-lagging", id.0)),
        tmux_window: None,
        follow_up: crate::tui::commands::CleanupFollowUp::DeleteRow,
        guard: Some(guard),
    })
}

#[tokio::test]
async fn a_guarded_cleanup_runs_no_teardown_when_the_store_says_not_done_but_the_board_says_done() {
    let mut h = lagging_harness(vec![]).await;
    let id = h.db.seed_lagging_task(None, "done", "running").await;

    h.dispatch(guarded_cleanup(
        id,
        crate::tui::commands::DeleteGuard::Task(id),
    ))
    .await;

    let msg = h.next_msg().await;
    assert!(refused(&msg), "got: {msg:?}");
    assert!(
        h.mock.recorded_calls().is_empty(),
        "no git or tmux command may run: {:?}",
        h.mock.recorded_calls()
    );
}

#[tokio::test]
async fn a_guarded_epic_cleanup_runs_no_teardown_when_the_store_holds_a_live_subtree_task() {
    let mut h = lagging_harness(vec![]).await;
    let epic = h.db.seed_lagging_epic().await;
    let id = h.db.seed_lagging_task(Some(epic), "done", "running").await;

    h.dispatch(guarded_cleanup(
        id,
        crate::tui::commands::DeleteGuard::Epic(epic),
    ))
    .await;

    assert!(refused(&h.next_msg().await));
    assert!(h.mock.recorded_calls().is_empty());
}

#[tokio::test]
async fn a_guarded_cleanup_still_tears_down_when_the_store_says_done_but_the_board_says_not() {
    // The board lags the other way: the task was finished elsewhere. The TRUE
    // status governs, so the delete goes ahead.
    let mut h = lagging_harness(vec![MockProcessRunner::ok(), MockProcessRunner::ok()]).await;
    let id = h.db.seed_lagging_task(None, "running", "done").await;

    h.dispatch(guarded_cleanup(
        id,
        crate::tui::commands::DeleteGuard::Task(id),
    ))
    .await;

    let msg = h.next_msg().await;
    assert!(
        matches!(
            msg,
            Message::Task(crate::tui::messages::TaskMessage::CleanupSucceeded { .. })
        ),
        "got: {msg:?}"
    );
}

#[tokio::test]
async fn deleting_a_task_the_store_says_is_not_done_changes_nothing() {
    let mut h = lagging_harness(vec![]).await;
    let id = h.db.seed_lagging_task(None, "done", "backlog").await;

    h.dispatch(Command::Task(TaskCommand::Delete(id))).await;

    assert!(refused(&h.next_msg().await));
    assert!(h.mock.recorded_calls().is_empty());
}

#[tokio::test]
async fn batch_deleting_with_an_item_the_store_says_is_not_done_changes_nothing() {
    let mut h = lagging_harness(vec![]).await;
    let id = h.db.seed_lagging_task(None, "done", "running").await;

    h.dispatch(Command::Task(TaskCommand::BatchDelete {
        task_ids: vec![id],
        epic_ids: vec![],
        teardowns: vec![],
    }))
    .await;

    assert!(refused(&h.next_msg().await));
}

#[tokio::test]
async fn deleting_an_epic_the_store_says_holds_a_live_task_changes_nothing() {
    let mut h = lagging_harness(vec![]).await;
    let epic = h.db.seed_lagging_epic().await;
    h.db.seed_lagging_task(Some(epic), "done", "running").await;

    h.dispatch(Command::Epic(crate::tui::commands::EpicCommand::Delete {
        id: epic,
        teardowns: vec![],
    }))
    .await;

    assert!(refused(&h.next_msg().await));
}

#[tokio::test]
async fn a_guarded_cleanup_runs_no_teardown_when_the_store_cannot_be_asked() {
    // Only a real answer from the store may let a delete through: an outage
    // is a refusal, never a pass.
    let mut h = lagging_harness(vec![]).await;
    let id = h.db.seed_lagging_task(None, "done", "done").await;
    h.db.fail_next_delete_check();

    h.dispatch(guarded_cleanup(
        id,
        crate::tui::commands::DeleteGuard::Task(id),
    ))
    .await;

    assert!(refused(&h.next_msg().await));
    assert!(h.mock.recorded_calls().is_empty());
}

// -----------------------------------------------------------------------
// BatchDelete: one pre-check before any teardown; teardowns serial per repo
// -----------------------------------------------------------------------

fn target(id: i64, repo: &str) -> crate::tui::commands::TeardownTarget {
    crate::tui::commands::TeardownTarget {
        id: models::TaskId(id),
        repo_path: repo.into(),
        worktree: Some(format!("{repo}/.worktrees/{id}-t")),
        tmux_window: None,
    }
}

#[tokio::test]
async fn a_refused_batch_starts_no_teardown_for_any_item() {
    let mut h = lagging_harness(vec![]).await;
    let ok = h.db.seed_lagging_task(None, "done", "done").await;
    let bad = h.db.seed_lagging_task(None, "done", "running").await;

    h.dispatch(Command::Task(TaskCommand::BatchDelete {
        task_ids: vec![ok, bad],
        epic_ids: vec![],
        teardowns: vec![target(ok.0, "/repo"), target(bad.0, "/repo")],
    }))
    .await;

    assert!(refused(&h.next_msg().await));
    assert!(h.mock.recorded_calls().is_empty(), "nothing torn down");
}

#[tokio::test]
async fn a_batch_that_goes_through_tears_down_after_the_delete() {
    // Delete first, tear down from what the delete removed: the teardown runs
    // only once the row is gone.
    let mut h = harness(MockProcessRunner::new(vec![
        MockProcessRunner::ok(),
        MockProcessRunner::ok(),
    ]))
    .await;
    let a = seed(&h.rt, "A", models::TaskStatus::Done).await;

    h.dispatch(Command::Task(TaskCommand::BatchDelete {
        task_ids: vec![a.id],
        epic_ids: vec![],
        teardowns: vec![target(a.id.0, "/repo")],
    }))
    .await;

    assert!(matches!(
        h.next_msg().await,
        Message::Task(crate::tui::messages::TaskMessage::BatchDeleted)
    ));
    assert!(h.rt.database.get_task(a.id).await.is_none());
    assert!(matches!(
        h.next_msg().await,
        Message::Task(crate::tui::messages::TaskMessage::CleanupSucceeded { .. })
    ));
}

#[test]
fn a_repo_is_one_queue_however_its_path_is_spelled() {
    use crate::runtime::tasks::repo_queue_key;
    let home = Some("/home/u");
    assert_eq!(
        repo_queue_key("~/x", home),
        repo_queue_key("/home/u/x", home)
    );
    assert_eq!(
        repo_queue_key("/home/u/x/", home),
        repo_queue_key("/home/u/x", home)
    );
    assert_ne!(repo_queue_key("/a", home), repo_queue_key("/b", home));
}

#[tokio::test]
async fn a_batch_that_goes_through_reports_it_so_the_selection_can_clear() {
    let mut h = quiet_harness().await;
    let a = seed(&h.rt, "A", models::TaskStatus::Done).await;

    h.dispatch(Command::Task(TaskCommand::BatchDelete {
        task_ids: vec![a.id],
        epic_ids: vec![],
        teardowns: vec![],
    }))
    .await;

    assert!(matches!(
        h.next_msg().await,
        Message::Task(crate::tui::messages::TaskMessage::BatchDeleted)
    ));
    assert!(h.rt.database.get_task(a.id).await.is_none());
}

#[tokio::test]
async fn batch_teardowns_run_one_repo_at_a_time_in_order() {
    let ok = MockProcessRunner::ok;
    let h = harness(MockProcessRunner::new(vec![
        ok(),
        ok(),
        ok(),
        ok(),
        ok(),
        ok(),
    ]))
    .await;

    let handles = h.rt.exec_cleanup_queues(vec![
        target(1, "/repoA"),
        target(2, "/repoA"),
        target(3, "/repoB"),
    ]);
    assert_eq!(handles.len(), 2, "one queue per repo");
    for handle in handles {
        handle.await.unwrap();
    }

    let calls = h.mock.flattened_calls();
    let pos = |needle: &str| {
        calls
            .iter()
            .position(|c| c.contains(needle) && c.contains("worktree remove"))
    };
    assert!(
        pos("/repoA/.worktrees/1-t").unwrap() < pos("/repoA/.worktrees/2-t").unwrap(),
        "a repo's removals run in queue order, got {calls:?}"
    );
    assert!(pos("/repoB/.worktrees/3-t").is_some());
}

#[tokio::test]
async fn a_refused_epic_delete_starts_no_teardown_for_any_subtask() {
    let mut h = lagging_harness(vec![]).await;
    let epic = h.db.seed_lagging_epic().await;
    let ok = h.db.seed_lagging_task(Some(epic), "done", "done").await;
    let bad = h.db.seed_lagging_task(Some(epic), "done", "running").await;

    h.dispatch(Command::Epic(crate::tui::commands::EpicCommand::Delete {
        id: epic,
        teardowns: vec![target(ok.0, "/repo"), target(bad.0, "/repo")],
    }))
    .await;

    assert!(refused(&h.next_msg().await));
    assert!(h.mock.recorded_calls().is_empty(), "nothing torn down");
}
