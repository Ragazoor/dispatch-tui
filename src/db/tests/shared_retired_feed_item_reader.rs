//! Where a `retired_without_task` READ goes (task #4971).
//!
//! The read twin of `shared_writer.rs`, the same shape `shared_usage_reader.rs`
//! and `shared_learning_reader.rs` use: with no reader attached, a read
//! answers from SQLite exactly as it always has. With one attached, the
//! reader answers INSTEAD of SQLite — proven here by making the two disagree
//! and asserting the reader's answer wins.

use super::*;
use crate::db::SharedRetiredFeedItemReader;
use std::sync::Mutex;

/// A reader that answers from a fixed, caller-supplied set of ids and
/// remembers which methods were called.
#[derive(Default)]
struct RecordingReader {
    calls: Mutex<Vec<&'static str>>,
    answer: Vec<String>,
}

impl RecordingReader {
    fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, what: &'static str) {
        self.calls.lock().unwrap().push(what);
    }
}

#[async_trait::async_trait]
impl SharedRetiredFeedItemReader for RecordingReader {
    async fn retired_without_task(
        &self,
        _feed_epic_id: EpicId,
        _external_ids: &[String],
    ) -> anyhow::Result<Vec<String>> {
        self.record("retired_without_task");
        Ok(self.answer.clone())
    }
}

fn a_feed_item(external_id: &str) -> crate::models::FeedItem {
    crate::models::FeedItem {
        external_id: external_id.to_string(),
        title: external_id.to_string(),
        description: String::new(),
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

/// The unchanged board: no reader, so the read answers from SQLite — proven
/// by retiring a feed task's id (via `delete_task`) and getting it back as
/// survivor-less.
#[tokio::test]
async fn with_no_reader_a_retired_without_task_read_goes_to_sqlite() {
    let db = unattached_db().await;
    let feed_epic = db.create_epic("Feed", "", None).await.unwrap().id;
    db.patch_epic(feed_epic, &EpicPatch::new().feed_command(Some("echo []")))
        .await
        .unwrap();
    db.upsert_feed_tasks(
        feed_epic,
        &[a_feed_item("ext-1")],
        &["/repo".to_string()],
        &["main".to_string()],
    )
    .await
    .unwrap();
    let task_id = db
        .list_tasks_for_epic(feed_epic)
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.external_id.as_deref() == Some("ext-1"))
        .unwrap()
        .id;
    db.delete_task(task_id).await.unwrap();

    let retired = db
        .retired_without_task(feed_epic, &["ext-1".to_string()])
        .await
        .unwrap();
    assert_eq!(retired, vec!["ext-1".to_string()]);
}

/// The configured board: the reader answers, not SQLite. Proven by making
/// them disagree — SQLite has no retired record at all (nothing was ever
/// deleted), yet the reader's own fixed answer is what comes back.
#[tokio::test]
async fn with_a_reader_attached_a_retired_without_task_read_routes_to_it() {
    let db = in_memory_db().await;
    let feed_epic = db.create_epic("Feed", "", None).await.unwrap().id;
    db.patch_epic(feed_epic, &EpicPatch::new().feed_command(Some("echo []")))
        .await
        .unwrap();

    let reader = Arc::new(RecordingReader {
        answer: vec!["from_the_store".to_string()],
        ..Default::default()
    });
    let db = db.with_shared_retired_feed_item_reader(reader.clone());

    let retired = db
        .retired_without_task(feed_epic, &["from_the_store".to_string()])
        .await
        .unwrap();
    assert_eq!(retired, vec!["from_the_store".to_string()]);
    assert_eq!(reader.calls(), vec!["retired_without_task"]);
}
