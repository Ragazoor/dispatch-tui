//! PR state (store-split M6): the in-memory twin of
//! `spacetime/module/src/pr_status.rs`.
//!
//! Every decision is the module's own pure function, called directly; what is
//! reimplemented here is the order the module's reducers apply them in.

use anyhow::Result;

use dispatch_spacetime_module as module;

use crate::models::TaskId;

use super::super::writes::ReducerOutcome;
use super::MemoryReducerCaller;

fn outcome(result: Result<(), String>) -> Result<ReducerOutcome> {
    Ok(match result {
        Ok(()) => ReducerOutcome::Applied(vec![]),
        Err(why) => ReducerOutcome::Refused(why),
    })
}

impl MemoryReducerCaller {
    /// Mirrors the module's `report_pr_status`.
    pub(super) fn apply_report_pr_status(
        &self,
        task: TaskId,
        host: String,
        state: String,
        review_decision: String,
    ) -> Result<ReducerOutcome> {
        outcome((|| {
            let mut tables = self.lock();
            self.acting_for(&tables, &host)?;
            module::validate_pr_status_report(&state, &review_decision)?;
            let Some(row) = tables.tasks.get(&task.0).cloned() else {
                return Ok(());
            };
            let prior_status = row.status.clone();
            let epic_id = row.epic_id;
            let stamp = self.now();
            let Some(mut next) = module::apply_pr_status(row, &state, &review_decision, &stamp)
            else {
                return Ok(());
            };
            if module::should_respawn_phoenix(&prior_status, &next)
                && self.respawn_phoenix_successor_of(&mut tables, &next, &stamp)
            {
                next.phoenix = false;
            }
            let merged = next.status == "done";
            self.write_task(&mut tables, next)?;
            if merged {
                self.recalculate_epic_chain(&mut tables, epic_id);
            }
            Ok(())
        })())
    }

    /// Mirrors the module's `report_pr_unreadable`.
    pub(super) fn apply_report_pr_unreadable(
        &self,
        task: TaskId,
        host: String,
        _reason: String,
    ) -> Result<ReducerOutcome> {
        outcome((|| {
            let mut tables = self.lock();
            self.acting_for(&tables, &host)?;
            let Some(row) = tables.tasks.get(&task.0).cloned() else {
                return Ok(());
            };
            if let Some(next) = module::apply_pr_unreadable(row, &self.now()) {
                self.write_task(&mut tables, next)?;
            }
            Ok(())
        })())
    }
}
