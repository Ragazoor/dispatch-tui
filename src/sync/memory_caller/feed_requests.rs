//! Feed requests: the in-memory twin of `spacetime/module/src/feed_request.rs`.
//!
//! Every decision is the module's own pure function, called directly; what is
//! reimplemented here is only the row storage and the order the module's
//! reducers apply them in.

use anyhow::Result;

use dispatch_spacetime_module as module;

use crate::models::{EpicId, FeedRequestId};

use super::super::writes::ReducerOutcome;
use super::{MemoryReducerCaller, Tables};

fn outcome(result: Result<(), String>) -> Result<ReducerOutcome> {
    Ok(match result {
        Ok(()) => ReducerOutcome::Applied(vec![]),
        Err(why) => ReducerOutcome::Refused(why),
    })
}

/// Mirrors the module's `find_feed_request`.
fn find_feed(tables: &Tables, id: i64) -> Result<module::FeedRequest, String> {
    tables
        .feed_requests
        .get(&id)
        .cloned()
        .ok_or_else(|| format!("feed request {id} does not exist"))
}

impl MemoryReducerCaller {
    fn write_feed_request(&self, tables: &mut Tables, mut row: module::FeedRequest) -> i64 {
        let id = Self::assign_id(row.id, &mut tables.next_feed_request_id);
        row.id = id;
        tables.feed_requests.insert(id, row.clone());
        self.rows.upsert_feed_request(&row.into());
        id
    }

    /// Mirrors the module's `prune_feed_requests`.
    fn prune_feed_requests(&self, tables: &mut Tables) {
        let micros = self.clock.now().timestamp_micros();
        let rows: Vec<module::FeedRequest> = tables.feed_requests.values().cloned().collect();
        for id in module::prunable_feed_requests(
            &module::request_retention_cutoff(micros),
            &module::stale_refresh_cutoff(micros),
            &rows,
        ) {
            tables.feed_requests.remove(&id);
            self.rows.remove_feed_request(id);
        }
    }

    /// Mirrors the module's `write_teardown_request_if_owed`.
    pub(super) fn write_teardown_request_if_owed(
        &self,
        tables: &mut Tables,
        task: &module::Task,
        ingest_host: &str,
    ) {
        let holder_is_ours = tables
            .hosts
            .get(&task.host)
            .is_some_and(|h| h.owner == self.sender);
        if holder_is_ours && module::teardown_request_owed(task, ingest_host) {
            self.prune_feed_requests(tables);
            let row = module::new_teardown_request(task, &self.sender, &self.now());
            self.write_feed_request(tables, row);
        }
    }

    pub(super) fn apply_request_feed_refresh(
        &self,
        epic: EpicId,
        host: String,
    ) -> Result<ReducerOutcome> {
        outcome(self.request_feed_refresh_body(epic.0, &host))
    }

    fn request_feed_refresh_body(&self, epic_id: i64, host: &str) -> Result<(), String> {
        let mut tables = self.lock();
        self.acting_for(&tables, host)?;
        let epic = tables.epics.get(&epic_id).cloned();
        let owner_row = tables
            .poll_owners
            .values()
            .find(|p| p.scope == "epic" && p.scope_id == epic_id)
            .cloned();
        let owner_host_owner = owner_row
            .as_ref()
            .and_then(|p| tables.hosts.get(&p.host))
            .map(|h| h.owner.clone());
        let open_for_epic = owner_row.as_ref().is_some_and(|p| {
            tables.feed_requests.values().any(|r| {
                r.host == p.host
                    && r.kind == module::FEED_REQUEST_REFRESH
                    && r.epic_id == epic_id
                    && module::feed_request_is_open(&r.state)
            })
        });
        let decision = module::decide_feed_refresh(
            epic.as_ref(),
            host,
            owner_row.as_ref().map(|p| p.host.as_str()),
            owner_host_owner.as_deref(),
            &self.sender,
            open_for_epic,
        )?;
        if decision == module::RefreshDecision::Write {
            self.prune_feed_requests(&mut tables);
            let owner = owner_row.map(|p| p.host).unwrap_or_default();
            let row = module::new_refresh_request(epic_id, &owner, &self.sender, &self.now());
            self.write_feed_request(&mut tables, row);
        }
        Ok(())
    }

    pub(super) fn apply_claim_feed_request(
        &self,
        id: FeedRequestId,
        host: String,
    ) -> Result<ReducerOutcome> {
        outcome((|| {
            let mut tables = self.lock();
            let row = find_feed(&tables, id.0)?;
            module::validate_feed_request_claim(&row, &host)?;
            self.acting_for(&tables, &host)?;
            self.write_feed_request(
                &mut tables,
                module::claim_feed_request_row(row, &self.now()),
            );
            Ok(())
        })())
    }

    pub(super) fn apply_report_feed_request_done(
        &self,
        id: FeedRequestId,
        reporter: String,
    ) -> Result<ReducerOutcome> {
        outcome((|| {
            let mut tables = self.lock();
            let row = find_feed(&tables, id.0)?;
            module::validate_feed_request_report(&row, &reporter)?;
            self.acting_for(&tables, &reporter)?;
            self.write_feed_request(&mut tables, module::done_feed_request_row(row, &self.now()));
            Ok(())
        })())
    }

    pub(super) fn apply_report_feed_request_failed(
        &self,
        id: FeedRequestId,
        reporter: String,
        reason: String,
    ) -> Result<ReducerOutcome> {
        outcome((|| {
            let mut tables = self.lock();
            let row = find_feed(&tables, id.0)?;
            module::validate_feed_request_report(&row, &reporter)?;
            self.acting_for(&tables, &reporter)?;
            self.write_feed_request(
                &mut tables,
                module::failed_feed_request_row(row, &reason, &self.now()),
            );
            Ok(())
        })())
    }
}
