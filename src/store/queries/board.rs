//! The board's card-read handle (`crate::store::BoardReads`) over the same
//! rows every other read answers from.

use async_trait::async_trait;

use crate::models::{PollScopeId, TaskId};

use super::super::Store;

/// Every method is infallible by signature — the rows are already decoded and
/// in memory. A board that is not connected holds no rows and answers empty;
/// it does not answer an error, because "disconnected" is the connection's
/// state to report (`sync.allium`'s `ConnectionIndicator`) and reporting it
/// again per read would put the same outage on screen eight times.
#[async_trait]
impl crate::store::BoardReads for Store {
    async fn poll_owner(&self, target: PollScopeId) -> Option<String> {
        self.rows.poll_owner(target).map(|row| row.host)
    }

    async fn host_owner(&self, host_id: &str) -> Option<String> {
        self.rows.host_owner(host_id)
    }

    async fn unseen_task_ids(&self) -> Vec<TaskId> {
        self.rows.unseen_task_ids()
    }

    async fn revision(&self) -> u64 {
        self.rows.generation()
    }

    async fn orchestration_board(&self) -> crate::models::OrchestrationBoard {
        use super::decoded;
        use crate::sync::decode;
        let open = self
            .rows
            .waiting_items()
            .into_iter()
            .filter(|i| i.resolved_at.is_empty())
            .collect();
        // Newest first, cut per epic before anything is decoded: the log
        // only grows, the board shows the newest few.
        let mut shown: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
        let newest = self
            .rows
            .orchestrator_log()
            .into_iter()
            .rev()
            .filter(|e| {
                let n = shown.entry(e.epic_id).or_default();
                *n += 1;
                *n <= crate::models::ORCHESTRATOR_LOG_SHOWN
            })
            .collect();
        crate::models::OrchestrationBoard {
            orchestrations: decoded(self.rows.epic_orchestrations(), decode::epic_orchestration),
            orchestrators: decoded(self.rows.orchestrators(), decode::orchestrator),
            open_items: decoded(open, decode::waiting_item),
            log: decoded(newest, decode::orchestrator_log_entry),
            ready_marks: decoded(self.rows.ready_marks(), decode::ready_mark),
        }
    }
}
