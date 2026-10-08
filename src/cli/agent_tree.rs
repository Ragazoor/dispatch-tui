//! `dispatch agent-tree <task_id>`: the changed-file tree pane. The feature
//! lives in [`crate::agent_tree`]; this is only its entry point.

use std::path::Path;

use anyhow::Result;

use crate::models::TaskId;

pub async fn run(data_dir: &Path, board_port: u16, task_id: TaskId) -> Result<()> {
    crate::agent_tree::run::run(data_dir, board_port, task_id).await
}
