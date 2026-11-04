//! Feed requests: reads from this host's subscribed rows, writes as the
//! module's request reducers, always naming THIS host.

use anyhow::Result;

use crate::models::{EpicId, FeedRequest, FeedRequestId};
use crate::store::{FeedRefreshAnswer, FeedRequestStore};
use crate::sync::ReducerOutcome;

use super::Store;

/// [`ReducerOutcome::won`] for a host-side call, and why it lost in the log.
fn won_logged(outcome: ReducerOutcome, what: &str, id: FeedRequestId) -> bool {
    match outcome {
        ReducerOutcome::Applied(_) => true,
        ReducerOutcome::Refused(why) => {
            tracing::info!(request = id.0, "the shared store refused the {what}: {why}");
            false
        }
    }
}

#[async_trait::async_trait]
impl FeedRequestStore for Store {
    async fn feed_requests(&self) -> Vec<FeedRequest> {
        self.rows.feed_requests()
    }

    async fn request_feed_refresh(&self, epic: EpicId) -> Result<FeedRefreshAnswer> {
        Ok(
            match self
                .caller
                .request_feed_refresh(epic, self.host.clone())
                .await?
            {
                ReducerOutcome::Applied(_) => FeedRefreshAnswer::Requested,
                ReducerOutcome::Refused(why) => FeedRefreshAnswer::Refused(why),
            },
        )
    }

    async fn claim_feed_request(&self, id: FeedRequestId) -> Result<bool> {
        Ok(won_logged(
            self.caller
                .claim_feed_request(id, self.host.clone())
                .await?,
            "feed request claim",
            id,
        ))
    }

    async fn report_feed_request_done(&self, id: FeedRequestId) -> Result<bool> {
        Ok(won_logged(
            self.caller
                .report_feed_request_done(id, self.host.clone())
                .await?,
            "feed request done report",
            id,
        ))
    }

    async fn report_feed_request_failed(&self, id: FeedRequestId, reason: &str) -> Result<bool> {
        Ok(won_logged(
            self.caller
                .report_feed_request_failed(id, self.host.clone(), reason.to_string())
                .await?,
            "feed request failure report",
            id,
        ))
    }
}
