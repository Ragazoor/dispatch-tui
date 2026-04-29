/// Full `Command` match dispatch — one entry per variant, in `Command` enum order.
///
/// Returns any follow-on commands that should be added to the execution queue.
/// The caller (`execute_commands`) extends the queue with the returned vec.
pub(super) fn dispatch(
    command: super::Command,
    app: &mut super::App,
    rt: &super::TuiRuntime,
) -> Vec<super::Command> {
    use super::Command::*;
    match command {
        PersistTask(task) => {
            rt.exec_persist_task(app, task);
            vec![]
        }
        InsertTask { draft, epic_id } => {
            rt.exec_insert_task(app, draft, epic_id);
            vec![]
        }
        DeleteTask(id) => {
            rt.exec_delete_task(app, id);
            vec![]
        }
        DispatchAgent { task, mode } => {
            rt.exec_dispatch_agent(task, mode);
            vec![]
        }
        CaptureTmux { id, window } => {
            rt.exec_capture_tmux(id, window);
            vec![]
        }
        PopOutEditor(kind) => {
            rt.exec_pop_out_editor(app, kind);
            vec![]
        }
        FinalizeEditorResult { kind, outcome } => {
            rt.exec_finalize_editor_result(app, kind, outcome)
        }
        SaveRepoPath(path) => {
            rt.exec_save_repo_path(app, path);
            vec![]
        }
        RefreshFromDb => rt.exec_refresh_from_db(app),
        Cleanup {
            id,
            repo_path,
            worktree,
            tmux_window,
        } => {
            rt.exec_cleanup(id, repo_path, worktree, tmux_window);
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
            rt.exec_quick_dispatch(app, draft, epic_id);
            vec![]
        }
        KillTmuxWindow { window } => {
            rt.exec_kill_tmux_window(window);
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
            rt.exec_finish(id, repo_path, branch, base_branch, worktree, tmux_window);
            vec![]
        }
        // Epic commands
        InsertEpic(draft) => {
            rt.exec_insert_epic(
                app,
                draft.title,
                draft.description,
                draft.repo_path,
                draft.parent_epic_id,
            );
            vec![]
        }
        DeleteEpic(id) => {
            rt.exec_delete_epic(app, id);
            vec![]
        }
        PersistEpic {
            id,
            status,
            sort_order,
        } => {
            rt.exec_persist_epic(app, id, status, sort_order);
            vec![]
        }
        RefreshEpicsFromDb => {
            rt.exec_refresh_epics_from_db(app);
            vec![]
        }
        TriggerEpicFeed {
            epic_id,
            epic_title,
            feed_command,
        } => {
            rt.exec_trigger_epic_feed(epic_id, epic_title, feed_command);
            vec![]
        }
        DispatchEpic { epic } => {
            rt.exec_dispatch_epic(app, epic);
            vec![]
        }
        ToggleEpicAutoDispatch { id, auto_dispatch } => {
            rt.exec_toggle_epic_auto_dispatch(app, id, auto_dispatch);
            vec![]
        }
        // Notification
        SendNotification {
            title,
            body,
            urgent,
        } => {
            rt.exec_send_notification(&title, &body, urgent);
            vec![]
        }
        // Settings
        PersistSetting { key, value } => {
            rt.exec_persist_setting(app, &key, value);
            vec![]
        }
        PersistStringSetting { key, value } => {
            rt.exec_persist_string_setting(app, &key, &value);
            vec![]
        }
        PersistFilterPreset {
            name,
            repo_paths,
            mode,
        } => {
            rt.exec_persist_filter_preset(app, &name, &repo_paths, mode.as_str());
            vec![]
        }
        DeleteFilterPreset(name) => {
            rt.exec_delete_filter_preset(app, &name);
            vec![]
        }
        DeleteRepoPath(path) => {
            rt.exec_delete_repo_path(app, &path);
            vec![]
        }
        // PR commands
        CreatePr {
            id,
            repo_path,
            branch,
            base_branch,
            title,
            description,
        } => {
            rt.exec_create_pr(id, repo_path, branch, base_branch, title, description);
            vec![]
        }
        CheckPrStatus { id, pr_url } => {
            rt.exec_check_pr_status(id, pr_url);
            vec![]
        }
        MergePr { id, pr_url } => {
            rt.exec_merge_pr(id, pr_url);
            vec![]
        }
        // Browser
        OpenInBrowser { url } => {
            rt.exec_open_in_browser(url);
            vec![]
        }
        // Patch sub-status
        PatchSubStatus { id, sub_status } => {
            rt.exec_patch_sub_status(app, id, sub_status);
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
            rt.exec_save_tips_state(seen_up_to, show_mode);
            vec![]
        }
        // Project commands
        CreateProject { name } => {
            rt.exec_create_project(app, name);
            vec![]
        }
        RenameProject { id, name } => {
            rt.exec_rename_project(app, id, name);
            vec![]
        }
        DeleteProject { id } => {
            rt.exec_delete_project(app, id);
            vec![]
        }
        ReorderProject { id, delta } => {
            rt.exec_reorder_project(app, id, delta);
            vec![]
        }
        LoadProposedLearnings => {
            rt.exec_load_proposed_learnings(app);
            vec![]
        }
        ApproveLearning(id) => {
            rt.exec_approve_learning(app, id);
            vec![]
        }
        RejectLearning(id) => {
            rt.exec_reject_learning(app, id);
            vec![]
        }
    }
}
