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
        use crate::sync::decode;
        fn kept<R, T, E: std::fmt::Display>(
            rows: Vec<R>,
            decode: impl Fn(&R) -> Result<T, E>,
        ) -> Vec<T> {
            rows.iter()
                .filter_map(|row| decode(row).map_err(|e| tracing::warn!("{e}")).ok())
                .collect()
        }
        let orchestrations = kept(self.rows.epic_orchestrations(), decode::epic_orchestration);
        let mut open_items: Vec<crate::models::WaitingItem> =
            kept(self.rows.waiting_items(), decode::waiting_item)
                .into_iter()
                .filter(crate::models::WaitingItem::is_open)
                .collect();
        open_items.sort_by_key(|i| i.id);
        let mut log: Vec<crate::models::OrchestratorLogEntry> =
            kept(self.rows.orchestrator_log(), decode::orchestrator_log_entry);
        log.sort_by_key(|e| std::cmp::Reverse(e.id));
        let mut shown: std::collections::HashMap<crate::models::EpicId, usize> =
            std::collections::HashMap::new();
        log.retain(|e| {
            let n = shown.entry(e.epic_id).or_default();
            *n += 1;
            *n <= crate::models::ORCHESTRATOR_LOG_SHOWN
        });
        crate::models::OrchestrationBoard {
            orchestrations,
            orchestrators: kept(self.rows.orchestrators(), decode::orchestrator),
            open_items,
            log,
            ready_marks: kept(self.rows.ready_marks(), decode::ready_mark),
        }
    }
}
