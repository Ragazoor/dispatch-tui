use super::*;
use crate::models::test_tmux_window;

/// A running task with a worktree and a tmux window — what `exit_session`
/// closes.
async fn running_task_with_window(
    db: &Arc<dyn store::TaskStore>,
    epic_id: Option<EpicId>,
) -> (TaskId, crate::models::TmuxWindow) {
    let svc = task_svc(db);
    let mut params = make_task_params("/repo");
    params.epic_id = epic_id;
    let id = svc.create_task(params).await.unwrap();
    let window = crate::models::TmuxWindow::for_task(id);
    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Running)
            .worktree(FieldUpdate::Set("/repo/.worktrees/wt".to_string()))
            .tmux_window(crate::service::TmuxWindowUpdate::Set(window.clone())),
    )
    .await
    .unwrap();
    (id, window)
}

mod claim_and_close;
mod epic_in_epic;
mod epic_service;
mod task_mutations;
mod task_service;
