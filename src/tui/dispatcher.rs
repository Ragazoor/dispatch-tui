//! Routing table for `App::update()`.
//!
//! Adding a new `Message` variant becomes a two-file edit: declare the variant
//! in `types.rs` and add the match arm here. The `App` state container and
//! lifecycle methods live in `mod.rs`; per-message handlers live in
//! `update/*.rs`.

use crate::tui::messages::{
    EditorMessage, EpicMessage, FeedMessage, InputMessage, LearningMessage, MainSessionMessage,
    PrMessage, ProjectMessage, RepoFilterMessage, SplitMessage, SystemMessage, TaskMessage,
    TipsMessage, WrapUpMessage,
};
use crate::tui::types::{Command, Message};
use crate::tui::App;

/// Per-domain dispatcher for [`EditorMessage`] variants.
fn dispatch_editor(app: &mut App, msg: EditorMessage) -> Vec<Command> {
    match msg {
        EditorMessage::DescriptionResult(value) => app.handle_description_editor_result(value),
        EditorMessage::Result { kind, outcome } => app.handle_editor_result(kind, outcome),
    }
}

/// Per-domain dispatcher for [`TaskMessage`] variants.
fn dispatch_task(app: &mut App, msg: TaskMessage) -> Vec<Command> {
    match msg {
        TaskMessage::Move { id, direction } => app.handle_move_task(id, direction),
        TaskMessage::ReorderItem(dir) => app.handle_reorder_item(dir),
        TaskMessage::Dispatch(id, mode) => app.handle_dispatch_task(id, mode),
        TaskMessage::Dispatched {
            id,
            worktree,
            tmux_window,
            switch_focus,
        } => app.handle_dispatched(id, worktree, tmux_window, switch_focus),
        TaskMessage::Created { task } => app.handle_task_created(task),
        TaskMessage::Delete(id) => app.handle_delete_task(id),
        TaskMessage::OpenDetail(task_id) => app.handle_open_task_detail(task_id),
        TaskMessage::CloseDetail => app.handle_close_task_detail(),
        TaskMessage::ToggleFlattened => app.handle_toggle_flattened(),
        TaskMessage::WindowGone(id) => app.handle_window_gone(id),
        TaskMessage::Refresh(tasks) => app.handle_refresh_tasks(tasks),
        TaskMessage::Updated(task) => app.handle_task_updated(task),
        TaskMessage::Resume(id) => app.handle_resume_task(id),
        TaskMessage::Resumed { id, tmux_window } => app.handle_resumed(id, tmux_window),
        TaskMessage::DispatchFailed(id) => app.handle_dispatch_failed(id),
        TaskMessage::MarkDispatching(id) => app.handle_mark_dispatching(id),
        TaskMessage::Edited(edit) => app.handle_task_edited(edit),
        TaskMessage::QuickDispatch { repo_path, epic_id } => {
            app.handle_quick_dispatch(repo_path, epic_id)
        }
        TaskMessage::AgentCrashed(id) => app.handle_agent_crashed(id),
        TaskMessage::KillAndRetry(id) => app.handle_kill_and_retry(id),
        TaskMessage::RetryResume(id) => app.handle_retry_resume(id),
        TaskMessage::RetryFresh(id) => app.handle_retry_fresh(id),
        TaskMessage::Archive(id) => app.handle_archive_task(id),
        TaskMessage::ToggleSelect(id) => app.handle_toggle_select(id),
        TaskMessage::BatchMove { ids, direction } => app.handle_batch_move_tasks(ids, direction),
        TaskMessage::BatchArchive(ids) => app.handle_batch_archive_tasks(ids),
        TaskMessage::FinishComplete(id) => app.handle_finish_complete(id),
        TaskMessage::FinishFailed {
            id,
            error,
            is_conflict,
        } => app.handle_finish_failed(id, error, is_conflict),
        TaskMessage::DetachTmux(id) => app.handle_detach_tmux(vec![id]),
        TaskMessage::BatchDetachTmux(ids) => app.handle_detach_tmux(ids),
    }
}

/// Per-domain dispatcher for [`EpicMessage`] variants.
fn dispatch_epic(app: &mut App, msg: EpicMessage) -> Vec<Command> {
    match msg {
        EpicMessage::Dispatch(id) => app.handle_dispatch_epic(id),
        EpicMessage::Enter(id) => app.handle_enter_epic(id),
        EpicMessage::Exit => app.handle_exit_epic(),
        EpicMessage::Refresh(epics) => app.handle_refresh_epics(epics),
        EpicMessage::Updated(epic) => app.handle_epic_updated(epic),
        EpicMessage::Created(epic) => app.handle_epic_created(epic),
        EpicMessage::Edit(id) => app.handle_edit_epic(id),
        EpicMessage::Edited(epic) => app.handle_epic_edited(epic),
        EpicMessage::Delete(id) => app.handle_delete_epic(id),
        EpicMessage::ToggleAutoDispatch(id) => app.handle_toggle_epic_auto_dispatch(id),
        EpicMessage::ToggleGroupByRepo(id) => app.handle_toggle_epic_group_by_repo(id),
        EpicMessage::ConfirmDelete => app.handle_confirm_delete_epic(),
        EpicMessage::MoveStatus(id, dir) => app.handle_move_epic_status(id, dir),
        EpicMessage::Archive(id) => app.handle_archive_epic(id),
        EpicMessage::ConfirmArchive => app.handle_confirm_archive_epic(),
        EpicMessage::StartNew => app.handle_start_new_epic(),
        EpicMessage::SubmitTitle(v) => app.handle_submit_epic_title(v),
        EpicMessage::SubmitDescription(v) => app.handle_submit_epic_description(v),
        EpicMessage::SubmitRepoPath(v) => app.handle_submit_epic_repo_path(v),
        EpicMessage::ToggleSelect(id) => app.handle_toggle_select_epic(id),
        EpicMessage::BatchArchive(ids) => app.handle_batch_archive_epics(ids),
    }
}

/// Per-domain dispatcher for [`InputMessage`] variants.
fn dispatch_input(app: &mut App, msg: InputMessage) -> Vec<Command> {
    match msg {
        InputMessage::StartNewTask => app.handle_start_new_task(),
        InputMessage::CopyTask => app.handle_copy_task(),
        InputMessage::CancelInput => app.handle_cancel_input(),
        InputMessage::ConfirmDeleteStart => app.handle_confirm_delete_start(),
        InputMessage::ConfirmDeleteYes => app.handle_confirm_delete_yes(),
        InputMessage::CancelDelete => app.handle_cancel_delete(),
        InputMessage::SubmitTitle(value) => app.handle_submit_title(value),
        InputMessage::SubmitDescription(value) => app.handle_submit_description(value),
        InputMessage::SubmitRepoPath(value) => app.handle_submit_repo_path(value),
        InputMessage::SubmitTag(tag) => app.handle_submit_tag(tag),
        InputMessage::SubmitBaseBranch(value) => app.handle_submit_base_branch(value),
        InputMessage::SubmitWrapUpMode(mode) => app.handle_submit_wrap_up_mode(mode),
        InputMessage::InputChar(c) => app.handle_input_char(c),
        InputMessage::InputBackspace => app.handle_input_backspace(),
        InputMessage::StartQuickDispatchSelection => app.handle_start_quick_dispatch_selection(),
        InputMessage::SelectQuickDispatchRepo(idx) => app.handle_select_quick_dispatch_repo(idx),
        InputMessage::CancelRetry => app.handle_cancel_retry(),
        InputMessage::ConfirmDone => app.handle_confirm_done(),
        InputMessage::CancelDone => app.handle_cancel_done(),
        InputMessage::ConfirmDetachTmux => app.handle_confirm_detach_tmux(),
    }
}

/// Per-domain dispatcher for [`SystemMessage`] variants.
fn dispatch_system(app: &mut App, msg: SystemMessage) -> Vec<Command> {
    match msg {
        SystemMessage::Tick => app.handle_tick(),
        SystemMessage::TerminalResized => vec![],
        SystemMessage::FocusChanged(focused) => app.handle_focus_changed(focused),
        SystemMessage::Quit => app.handle_quit(),
        SystemMessage::Error(text) => app.handle_error(text),
        SystemMessage::DismissError => app.handle_dismiss_error(),
        SystemMessage::StatusInfo(text) => app.handle_status_info(text),
        SystemMessage::ToggleHelp => app.handle_toggle_help(),
        SystemMessage::ToggleNotifications => app.handle_toggle_notifications(),
        SystemMessage::OpenInBrowser { url } => app.handle_open_in_browser(url),
        SystemMessage::MessageReceived(id) => app.handle_message_received(id),
    }
}

/// Per-domain dispatcher for [`PrMessage`] variants.
fn dispatch_pr(app: &mut App, msg: PrMessage) -> Vec<Command> {
    match msg {
        PrMessage::Merged(id) => app.handle_pr_merged(id),
        PrMessage::StartMerge(id) => app.handle_start_merge_pr(id),
        PrMessage::ConfirmMerge => app.handle_confirm_merge_pr(),
        PrMessage::CancelMerge => app.handle_cancel_merge_pr(),
        PrMessage::MergeFailed { id, error } => app.handle_merge_pr_failed(id, error),
        PrMessage::ReviewState {
            id,
            review_decision,
        } => app.handle_pr_review_state(id, review_decision),
    }
}

/// Per-domain dispatcher for [`FeedMessage`] variants.
fn dispatch_feed(app: &mut App, msg: FeedMessage) -> Vec<Command> {
    match msg {
        FeedMessage::TriggerEpic(id) => app.handle_trigger_epic_feed(id),
        FeedMessage::Refreshed { epic_title, count } => {
            app.handle_feed_refreshed(epic_title, count)
        }
        FeedMessage::Failed { epic_title, error } => app.handle_feed_failed(epic_title, error),
    }
}

/// Per-domain dispatcher for [`WrapUpMessage`] variants.
fn dispatch_wrap_up(app: &mut App, msg: WrapUpMessage) -> Vec<Command> {
    match msg {
        WrapUpMessage::Start(id) => app.handle_start_wrap_up(id),
        WrapUpMessage::Rebase => app.handle_wrap_up_rebase(),
        WrapUpMessage::Done => app.handle_wrap_up_done(),
        WrapUpMessage::Cancel => app.handle_cancel_wrap_up(),
        WrapUpMessage::EpicStart(id) => app.handle_start_epic_wrap_up(id),
        WrapUpMessage::EpicRebase => app.handle_epic_wrap_up(),
        WrapUpMessage::EpicCancel => app.handle_cancel_epic_wrap_up(),
        WrapUpMessage::CancelMergeQueue => app.handle_cancel_merge_queue(),
    }
}

/// Per-domain dispatcher for [`ProjectMessage`] variants.
fn dispatch_project(app: &mut App, msg: ProjectMessage) -> Vec<Command> {
    match msg {
        ProjectMessage::Updated(projects) => app.handle_projects_updated(projects),
        ProjectMessage::Select(project_id) => app.handle_select_project(project_id),
        ProjectMessage::Follow(project_id) => app.handle_follow_project(project_id),
    }
}

/// Per-domain dispatcher for [`SplitMessage`] variants.
fn dispatch_split(app: &mut App, msg: SplitMessage) -> Vec<Command> {
    match msg {
        SplitMessage::Toggle => app.handle_toggle_split_mode(),
        SplitMessage::Swap(task_id) => app.handle_swap_split_pane(task_id),
        SplitMessage::PaneOpened { pane_id, task_id } => {
            app.handle_split_pane_opened(pane_id, task_id)
        }
        SplitMessage::PaneClosed => app.handle_split_pane_closed(),
    }
}

/// Per-domain dispatcher for [`MainSessionMessage`] variants.
fn dispatch_main_session(app: &mut App, msg: MainSessionMessage) -> Vec<Command> {
    match msg {
        MainSessionMessage::SubmitDir(dir) => app.handle_submit_main_session_dir(dir),
        MainSessionMessage::Created(window) => app.handle_main_session_created(window),
    }
}

/// Per-domain dispatcher for [`TipsMessage`] variants.
fn dispatch_tips(app: &mut App, msg: TipsMessage) -> Vec<Command> {
    match msg {
        TipsMessage::Show {
            tips,
            starting_index,
            max_seen_id,
            show_mode,
        } => app.handle_show_tips(tips, starting_index, max_seen_id, show_mode),
        TipsMessage::Next => app.handle_next_tip(),
        TipsMessage::Prev => app.handle_prev_tip(),
        TipsMessage::SetMode(mode) => app.handle_set_tips_mode(mode),
        TipsMessage::Close => app.handle_close_tips(),
    }
}

/// Per-domain dispatcher for [`LearningMessage`] variants.
fn dispatch_learning(app: &mut App, msg: LearningMessage) -> Vec<Command> {
    use crate::tui::commands::LearningCommand;
    match msg {
        LearningMessage::Open => vec![Command::Learning(LearningCommand::Load)],
        LearningMessage::Show(learnings) => app.handle_show_learnings(learnings),
        LearningMessage::Close => app.handle_close_learnings(),
        LearningMessage::Navigate(delta) => app.handle_navigate_learning(delta),
        LearningMessage::Archive(id) => app.handle_archive_learning(id),
        LearningMessage::Reject(id) => app.handle_reject_learning(id),
        LearningMessage::Approve(id) => app.handle_approve_learning(id),
        LearningMessage::Edit(id) => app.handle_edit_learning(id),
        LearningMessage::Actioned(id) => app.handle_learning_actioned(id),
        LearningMessage::Edited(updated) => app.handle_learning_edited(updated),
        LearningMessage::ToggleView => app.handle_toggle_learnings_view(),
        LearningMessage::NavigateTree(nav) => app.handle_navigate_tree_learning(nav),
        LearningMessage::NeedsReviewCountUpdated(n) => {
            app.needs_review_count = n;
            vec![]
        }
    }
}

/// Process a message and return a list of side-effect commands.
pub(in crate::tui) fn dispatch(app: &mut App, msg: Message) -> Vec<Command> {
    match msg {
        // ── Board navigation, view toggles, system events ──
        Message::System(sm) => dispatch_system(app, sm),
        Message::Task(tm) => dispatch_task(app, tm),
        Message::NavigateColumn(delta) => app.handle_navigate_column(delta),
        Message::NavigateRow(delta) => app.handle_navigate_row(delta),
        Message::Split(sm) => dispatch_split(app, sm),
        Message::RepoPathsUpdated(paths) => app.handle_repo_paths_updated(paths),

        // ── Task wrap-up ──
        Message::WrapUp(wm) => dispatch_wrap_up(app, wm),
        Message::ClearSelection => app.handle_clear_selection(),
        Message::SelectAllColumn => app.handle_select_all_column(),

        // ── Form input, text entry, creation flows ──
        Message::Input(im) => dispatch_input(app, im),
        Message::Editor(em) => dispatch_editor(app, em),

        // ── Epic CRUD, lifecycle, wrap-up ──
        Message::Epic(em) => dispatch_epic(app, em),

        // ── PR flow: creation, merge, review state ──
        Message::Pr(pm) => dispatch_pr(app, pm),

        // ── Task repo filters and filter presets ──
        Message::RepoFilter(rfm) => dispatch_repo_filter(app, rfm),

        // ── Tips overlay ──
        Message::Tips(tm) => dispatch_tips(app, tm),

        // ── Project messages ──
        Message::Project(pm) => dispatch_project(app, pm),
        Message::Feed(fm) => dispatch_feed(app, fm),
        Message::Learning(lm) => dispatch_learning(app, lm),

        // ── Main session ──
        Message::MainSession(mm) => dispatch_main_session(app, mm),
    }
}

/// Per-domain dispatcher for [`RepoFilterMessage`] variants.
fn dispatch_repo_filter(app: &mut App, msg: RepoFilterMessage) -> Vec<Command> {
    use crate::tui::messages::RepoFilterMessage::*;
    match msg {
        Start => app.handle_start_repo_filter(),
        Close => app.handle_close_repo_filter(),
        Toggle(path) => app.handle_toggle_repo_filter(path),
        ToggleAll => app.handle_toggle_all_repo_filter(),
        ToggleMode => app.handle_toggle_repo_filter_mode(),
        ToggleOnlyActive => app.handle_toggle_only_active(),
        MoveCursor(delta) => app.handle_move_repo_cursor(delta),
        StartSavePreset => app.handle_start_save_preset(),
        SavePreset(name) => app.handle_save_filter_preset(name),
        LoadPreset(name) => app.handle_load_filter_preset(name),
        StartDeletePreset => app.handle_start_delete_preset(),
        DeletePreset(name) => app.handle_delete_filter_preset(name),
        StartDeleteRepoPath => app.handle_start_delete_repo_path(),
        DeleteRepoPath(path) => app.handle_delete_repo_path(path),
        CancelPresetInput => app.handle_cancel_preset_input(),
        PresetsLoaded(presets) => app.handle_filter_presets_loaded(presets),
    }
}
