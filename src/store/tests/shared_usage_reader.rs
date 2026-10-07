//! Where a usage READ goes — Phase 11 of the SpacetimeDB migration.
//!
//! Spec: none — `usage_events` is append-only telemetry with no
//! user-observable rule beyond "recorded" (task #4915).
//!
//! The read twin of `shared_writer.rs`, the same shape
//! `shared_learning_reader.rs` uses: with no reader attached, a read answers
//! from SQLite exactly as it always has. With one attached, the reader
//! answers INSTEAD of SQLite — proven here by making the two disagree and
//! asserting the reader's answer wins.

use super::*;
use crate::models::{UsageActor, UsageCategory, UsageEvent, UsageSummary};
use crate::store::{SharedUsageReader, UsageQuery};
use std::sync::Mutex;

/// A reader that answers from a fixed, caller-supplied set of summaries and
/// remembers which methods were called.
#[derive(Default)]
struct RecordingReader {
    calls: Mutex<Vec<&'static str>>,
    summaries: Vec<UsageSummary>,
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
impl SharedUsageReader for RecordingReader {
    async fn query_usage(&self, _query: &UsageQuery) -> anyhow::Result<Vec<UsageSummary>> {
        self.record("query_usage");
        Ok(self.summaries.clone())
    }
}

fn a_summary(action: &str, count: i64) -> UsageSummary {
    UsageSummary {
        category: "keybinding".to_string(),
        action: action.to_string(),
        detail: None,
        actor: "human".to_string(),
        count,
        last_used: chrono::Utc::now(),
    }
}

/// The unchanged board: no reader, so a read answers from SQLite.
#[tokio::test]
async fn with_no_reader_a_usage_read_goes_to_sqlite() {
    let db = in_memory_db().await;
    db.record_usage_event(&UsageEvent {
        category: UsageCategory::Keybinding,
        action: "local_action".to_string(),
        detail: None,
        actor: UsageActor::Human,
    })
    .await
    .unwrap();

    let summary = db.query_usage(&UsageQuery::default()).await.unwrap();
    assert_eq!(summary.len(), 1);
    assert_eq!(summary[0].action, "local_action");
}

/// The configured board: the reader answers, not SQLite. Proven by making
/// them disagree — an event recorded locally (no writer attached, so it
/// landed in SQLite) is invisible, and the reader's own summary is what comes
/// back.
#[tokio::test]
async fn with_a_reader_attached_a_usage_read_routes_to_it() {
    let db = in_memory_db().await;
    db.record_usage_event(&UsageEvent {
        category: UsageCategory::Keybinding,
        action: "local_only".to_string(),
        detail: None,
        actor: UsageActor::Human,
    })
    .await
    .unwrap();

    let reader = Arc::new(RecordingReader {
        summaries: vec![a_summary("from_the_store", 7)],
        ..Default::default()
    });
    let db = db.with_shared_usage_reader(reader.clone());

    let summary = db.query_usage(&UsageQuery::default()).await.unwrap();
    assert_eq!(summary.len(), 1);
    assert_eq!(summary[0].action, "from_the_store");
    assert_eq!(summary[0].count, 7);

    assert_eq!(reader.calls(), vec!["query_usage"]);
}
