/// Full `Command` match dispatch — one entry per variant, in `Command` enum order.
///
/// Returns any follow-on commands that should be added to the execution queue.
/// The caller (`execute_commands`) extends the queue with the returned vec.
pub(super) async fn dispatch(
    command: super::Command,
    app: &mut super::App,
    rt: &super::TuiRuntime,
) -> Vec<super::Command> {
    use super::Command::*;
    match command {
        Task(cmd) => dispatch_task(rt, app, cmd).await,
        Editor(cmd) => dispatch_editor(rt, app, cmd).await,
        Feed(cmd) => {
            dispatch_feed(rt, cmd);
            vec![]
        }
        SaveRepoPath(path) => {
            rt.exec_save_repo_path(app, path).await;
            vec![]
        }
        OpenMainSession => {
            rt.exec_open_main_session(app).await;
            vec![]
        }
        // Epic commands
        Epic(cmd) => {
            dispatch_epic(rt, app, cmd).await;
            vec![]
        }
        System(cmd) => {
            dispatch_system(rt, cmd);
            vec![]
        }
        // Settings
        PersistSetting { key, value } => {
            rt.exec_persist_setting(app, &key, value).await;
            vec![]
        }
        PersistStringSetting { key, value } => {
            rt.exec_persist_string_setting(app, &key, &value).await;
            vec![]
        }
        PersistFilterPreset {
            name,
            repo_paths,
            mode,
        } => {
            rt.exec_persist_filter_preset(app, &name, &repo_paths, mode.as_str())
                .await;
            vec![]
        }
        DeleteFilterPreset(name) => {
            rt.exec_delete_filter_preset(app, &name).await;
            vec![]
        }
        DeleteRepoPath(path) => {
            rt.exec_delete_repo_path(app, &path).await;
            vec![]
        }
        // PR commands (creation is agent-driven via the /wrap-up skill)
        Pr(cmd) => {
            dispatch_pr(rt, cmd);
            vec![]
        }
        // Split mode
        EnterSplitMode => {
            rt.exec_enter_split_mode(app);
            vec![]
        }
        EnterSplitModeWithTask { task_id, window } => {
            rt.exec_enter_split_mode_with_task(app, task_id, &window);
            vec![]
        }
        ExitSplitMode {
            pane_id,
            restore_window,
        } => {
            rt.exec_exit_split_mode(app, &pane_id, restore_window.as_deref());
            vec![]
        }
        SwapSplitPane {
            task_id,
            new_window,
            old_pane_id,
            old_window,
        } => {
            rt.exec_swap_split_pane(
                app,
                task_id,
                &new_window,
                old_pane_id.as_deref(),
                old_window.as_deref(),
            );
            vec![]
        }
        FocusSplitPane { pane_id } => {
            rt.exec_focus_split_pane(pane_id);
            vec![]
        }
        CheckSplitPaneExists { pane_id } => {
            rt.exec_check_split_pane(app, &pane_id);
            vec![]
        }
        RespawnSplitPane { pane_id } => {
            rt.exec_respawn_split_pane(app, &pane_id);
            vec![]
        }
        // Tips
        SaveTipsState {
            seen_up_to,
            show_mode,
        } => {
            rt.exec_save_tips_state(seen_up_to, show_mode).await;
            vec![]
        }
        // Project commands
        CreateProject { name } => {
            rt.exec_create_project(app, name).await;
            vec![]
        }
        RenameProject { id, name } => {
            rt.exec_rename_project(app, id, name).await;
            vec![]
        }
        DeleteProject { id } => {
            rt.exec_delete_project(app, id).await;
            vec![]
        }
        ReorderProject { id, delta } => {
            rt.exec_reorder_project(app, id, delta).await;
            vec![]
        }
        Learning(cmd) => {
            dispatch_learning(rt, app, cmd).await;
            vec![]
        }
    }
}

async fn dispatch_learning(
    rt: &super::TuiRuntime,
    app: &mut super::App,
    cmd: crate::tui::commands::LearningCommand,
) {
    use crate::tui::commands::LearningCommand::*;
    match cmd {
        Load => rt.exec_load_learnings(app).await,
        Archive(id) => rt.exec_archive_learning(app, id).await,
        Reject(id) => rt.exec_reject_learning(app, id).await,
        Approve(id) => rt.exec_approve_learning(app, id).await,
    }
}

/// Per-domain dispatcher for [`crate::tui::commands::TaskCommand`] variants.
async fn dispatch_task(
    rt: &super::TuiRuntime,
    app: &mut super::App,
    cmd: crate::tui::commands::TaskCommand,
) -> Vec<super::Command> {
    use crate::tui::commands::TaskCommand::*;
    match cmd {
        Persist(task) => {
            rt.exec_persist_task(app, task).await;
            vec![]
        }
        Insert { draft, epic_id } => {
            rt.exec_insert_task(app, draft, epic_id).await;
            vec![]
        }
        Delete(id) => {
            rt.exec_delete_task(app, id).await;
            vec![]
        }
        DispatchAgent { task, mode } => {
            rt.exec_dispatch_agent(task, mode).await;
            vec![]
        }
        Cleanup {
            id,
            repo_path,
            worktree,
            tmux_window,
        } => {
            rt.exec_cleanup(id, repo_path, worktree, tmux_window).await;
            vec![]
        }
        Finish {
            id,
            repo_path,
            branch,
            base_branch,
            worktree,
            tmux_window,
        } => {
            rt.exec_finish(id, repo_path, branch, base_branch, worktree, tmux_window)
                .await;
            vec![]
        }
        CheckWindow { id, window } => {
            rt.exec_check_window(id, window);
            vec![]
        }
        Resume { task } => {
            rt.exec_resume(task);
            vec![]
        }
        JumpToTmux { window } => {
            rt.exec_jump_to_tmux(app, window);
            vec![]
        }
        QuickDispatch { draft, epic_id } => {
            rt.exec_quick_dispatch(app, draft, epic_id).await;
            vec![]
        }
        KillTmuxWindow { window } => {
            rt.exec_kill_tmux_window(window);
            vec![]
        }
        PatchSubStatus { id, sub_status } => {
            rt.exec_patch_sub_status(app, id, sub_status).await;
            vec![]
        }
        SeedActivity { id, at } => {
            rt.exec_seed_activity(app, id, at).await;
            vec![]
        }
        RefreshFromDb => rt.exec_refresh_from_db(app).await,
    }
}

/// Per-domain dispatcher for [`crate::tui::commands::EpicCommand`] variants.
async fn dispatch_epic(
    rt: &super::TuiRuntime,
    app: &mut super::App,
    cmd: crate::tui::commands::EpicCommand,
) {
    use crate::tui::commands::EpicCommand::*;
    match cmd {
        Dispatch { epic } => rt.exec_dispatch_epic(app, epic).await,
        Insert(draft) => {
            rt.exec_insert_epic(
                app,
                draft.title,
                draft.description,
                draft.repo_path,
                draft.parent_epic_id,
            )
            .await
        }
        Delete(id) => rt.exec_delete_epic(app, id).await,
        Persist {
            id,
            status,
            sort_order,
        } => rt.exec_persist_epic(app, id, status, sort_order).await,
        ToggleAutoDispatch { id, auto_dispatch } => {
            rt.exec_toggle_epic_auto_dispatch(app, id, auto_dispatch)
                .await
        }
        ToggleGroupByRepo { id, group_by_repo } => {
            rt.exec_toggle_epic_group_by_repo(app, id, group_by_repo)
                .await
        }
        RefreshFromDb => rt.exec_refresh_epics_from_db(app).await,
    }
}

/// Per-domain dispatcher for [`crate::tui::commands::SystemCommand`] variants.
fn dispatch_system(rt: &super::TuiRuntime, cmd: crate::tui::commands::SystemCommand) {
    use crate::tui::commands::SystemCommand::*;
    match cmd {
        SendNotification {
            title,
            body,
            urgent,
        } => rt.exec_send_notification(&title, &body, urgent),
        OpenInBrowser { url } => rt.exec_open_in_browser(url),
    }
}

/// Per-domain dispatcher for [`crate::tui::commands::PrCommand`] variants.
fn dispatch_pr(rt: &super::TuiRuntime, cmd: crate::tui::commands::PrCommand) {
    use crate::tui::commands::PrCommand::*;
    match cmd {
        CheckStatus { id, pr_url } => rt.exec_check_pr_status(id, pr_url),
        Merge { id, pr_url } => rt.exec_merge_pr(id, pr_url),
    }
}

/// Per-domain dispatcher for [`crate::tui::commands::FeedCommand`] variants.
fn dispatch_feed(rt: &super::TuiRuntime, cmd: crate::tui::commands::FeedCommand) {
    use crate::tui::commands::FeedCommand::*;
    match cmd {
        TriggerEpic {
            epic_id,
            epic_title,
            feed_command,
        } => rt.exec_trigger_epic_feed(epic_id, epic_title, feed_command),
    }
}

/// Per-domain dispatcher for [`crate::tui::commands::EditorCommand`] variants.
///
/// `FinalizeResult` re-enters the queue: post-edit `app.update(...)` calls
/// inside `exec_finalize_editor_result` can produce follow-on commands
/// (DB persistence, status messages), which the runtime queue then drains.
async fn dispatch_editor(
    rt: &super::TuiRuntime,
    app: &mut super::App,
    cmd: crate::tui::commands::EditorCommand,
) -> Vec<super::Command> {
    use crate::tui::commands::EditorCommand::*;
    match cmd {
        PopOut(kind) => {
            rt.exec_pop_out_editor(app, kind);
            vec![]
        }
        FinalizeResult { kind, outcome } => {
            rt.exec_finalize_editor_result(app, kind, outcome).await
        }
    }
}
