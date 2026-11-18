use anyhow::Result;

use crate::models::{UsageEvent, UsageSummary};
use crate::sync::{decode, encode};

use super::super::{Store, UsageQuery};

/// The cap the board sends with every event. The store owns retention and only
/// lets a cap raise its own, so this is the module's `USAGE_EVENT_CAP`, which
/// a test pins.
const USAGE_CAP_HINT: i64 = 100_000;

#[cfg(test)]
#[test]
fn the_cap_hint_is_the_modules_retention_cap() {
    assert_eq!(USAGE_CAP_HINT, dispatch_spacetime_module::USAGE_EVENT_CAP);
}

#[async_trait::async_trait]
impl crate::store::UsageStore for Store {
    /// Append-only telemetry with no id readback, no patch, no delete. The
    /// retention cap is the module's (`USAGE_EVENT_CAP`); the reducer still
    /// takes one, so the board sends that constant.
    async fn record_usage_event(&self, event: &UsageEvent) -> Result<()> {
        let row = encode::usage_event_row(event, &self.now());
        self.caller
            .record_usage_event(row, USAGE_CAP_HINT)
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
