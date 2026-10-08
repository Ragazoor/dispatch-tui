use anyhow::Result;

use crate::models::{UsageEvent, UsageSummary};

use super::super::{Store, UsageCap, UsageQuery};

#[async_trait::async_trait]
impl crate::store::UsageStore for Store {
    async fn record_usage_event_with_cap(&self, event: &UsageEvent, cap: UsageCap) -> Result<()> {
        let writer = self.shared_writer()?;
        writer.record_usage_event_with_cap(event, cap).await
    }

    async fn query_usage(&self, q: &UsageQuery) -> Result<Vec<UsageSummary>> {
        let reader = self.shared_usage_reader()?;
        reader.query_usage(q).await
    }
}
