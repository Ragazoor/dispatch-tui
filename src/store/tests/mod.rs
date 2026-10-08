use super::*;

mod epics;
mod hooks;
mod learnings;
mod open_in_memory_store;
mod settings;
mod store_seam;
mod tasks;
mod tasks_feed;
mod tasks_patch;
mod usage;

pub(super) async fn in_memory_db() -> Store {
    Store::open_in_memory().unwrap()
}

pub(super) async fn create_task_returning(
    db: &Store,
    title: &str,
    description: &str,
    repo_path: &str,
    plan: Option<&str>,
    status: TaskStatus,
) -> anyhow::Result<Task> {
    let id = db
        .create_task(CreateTaskRequest {
            description,
            plan,
            status,
            ..CreateTaskRequest::fixture(title, repo_path)
        })
        .await?;
    db.get_task(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Task {id} vanished after insert"))
}

/// Create a backlog task and unwrap, for tests that don't care about
/// [`create_task_returning`]'s `Result`. Shared across the db test modules,
/// which otherwise would each declare an identical private copy.
pub(super) async fn make_task(db: &Store, title: &str) -> Task {
    create_task_returning(db, title, "desc", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap()
}
