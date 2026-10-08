//! The board's card-read handle (`crate::store::BoardReads`) over the same
//! rows every other read answers from.

use anyhow::Result;
use async_trait::async_trait;

use crate::models::PollScopeId;

use super::super::Store;

/// Every method is infallible in practice — the rows are already decoded and
/// in memory. A board that is not connected holds no rows and answers empty;
/// it does not answer an error, because "disconnected" is the connection's
/// state to report (`sync.allium`'s `ConnectionIndicator`) and reporting it
/// again per read would put the same outage on screen eight times.
#[async_trait]
impl crate::store::BoardReads for Store {
    async fn poll_owner(&self, target: PollScopeId) -> Result<Option<String>> {
        Ok(self.rows.poll_owner(target).map(|row| row.host))
    }

    async fn revision(&self) -> u64 {
        self.rows.generation()
    }
}
