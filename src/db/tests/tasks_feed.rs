use super::*;
use crate::models::test_tmux_window;

// ---------------------------------------------------------------------------
// upsert_feed_tasks
// ---------------------------------------------------------------------------

fn make_feed_item(external_id: &str, title: &str) -> crate::models::FeedItem {
    crate::models::FeedItem {
        external_id: external_id.to_string(),
        title: title.to_string(),
        description: "desc".to_string(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Bug,
        labels: Vec::new(),
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }
}

/// Build a parallel vec of "main" base branches for tests that don't
/// exercise the per-task base_branch path.
fn main_branches(n: usize) -> Vec<String> {
    vec!["main".to_string(); n]
}

// ---------------------------------------------------------------------------
// Retired feed items (core.allium: RetiredFeedItem; tasks.allium: DeleteTask;
// feeds.allium: IngestSkipsRetiredFeedItems)
// ---------------------------------------------------------------------------
//
// Every assertion here is behavioural: a retired id is observed through the
// only thing it changes, which is whether the next upsert inserts a task for
// it. That keeps these tests independent of how the record is stored.

/// A root epic that carries a `feed_command`, i.e. one whose cycle emits items
/// and so is the `nearest_feed_epic` a deletion retires under.
async fn feed_epic(db: &Database, title: &str) -> Epic {
    let epic = db.create_epic(title, "", None).await.unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some("echo []")))
        .await
        .unwrap();
    epic
}

/// Upsert `items` into `epic` with placeholder repo paths and base branches.
async fn upsert(db: &Database, epic: EpicId, items: &[crate::models::FeedItem]) {
    db.upsert_feed_tasks(
        epic,
        items,
        &vec!["/repo".to_string(); items.len()],
        &main_branches(items.len()),
    )
    .await
    .unwrap();
}

/// The only id-to-task lookup these tests need: every task under `epic`
/// carrying `external_id`.
async fn tasks_with_external_id(db: &Database, epic: EpicId, external_id: &str) -> Vec<Task> {
    db.list_tasks_for_epic(epic)
        .await
        .unwrap()
        .into_iter()
        .filter(|t| t.external_id.as_deref() == Some(external_id))
        .collect()
}

/// Complete and delete the single task under `epic` that carries
/// `external_id` — the DeleteTask gesture (`x` on a Done card), which requires
/// status = done.
async fn complete_and_delete(db: &Database, epic: EpicId, external_id: &str) -> TaskId {
    let tasks = tasks_with_external_id(db, epic, external_id).await;
    assert_eq!(
        tasks.len(),
        1,
        "fixture: exactly one task for {external_id}"
    );
    let id = tasks[0].id;
    db.patch_task(id, &TaskPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();
    db.delete_task(id).await.unwrap();
    id
}

mod additive_and_stale;
mod done_stamping;
mod insert_and_retired;
mod labels_and_urls;
mod repo_branch_tag;
