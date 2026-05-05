use super::*;

mod alerts;
mod epics;
mod learnings;
mod migrations;
mod prs;
mod projects;
mod settings;
mod tasks;

pub(super) fn in_memory_db() -> Database {
    Database::open_in_memory().unwrap()
}

pub(super) fn create_task_returning(
    db: &Database,
    title: &str,
    description: &str,
    repo_path: &str,
    plan: Option<&str>,
    status: TaskStatus,
) -> anyhow::Result<Task> {
    let id = db.create_task(CreateTaskRequest {
        title,
        description,
        repo_path,
        plan,
        status,
        base_branch: "main",
        epic_id: None,
        sort_order: None,
        tag: None,
        project_id: ProjectId(1),
    })?;
    db.get_task(id)?
        .ok_or_else(|| anyhow::anyhow!("Task {id} vanished after insert"))
}
