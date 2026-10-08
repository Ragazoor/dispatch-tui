//! `dispatch agent-diff <task_id>`: the diff pane beneath the agent tree. The
//! feature lives in [`crate::agent_tree::diff_viewer`]; this is only its entry
//! point.

use anyhow::Result;

use crate::models::TaskId;

pub async fn run(board_port: u16, task_id: TaskId) -> Result<()> {
    crate::agent_tree::diff_viewer::run(board_port, task_id).await
}
