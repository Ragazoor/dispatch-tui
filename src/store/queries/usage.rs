use anyhow::Result;

use crate::models::{UsageEvent, UsageSummary};
use crate::sync::encode;

use super::super::{Store, UsageCap, UsageQuery};

#[async_trait::async_trait]
impl crate::store::UsageStore for Store {
    /// Append-only telemetry with no id readback, no patch, no delete. `cap`
    /// is carried on every call rather than read from a stored default, so
    /// the reducer's prune stays in step with whatever `UsageCap` the caller
    /// passed.
    async fn record_usage_event_with_cap(&self, event: &UsageEvent, cap: UsageCap) -> Result<()> {
        let row = encode::usage_event_row(event, &self.now());
        self.caller
            .record_usage_event(row, cap.value() as i64)
            .await?
            .applied()
    }

    /// `query_usage` groups and counts rows — a shape a subscription's `WHERE`
    /// clause cannot express — so the aggregation runs in Rust over the rows a
    /// standing subscription already holds.
    async fn query_usage(&self, q: &UsageQuery) -> Result<Vec<UsageSummary>> {
        Ok(self.rows.usage_summary(q))
    }
}
