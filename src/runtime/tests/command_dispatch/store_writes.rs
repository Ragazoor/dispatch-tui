use super::*;

/// Run one command through the real dispatcher and return its follow-on
/// commands (the vec `execute_commands` extends its queue with).
async fn dispatch_one(rt: &TuiRuntime, app: &mut App, cmd: Command) -> Vec<Command> {
    commands::dispatch(cmd, app, rt).await
}

#[tokio::test]
async fn dispatch_save_repo_path_persists_and_updates_app() {
    let (rt, mut app) = test_runtime().await;

    let extra = dispatch_one(
        &rt,
        &mut app,
        Command::Settings(SettingsCommand::SaveRepoPath("/some/repo".into())),
    )
    .await;

    assert!(
        extra.is_empty(),
        "SaveRepoPath produces no follow-on commands"
    );
    assert!(
        rt.database
            .list_repo_paths()
            .await
            .contains(&"/some/repo".to_string()),
        "repo path should be persisted"
    );
    assert!(
        app.repo_paths().contains(&"/some/repo".to_string()),
        "App should have been refreshed with the new path"
    );
}

#[tokio::test]
async fn dispatch_save_base_branch_persists_and_updates_app() {
    let (rt, mut app) = test_runtime().await;

    dispatch_one(
        &rt,
        &mut app,
        Command::Settings(SettingsCommand::SaveBaseBranch(
            "/some/repo".into(),
            "develop".into(),
        )),
    )
    .await;

    let pairs = rt.database.list_all_base_branches().await;
    assert!(
        pairs
            .iter()
            .any(|(repo, branch)| repo == "/some/repo" && branch == "develop"),
        "base branch should be persisted, got {pairs:?}"
    );
}

#[tokio::test]
async fn dispatch_task_persist_writes_status_to_db() {
    let (rt, mut app) = test_runtime().await;
    let mut task = seed(&rt, "Persist me", models::TaskStatus::Backlog).await;
    // Mirror what every production status move does: the sub-status travels
    // with the status (`SubStatus::default_for`), because the store
    // rejects any (status, sub_status) pair the module's
    // `validate_sub_status` disallows.
    task.status = models::TaskStatus::Running;
    task.sub_status = models::SubStatus::default_for(models::TaskStatus::Running);
    task.worktree = Some("/wt".into());

    dispatch_one(
        &rt,
        &mut app,
        Command::Task(TaskCommand::Persist(PersistFields::from_task(&task))),
    )
    .await;

    let stored = rt.database.get_task(task.id).await.unwrap();
    assert_eq!(stored.status, models::TaskStatus::Running);
    assert_eq!(stored.sub_status, models::SubStatus::Active);
    assert_eq!(stored.worktree.as_deref(), Some("/wt"));
    assert!(
        app.dirty_since_refresh,
        "a successful persist must mark the board dirty so the next tick refreshes"
    );
}

#[tokio::test]
async fn dispatch_task_persist_surfaces_service_rejection_and_writes_nothing() {
    let (rt, mut app) = test_runtime().await;
    let mut task = seed(&rt, "Persist me", models::TaskStatus::Backlog).await;
    // Running + SubStatus::None is rejected by the module's `validate_sub_status`, so
    // the whole patch is refused — including the worktree field.
    task.status = models::TaskStatus::Running;
    task.worktree = Some("/wt".into());

    dispatch_one(
        &rt,
        &mut app,
        Command::Task(TaskCommand::Persist(PersistFields::from_task(&task))),
    )
    .await;

    let stored = rt.database.get_task(task.id).await.unwrap();
    assert_eq!(stored.status, models::TaskStatus::Backlog);
    assert_eq!(stored.worktree, None);
    let err = app.error_popup().unwrap_or_default();
    assert!(
        err.contains("persisting task"),
        "the rejection must reach the user as an error popup, got {err:?}"
    );
}

#[tokio::test]
async fn dispatch_task_delete_removes_the_row() {
    let (rt, mut app) = test_runtime().await;
    let task = seed(&rt, "Delete me", models::TaskStatus::Done).await;

    dispatch_one(&rt, &mut app, Command::Task(TaskCommand::Delete(task.id))).await;

    assert!(
        rt.database.get_task(task.id).await.is_none(),
        "the task row should be gone"
    );
}

/// `ClearWorktreePointer` is the write a *successful* teardown earns: it is the
/// only thing that clears the worktree column on the retry-fresh path.
#[tokio::test]
async fn dispatch_task_clear_worktree_pointer_clears_both_pointers() {
    let (rt, mut app) = test_runtime().await;
    let task = seed(&rt, "Torn down", models::TaskStatus::Running).await;
    rt.db_write().give_agent(task.id);
    rt.db_write()
        .patch_task(
            task.id,
            &store::TaskPatch::new()
                .worktree(Some("/repo/.worktrees/1-torn-down"))
                .tmux_window(Some(&test_tmux_window("task-1")))
                .host(Some("this-machine")),
        )
        .await
        .unwrap();

    dispatch_one(
        &rt,
        &mut app,
        Command::Task(TaskCommand::ClearWorktreePointer(task.id)),
    )
    .await;

    let stored = rt.database.get_task(task.id).await.unwrap();
    assert_eq!(stored.worktree, None);
    assert_eq!(stored.tmux_window, None);
    // Paired with `worktree` per core/Task's `HostTracksWorktree` invariant
    // (docs/specs/core.allium): this is the write that forgets the worktree,
    // so it owes the clear (`RetryFresh` in docs/specs/dispatch.allium).
    assert_eq!(stored.host, None);
}

#[tokio::test]
async fn dispatch_task_move_to_epic_reparents_and_cascades_a_refresh() {
    let (rt, mut app) = test_runtime().await;
    let task = seed(&rt, "Adopt me", models::TaskStatus::Backlog).await;
    let epic_id = rt
        .db_write()
        .create_epic("Parent epic", "desc", None)
        .await
        .unwrap()
        .id;

    let extra = dispatch_one(
        &rt,
        &mut app,
        Command::Task(TaskCommand::MoveToEpic {
            id: task.id,
            new_epic: Some(epic_id),
        }),
    )
    .await;

    let stored = rt.database.get_task(task.id).await.unwrap();
    assert_eq!(
        stored.epic_id,
        Some(epic_id),
        "the task should now belong to the epic"
    );
    // MoveToEpic chains into exec_refresh_from_db, so App must see the move
    // without any further command being dispatched by the caller.
    assert!(
        app.tasks().iter().any(|t| t.id == task.id),
        "the refreshed board should contain the moved task"
    );
    // Any follow-on commands the refresh produced must be drainable, not
    // silently dropped.
    for cmd in extra {
        drain(&rt, &mut app, cmd).await;
    }
}

#[tokio::test]
async fn dispatch_task_refresh_from_db_syncs_app_from_db() {
    let (rt, mut app) = test_runtime().await;
    assert!(app.tasks().is_empty(), "precondition: empty board");
    let task = seed(
        &rt,
        "Written behind App's back",
        models::TaskStatus::Backlog,
    )
    .await;

    dispatch_one(&rt, &mut app, Command::Task(TaskCommand::RefreshFromDb)).await;

    assert_eq!(app.tasks().len(), 1);
    assert_eq!(app.tasks()[0].id, task.id);
}

#[tokio::test]
async fn dispatch_task_patch_sub_status_writes_to_db() {
    let (rt, mut app) = test_runtime().await;
    let task = seed(&rt, "Patch me", models::TaskStatus::Running).await;

    dispatch_one(
        &rt,
        &mut app,
        Command::Task(TaskCommand::PatchSubStatus {
            id: task.id,
            sub_status: models::SubStatus::NeedsInput,
        }),
    )
    .await;

    let stored = rt.database.get_task(task.id).await.unwrap();
    assert_eq!(stored.sub_status, models::SubStatus::NeedsInput);
}

#[tokio::test]
async fn dispatch_epic_insert_then_persist_writes_to_db() {
    let (rt, mut app) = test_runtime().await;

    dispatch_one(
        &rt,
        &mut app,
        Command::Epic(crate::tui::commands::EpicCommand::Insert(tui::EpicDraft {
            title: "Epic via dispatch".into(),
            description: "desc".into(),
            parent_epic_id: None,
            ..Default::default()
        })),
    )
    .await;

    let epics = rt.database.list_epics().await;
    assert_eq!(epics.len(), 1);
    assert_eq!(epics[0].title, "Epic via dispatch");
}

/// `work-packages.allium`: the board's package writes reach the store through
/// the service, and the board re-reads its packages after each one.
#[tokio::test]
async fn dispatch_work_package_commands_write_and_refresh_the_board() {
    use crate::tui::commands::WorkPackageCommand as W;
    let (rt, mut app) = test_runtime().await;
    let task = seed(&rt, "linked", models::TaskStatus::Backlog).await;
    let item = models::WorkPackageItem::Task(task.id);

    dispatch_one(
        &rt,
        &mut app,
        Command::WorkPackage(W::Create {
            title: "Window 1".into(),
            item,
            workspace_id: 0,
            visibility: models::Visibility::Public,
        }),
    )
    .await;
    let packages = rt.database.list_work_packages().await;
    assert_eq!(packages.len(), 1);
    let id = packages[0].id;
    assert_eq!(
        app.work_packages(),
        packages.as_slice(),
        "the board re-read its packages"
    );
    assert_eq!(
        app.work_package_links(),
        [models::WorkPackageLink {
            work_package_id: id,
            item
        }]
    );

    dispatch_one(&rt, &mut app, Command::WorkPackage(W::Unlink { id, item })).await;
    assert!(rt.database.list_work_package_links().await.is_empty());
    dispatch_one(&rt, &mut app, Command::WorkPackage(W::Link { id, item })).await;
    assert_eq!(rt.database.list_work_package_links().await.len(), 1);

    dispatch_one(
        &rt,
        &mut app,
        Command::WorkPackage(W::Rename {
            id,
            title: "Window 2".into(),
        }),
    )
    .await;
    assert_eq!(rt.database.list_work_packages().await[0].title, "Window 2");

    dispatch_one(&rt, &mut app, Command::WorkPackage(W::Delete(id))).await;
    assert!(rt.database.list_work_packages().await.is_empty());
    assert!(app.work_packages().is_empty());
    assert!(
        rt.database.get_task(task.id).await.is_some(),
        "deleting a package keeps its tasks"
    );
}

/// `work-packages.allium: CreateTaskInPackageView`: a task drafted in a
/// package view is created with no epic, then linked to the package, and the
/// board re-reads its packages.
#[tokio::test]
async fn an_insert_drafted_in_a_package_view_links_the_new_task() {
    let (rt, mut app) = test_runtime().await;
    let package = rt
        .work_package_svc
        .create_work_package(crate::service::CreateWorkPackageParams {
            title: "Window 1".into(),
            description: String::new(),
            link: vec![],
            ..Default::default()
        })
        .await
        .expect("package created");

    dispatch_one(
        &rt,
        &mut app,
        Command::Task(crate::tui::commands::TaskCommand::Insert {
            draft: tui::TaskDraft {
                title: "in package".into(),
                repo_path: "/repo".into(),
                work_package: Some(package),
                ..Default::default()
            },
            epic_id: None,
        }),
    )
    .await;

    let task = rt
        .database
        .list_all()
        .await
        .into_iter()
        .find(|t| t.title == "in package")
        .expect("task created");
    assert_eq!(task.epic_id, None);
    assert_eq!(
        app.work_package_links(),
        [models::WorkPackageLink {
            work_package_id: package,
            item: models::WorkPackageItem::Task(task.id),
        }]
    );
}

/// A refused link keeps the created task and shows the error.
#[tokio::test]
async fn an_insert_whose_link_is_refused_keeps_the_task_and_surfaces_an_error() {
    let (rt, mut app) = test_runtime().await;
    dispatch_one(
        &rt,
        &mut app,
        Command::Task(crate::tui::commands::TaskCommand::Insert {
            draft: tui::TaskDraft {
                title: "orphan".into(),
                repo_path: "/repo".into(),
                work_package: Some(models::WorkPackageId(404)),
                ..Default::default()
            },
            epic_id: None,
        }),
    )
    .await;
    assert!(rt
        .database
        .list_all()
        .await
        .iter()
        .any(|t| t.title == "orphan"));
    assert!(app.error_popup().is_some());
}

#[tokio::test]
async fn a_refused_work_package_write_surfaces_an_error() {
    use crate::tui::commands::WorkPackageCommand as W;
    let (rt, mut app) = test_runtime().await;
    dispatch_one(
        &rt,
        &mut app,
        Command::WorkPackage(W::Rename {
            id: models::WorkPackageId(404),
            title: "nope".into(),
        }),
    )
    .await;
    assert!(app.error_popup().is_some());
}

#[tokio::test]
async fn dispatch_persist_setting_writes_both_kinds() {
    let (rt, mut app) = test_runtime().await;

    dispatch_one(
        &rt,
        &mut app,
        Command::Settings(SettingsCommand::PersistSetting {
            key: "notifications_enabled".into(),
            value: true,
        }),
    )
    .await;
    dispatch_one(
        &rt,
        &mut app,
        Command::Settings(SettingsCommand::PersistStringSetting {
            key: "test_string_setting".into(),
            value: "/main".into(),
        }),
    )
    .await;

    assert_eq!(
        rt.database
            .get_setting_bool("notifications_enabled")
            .await
            .unwrap(),
        Some(true)
    );
    assert_eq!(
        rt.database
            .get_setting_string("test_string_setting")
            .await
            .unwrap()
            .as_deref(),
        Some("/main")
    );
}

#[tokio::test]
async fn dispatch_editor_pop_out_refuses_when_a_session_is_already_open() {
    let (rt, mut app) = test_runtime().await;
    let task = seed(&rt, "Already editing", models::TaskStatus::Backlog).await;
    *rt.editor_session.lock().unwrap() = Some(super::editor::EditorSession::occupied_for_test(
        &test_tmux_window("edit-1"),
    ));

    dispatch_one(
        &rt,
        &mut app,
        Command::Editor(EditorCommand::PopOut(crate::tui::EditKind::TaskEdit(
            Box::new(task),
        ))),
    )
    .await;

    assert_eq!(
        app.status_message(),
        Some(super::editor::EDITOR_ALREADY_OPEN_MSG),
        "the guard must surface a status message instead of opening a second editor"
    );
}

#[tokio::test]
async fn dispatch_editor_finalize_result_persists_the_edit() {
    let (rt, mut app) = test_runtime().await;
    let task = seed(&rt, "Old title", models::TaskStatus::Backlog).await;
    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        vec![task.clone()],
    )));

    let edited = "--- TITLE ---\nNew title\n\
            --- DESCRIPTION ---\nNew description\n\
            --- REPO_PATH ---\n\n\
            --- STATUS ---\n\n\
            --- PLAN ---\n\n\
            --- TAG ---\n\n\
            --- BASE_BRANCH ---\n\n";

    drain(
        &rt,
        &mut app,
        Command::Editor(EditorCommand::FinalizeResult {
            kind: crate::tui::EditKind::TaskEdit(Box::new(task.clone())),
            outcome: crate::tui::EditorOutcome::Saved(edited.into()),
        }),
    )
    .await;

    let stored = rt.database.get_task(task.id).await.unwrap();
    assert_eq!(stored.title, "New title");
    assert_eq!(stored.description, "New description");
}

#[tokio::test]
async fn dispatch_task_insert_writes_a_new_row() {
    let (rt, mut app) = test_runtime().await;

    dispatch_one(
        &rt,
        &mut app,
        Command::Task(TaskCommand::Insert {
            draft: tui::TaskDraft {
                title: "Inserted via dispatch".into(),
                description: "desc".into(),
                repo_path: "/repo".into(),
                ..Default::default()
            },
            epic_id: None,
        }),
    )
    .await;

    let tasks = rt.database.list_all().await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "Inserted via dispatch");
    assert_eq!(app.tasks().len(), 1);
}

#[tokio::test]
async fn dispatch_task_seed_activity_stamps_the_hook_column() {
    let (rt, mut app) = test_runtime().await;
    let task = seed(&rt, "Just dispatched", models::TaskStatus::Running).await;
    let at = chrono::Utc::now();

    dispatch_one(
        &rt,
        &mut app,
        Command::Task(TaskCommand::SeedActivity { id: task.id, at }),
    )
    .await;

    let stored = rt.database.get_task(task.id).await.unwrap();
    assert!(
        stored.last_pre_tool_use_at.is_some(),
        "the activity timestamp must be seeded so the task is not immediately Stale"
    );
}

#[tokio::test]
async fn dispatch_task_batch_patch_sub_status_updates_every_task() {
    let (rt, mut app) = test_runtime().await;
    let a = seed(&rt, "A", models::TaskStatus::Running).await;
    let b = seed(&rt, "B", models::TaskStatus::Running).await;

    dispatch_one(
        &rt,
        &mut app,
        Command::Task(TaskCommand::BatchPatchSubStatus {
            updates: vec![
                (a.id, models::SubStatus::Stale),
                (b.id, models::SubStatus::Crashed),
            ],
        }),
    )
    .await;

    assert_eq!(
        rt.database.get_task(a.id).await.unwrap().sub_status,
        models::SubStatus::Stale
    );
    assert_eq!(
        rt.database.get_task(b.id).await.unwrap().sub_status,
        models::SubStatus::Crashed
    );
}

#[tokio::test]
async fn dispatch_epic_persist_delete_and_toggles_reach_the_db() {
    use crate::tui::commands::EpicCommand;
    let (rt, mut app) = test_runtime().await;
    let epic = rt
        .db_write()
        .create_epic("Toggled", "desc", None)
        .await
        .unwrap();

    dispatch_one(
        &rt,
        &mut app,
        Command::Epic(EpicCommand::Persist {
            id: epic.id,
            status: Some(models::TaskStatus::Review),
            sort_order: Some(3),
            completed_at: None,
        }),
    )
    .await;
    dispatch_one(
        &rt,
        &mut app,
        Command::Epic(EpicCommand::ToggleAutoDispatch {
            id: epic.id,
            auto_dispatch: true,
        }),
    )
    .await;
    dispatch_one(
        &rt,
        &mut app,
        Command::Epic(EpicCommand::ToggleGroupByRepo {
            id: epic.id,
            group_by_repo: true,
        }),
    )
    .await;

    let stored = rt.database.get_epic(epic.id).await.unwrap();
    assert_eq!(stored.status, models::TaskStatus::Review);
    assert_eq!(stored.sort_order, Some(3));
    assert!(stored.auto_dispatch);
    assert!(stored.group_by_repo);

    // RefreshFromDb must make the App agree with the DB.
    dispatch_one(&rt, &mut app, Command::Epic(EpicCommand::RefreshFromDb)).await;
    assert!(app.epics().iter().any(|e| e.id == epic.id));

    dispatch_one(&rt, &mut app, Command::Epic(EpicCommand::Delete(epic.id))).await;
    assert!(rt.database.get_epic(epic.id).await.is_none());
}

#[tokio::test]
async fn dispatch_epic_reparent_moves_the_child() {
    use crate::tui::commands::EpicCommand;
    let (rt, mut app) = test_runtime().await;
    let parent = rt.db_write().create_epic("Parent", "", None).await.unwrap();
    let child = rt.db_write().create_epic("Child", "", None).await.unwrap();

    dispatch_one(
        &rt,
        &mut app,
        Command::Epic(EpicCommand::Reparent {
            id: child.id,
            new_parent: Some(parent.id),
        }),
    )
    .await;

    let stored = rt.database.get_epic(child.id).await.unwrap();
    assert_eq!(stored.parent_epic_id, Some(parent.id));
}

#[tokio::test]
async fn dispatch_repo_filter_delete_repo_path_forgets_the_path() {
    use crate::tui::commands::RepoFilterCommand;
    let (rt, mut app) = test_runtime().await;
    dispatch_one(
        &rt,
        &mut app,
        Command::Settings(SettingsCommand::SaveRepoPath("/doomed".into())),
    )
    .await;

    dispatch_one(
        &rt,
        &mut app,
        Command::RepoFilter(RepoFilterCommand::DeleteRepoPath("/doomed".into())),
    )
    .await;

    assert!(
        !rt.database
            .list_repo_paths()
            .await
            .contains(&"/doomed".to_string()),
        "the path should no longer be known"
    );
}

/// `workspaces.allium: SetOwnNameFromBoard`: the command sets the caller's own
/// name through the people service, and says so on the status line.
#[tokio::test]
async fn setting_a_display_name_saves_it_and_reports_it() {
    let db = test_db().await;
    let (rt, mut app) = test_runtime_over(db.clone()).await;

    dispatch_one(
        &rt,
        &mut app,
        Command::People(crate::tui::commands::PeopleCommand::SetDisplayName(
            "Alice".into(),
        )),
    )
    .await;

    assert_eq!(db.shown_name_of(crate::store::MEMORY_USER_ID), "Alice");
    assert_eq!(app.status_message(), Some("Display name set to Alice"));
}
