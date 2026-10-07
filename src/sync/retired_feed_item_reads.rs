//! Where a shared-table retired-feed-item READ goes, when it does not go to
//! SQLite.
//!
//! Spec: `docs/specs/core.allium`'s `RetiredFeedItem`, `docs/specs/feeds.allium`'s
//! `IngestSkipsRetiredFeedItems` (task #4971).
//!
//! The same shape as `usage_reads`/`learning_reads`, for the same reason:
//! `retired_without_task` joins `retired_feed_items` against `tasks` across a
//! whole epic subtree, which a subscription's `WHERE` clause cannot express at
//! all, so the join runs in Rust over the rows a standing subscription already
//! holds ([`SharedRows::retired_without_task`]) rather than over SQLite.

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::models::EpicId;
use crate::store::SharedRetiredFeedItemReader;

use super::SharedRows;

/// Reads from the live view of this board's subscriptions.
///
/// Infallible in practice, like [`super::SubscriptionUsageReads`] — the rows
/// are already decoded and in memory — but keeps the `Result` its port
/// declares, so callers have one code path regardless of backend.
pub struct SubscriptionRetiredFeedItemReads {
    rows: Arc<SharedRows>,
}

impl SubscriptionRetiredFeedItemReads {
    pub fn new(rows: Arc<SharedRows>) -> Self {
        Self { rows }
    }
}

#[async_trait]
impl SharedRetiredFeedItemReader for SubscriptionRetiredFeedItemReads {
    async fn retired_without_task(
        &self,
        feed_epic_id: EpicId,
        external_ids: &[String],
    ) -> Result<Vec<String>> {
        Ok(self.rows.retired_without_task(feed_epic_id, external_ids))
    }
}
