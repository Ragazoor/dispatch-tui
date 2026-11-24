//! An orchestrated epic's orchestrator on one machine. Mirrors the module's
//! `orchestrator.rs`, whose checks it calls.

use anyhow::Result;

use dispatch_spacetime_module as module;

use crate::models::EpicId;

use super::super::writes::ReducerOutcome;
use super::{MemoryReducerCaller, Tables};

impl MemoryReducerCaller {
    pub(super) fn apply_ensure_orchestrator(
        &self,
        epic: EpicId,
        host: String,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        if let Err(why) = self.acting_for(&tables, &host) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let seed = self.clock.now().timestamp_micros() as u64;
        match module::check_ensure_orchestrator(&*tables, epic.0, &host, seed) {
            Ok(Some(row)) => {
                let id = Self::assign_id(0, &mut tables.next_orchestrator_id);
                self.write_orchestrator(&mut tables, module::Orchestrator { id, ..row });
                Ok(ReducerOutcome::Applied(vec![]))
            }
            Ok(None) => Ok(ReducerOutcome::Applied(vec![])),
            Err(why) => Ok(ReducerOutcome::Refused(why)),
        }
    }

    pub(super) fn apply_start_orchestrator_session(
        &self,
        id: i64,
        host: String,
        resumed: bool,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        if let Err(why) = self.acting_for(&tables, &host) {
            return Ok(ReducerOutcome::Refused(why));
        }
        match module::check_start_orchestrator_session(&*tables, id, &host) {
            Ok(row) => {
                self.insert_log_entry(
                    &mut tables,
                    module::session_entry(&row, module::start_entry_text(resumed)),
                );
                let row = module::Orchestrator {
                    started_at: self.now(),
                    ..row
                };
                self.write_orchestrator(&mut tables, row);
                Ok(ReducerOutcome::Applied(vec![]))
            }
            Err(why) => Ok(ReducerOutcome::Refused(why)),
        }
    }

    pub(super) fn apply_end_orchestrator_session(
        &self,
        id: i64,
        host: String,
        died: bool,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        if let Err(why) = self.acting_for(&tables, &host) {
            return Ok(ReducerOutcome::Refused(why));
        }
        match module::check_end_orchestrator_session(&*tables, id, &host, died) {
            Ok(row) => {
                self.insert_log_entry(
                    &mut tables,
                    module::session_entry(&row, module::end_entry_text(died)),
                );
                self.write_orchestrator(&mut tables, row);
                Ok(ReducerOutcome::Applied(vec![]))
            }
            Err(why) => Ok(ReducerOutcome::Refused(why)),
        }
    }

    pub(super) fn apply_orchestrator_turn_completed(&self, name: String) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let row = match module::check_orchestrator_turn(&*tables, &name) {
            Ok(row) => row,
            Err(why) => return Ok(ReducerOutcome::Refused(why)),
        };
        if let Err(why) = self.acting_for(&tables, &row.host) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let row = module::Orchestrator {
            last_turn_at: self.now(),
            ..row
        };
        self.write_orchestrator(&mut tables, row);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    fn write_orchestrator(&self, tables: &mut Tables, row: module::Orchestrator) {
        tables.orchestrators.insert(row.id, row.clone());
        self.rows.upsert_orchestrator(&row.into());
    }

    /// Mirrors the module's `delete_orchestrators_of`, called with the
    /// switch's delete.
    pub(super) fn delete_orchestrators_of(&self, tables: &mut Tables, epic_id: i64) {
        let ids: Vec<i64> = tables
            .orchestrators
            .values()
            .filter(|o| o.epic_id == epic_id)
            .map(|o| o.id)
            .collect();
        for id in ids {
            tables.orchestrators.remove(&id);
            self.rows.remove_orchestrator(id);
        }
    }
}
