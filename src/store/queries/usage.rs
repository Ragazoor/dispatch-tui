use anyhow::Result;

use crate::models::{UsageEvent, UsageSummary};
use crate::sync::{decode, encode};

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
    /// clause cannot express — so the store aggregates on demand
    /// (`own_usage_summary`) and the board holds no usage rows. A view over
    /// usage would re-run on every recorded event (`docs/architecture.md`,
    /// "Read enforcement: per-caller views").
    async fn query_usage(&self, q: &UsageQuery) -> Result<Vec<UsageSummary>> {
        let limit = q.limit.unwrap_or(50).clamp(1, 500) as i64;
        let rows = self
            .caller
            .own_usage_summary(
                q.category.clone(),
                q.actor.clone(),
                q.since.map(encode::stamp),
                limit,
            )
            .await?;
        rows.iter()
            .map(|row| decode::usage_summary_row(row).map_err(anyhow::Error::from))
            .collect()
    }
}
