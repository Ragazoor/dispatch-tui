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
            Command::RepoSync(_) => "RepoSync",
            Command::Pr(_) => "Pr",
            Command::Split(_) => "Split",
            Command::Learning(_) => "Learning",
            Command::Usage(_) => "Usage",
            Command::Budget(_) => "Budget",
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
        Command::RepoSync(RepoSyncCommand::Refresh {
            repo_path: String::new(),
            fetch_first: false,
        }),
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
async fn a_batch_guard_refuses_the_whole_batch_when_one_item_is_not_done() {
    let mut h = quiet_harness().await;
    let done = seed(&h.rt, "Done", models::TaskStatus::Done).await;
    let live = seed(&h.rt, "Live", models::TaskStatus::Running).await;

    h.dispatch(Command::Task(TaskCommand::Cleanup {
        id: done.id,
        repo_path: "/repo".into(),
        worktree: Some("/repo/.worktrees/1-done".into()),
        tmux_window: None,
        follow_up: crate::tui::commands::CleanupFollowUp::Nothing,
        guard: Some(crate::tui::commands::DeleteGuard::Batch {
            task_ids: vec![done.id, live.id],
            epic_ids: vec![],
        }),
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
    }))
    .await;

    assert!(refused(&h.next_msg().await));
    assert!(h.rt.database.get_task(done.id).await.is_some());
    assert!(h.rt.database.get_task(live.id).await.is_some());
}
