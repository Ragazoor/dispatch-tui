//! Where a `retired_without_task` READ goes (task #4971).
//!
//! The read twin of `shared_writer.rs`, the same shape `shared_usage_reader.rs`
//! and `shared_learning_reader.rs` use: with no reader attached, a read
//! answers from SQLite exactly as it always has. With one attached, the
//! reader answers INSTEAD of SQLite — proven here by making the two disagree
//! and asserting the reader's answer wins.

use super::*;
use crate::store::SharedRetiredFeedItemReader;
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
