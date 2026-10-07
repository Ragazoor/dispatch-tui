//! Where a shared-table usage READ goes, when it does not go to SQLite.
//!
//! Spec: none — `usage_events` is append-only telemetry with no
//! user-observable rule beyond "recorded" (Phase 11, task #4915).
//!
//! The same shape as `learning_reads`, for the same reason: `query_usage`
//! groups and counts rows, which a subscription's `WHERE` clause cannot do at
//! all, so the aggregation runs in Rust over the rows a standing subscription
//! already holds ([`SharedRows::usage_summary`]) rather than over SQLite.

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::models::UsageSummary;
use crate::store::{SharedUsageReader, UsageQuery};

use super::SharedRows;

/// Reads from the live view of this board's subscriptions.
///
/// Infallible in practice, like [`super::SubscriptionLearningReads`] — the
/// rows are already decoded and in memory — but keeps the `Result` its port
/// declares, so callers have one code path regardless of backend.
pub struct SubscriptionUsageReads {
    rows: Arc<SharedRows>,
}

impl SubscriptionUsageReads {
    pub fn new(rows: Arc<SharedRows>) -> Self {
        Self { rows }
    }
}

#[async_trait]
impl SharedUsageReader for SubscriptionUsageReads {
    async fn query_usage(&self, query: &UsageQuery) -> Result<Vec<UsageSummary>> {
        Ok(self.rows.usage_summary(query))
    }
}
