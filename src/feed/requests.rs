//! Serving feed requests: the work another host asked this one to do.
//!
//! Spec: `docs/specs/store-split.allium` (`FeedRequest`,
//! `HostClaimsFeedRequest`, `StaleDeleteRequestsTeardownOnTheHoldingHost`) and
//! `docs/specs/feeds.allium` (`SerialisedFeedCycle`, scope across hosts).
//!
//! A `refresh` row asks this host, the epic's poll owner, to run one feed
//! cycle. A `teardown` row asks it, the host that dispatched a feed task the
//! stale delete removed, to release the task's worktree and window. Both are
//! served from the feed runner's tick, so a refresh needs no interval to
//! elapse and the one runner owns the per-process cycle guard.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use crate::models::{FeedRequest, FeedRequestId, FeedRequestKind};
use crate::process::ProcessRunner;
use crate::store::TaskStore;

use std::sync::Arc;

/// The ids of the requests this process has handed out, shared with the tasks
/// that serve them so a failed claim or report can hand its row back.
#[derive(Clone, Default)]
pub(super) struct ServedRequests(Arc<Mutex<HashSet<i64>>>);

impl ServedRequests {
    fn ids(&self) -> std::sync::MutexGuard<'_, HashSet<i64>> {
        // A poisoned set is still a set of ids: keep serving.
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Hand `id` back: its claim or report never reached the store, so the row
    /// is still open there and the next tick must see it again.
    pub(super) fn forget(&self, id: FeedRequestId) {
        self.ids().remove(&id.0);
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.ids().len()
    }
}

/// The rows this host must serve now: open, naming a host this board serves,
/// requested by this host's owner, and not served already by this process.
/// A row another identity wrote is logged and left alone (the module's check
/// is the first line, this the second). Ids whose rows are no longer open are
/// dropped from `served`, so the set follows the table instead of growing.
pub(super) fn rows_to_serve(
    rows: Vec<FeedRequest>,
    host_id: &str,
    owner: &str,
    served: &ServedRequests,
) -> Vec<FeedRequest> {
    let mut served = served.ids();
    served.retain(|id| rows.iter().any(|r| r.id.0 == *id && r.state.is_open()));
    let mut out = Vec::new();
    for row in rows {
        if !row.state.is_open() || row.host != host_id || served.contains(&row.id.0) {
            continue;
        }
        if row.requested_by != owner {
            tracing::warn!(
                request = row.id.0,
                requested_by = %row.requested_by,
                "ignoring a feed request this host's owner did not make"
            );
            served.insert(row.id.0);
            continue;
        }
        served.insert(row.id.0);
        out.push(row);
    }
    out
}

/// Serve every teardown row: claim it, tear the task's window and worktree down
/// (one repo's rows one after another, so they do not contend on its index
/// lock), and report each row done or failed.
pub(super) async fn serve_teardowns(
    db: Arc<dyn TaskStore>,
    runner: Arc<dyn ProcessRunner>,
    rows: Vec<FeedRequest>,
    served: ServedRequests,
) {
    let mut by_repo: HashMap<String, Vec<FeedRequest>> = HashMap::new();
    for row in rows {
        debug_assert_eq!(row.kind, FeedRequestKind::Teardown);
        match db.claim_feed_request(row.id).await {
            Ok(true) => by_repo.entry(row.repo_path.clone()).or_default().push(row),
            Ok(false) => {}
            Err(err) => {
                tracing::warn!(request = row.id.0, "feed teardown: claim not sent: {err:#}");
                served.forget(row.id);
            }
        }
    }
    let mut handles = Vec::new();
    for (_repo, rows) in by_repo {
        let runner = Arc::clone(&runner);
        handles.push(tokio::task::spawn_blocking(move || {
            rows.into_iter()
                .map(|row| {
                    let result = crate::dispatch::teardown_task(
                        &row.repo_path,
                        row.worktree.as_deref(),
                        row.tmux_window.as_ref(),
                        &*runner,
                    );
                    (row.id, result.map_err(|f| f.to_string()))
                })
                .collect::<Vec<_>>()
        }));
    }
    for handle in handles {
        let results = match handle.await {
            Ok(results) => results,
            Err(err) => {
                tracing::warn!("feed teardown: teardown thread did not complete: {err}");
                continue;
            }
        };
        for (id, result) in results {
            let reported = match result {
                Ok(()) => db.report_feed_request_done(id).await,
                Err(reason) => {
                    tracing::warn!(request = id.0, "feed teardown failed: {reason}");
                    db.report_feed_request_failed(id, &reason).await
                }
            };
            if let Err(err) = reported {
                tracing::warn!(request = id.0, "feed teardown: report not sent: {err:#}");
                served.forget(id);
            }
        }
    }
}
