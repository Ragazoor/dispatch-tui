//! The orchestrator switch and the ready mark. Mirrors the module's
//! `orchestration.rs`, whose checks it calls.

use anyhow::Result;

use dispatch_spacetime_module as module;

use crate::models::{EpicId, TaskId};

use super::super::writes::ReducerOutcome;
use super::{MemoryReducerCaller, Tables};

impl MemoryReducerCaller {
    pub(super) fn apply_switch_orchestrator_on(
        &self,
        epic: EpicId,
        start_rule: Option<String>,
        max_parallel: Option<i64>,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let row = match module::check_switch_orchestrator_on(
            &*tables,
            epic.0,
            start_rule.as_deref(),
            max_parallel,
        ) {
            Ok(row) => row,
            Err(why) => return Ok(ReducerOutcome::Refused(why)),
        };
        let row = module::EpicOrchestration {
            switched_on_at: self.now(),
            ..row
        };
        self.write_orchestration(&mut tables, row);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_change_orchestrator_settings(
        &self,
        epic: EpicId,
        start_rule: Option<String>,
        max_parallel: Option<i64>,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let row = match module::check_change_orchestrator_settings(
            &*tables,
            epic.0,
            start_rule.as_deref(),
            max_parallel,
        ) {
            Ok(row) => row,
            Err(why) => return Ok(ReducerOutcome::Refused(why)),
        };
        self.write_orchestration(&mut tables, row);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_switch_orchestrator_off(&self, epic: EpicId) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        if let Err(why) = module::check_switch_orchestrator_off(&*tables, epic.0) {
            return Ok(ReducerOutcome::Refused(why));
        }
        self.delete_orchestration(&mut tables, epic.0);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_mark_task_ready(
        &self,
        task: TaskId,
        marked_by: String,
        agent_name: String,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let row = match module::check_mark_task_ready(&*tables, task.0, &marked_by, &agent_name) {
            Ok(row) => row,
            Err(why) => return Ok(ReducerOutcome::Refused(why)),
        };
        if let Some(row) = row {
            let row = module::ReadyMark {
                marked_at: self.now(),
                ..row
            };
            self.write_ready_mark(&mut tables, row);
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_clear_ready_mark(&self, task: TaskId) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        if let Err(why) = module::check_clear_ready_mark(&*tables, task.0) {
            return Ok(ReducerOutcome::Refused(why));
        }
        self.delete_ready_mark(&mut tables, task.0);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    fn write_orchestration(&self, tables: &mut Tables, row: module::EpicOrchestration) {
        tables.epic_orchestrations.insert(row.epic_id, row.clone());
        self.rows.upsert_epic_orchestration(&row.into());
    }

    fn write_ready_mark(&self, tables: &mut Tables, row: module::ReadyMark) {
        tables.ready_marks.insert(row.task_id, row.clone());
        self.rows.upsert_ready_mark(&row.into());
    }

    pub(super) fn delete_orchestration(&self, tables: &mut Tables, epic_id: i64) {
        if tables.epic_orchestrations.remove(&epic_id).is_some() {
            self.rows.remove_epic_orchestration(EpicId(epic_id));
        }
        self.delete_orchestrators_of(tables, epic_id);
    }

    pub(super) fn delete_ready_mark(&self, tables: &mut Tables, task_id: i64) {
        if tables.ready_marks.remove(&task_id).is_some() {
            self.rows.remove_ready_mark(TaskId(task_id));
        }
    }

    /// Mirrors the module's `drop_ready_mark_on_leaving_backlog`.
    pub(super) fn drop_ready_mark_on_leaving_backlog(
        &self,
        tables: &mut Tables,
        prior: &module::Task,
        next: &module::Task,
    ) {
        if module::leaves_backlog(&prior.status, &next.status) {
            self.delete_ready_mark(tables, next.id);
        }
    }

    /// Mirrors the module's `switch_off_on_reparent`.
    pub(super) fn switch_off_on_reparent(
        &self,
        tables: &mut Tables,
        epic_id: i64,
        new_parent: i64,
    ) {
        for id in module::switches_off_on_reparent(&*tables, epic_id, new_parent) {
            self.delete_orchestration(tables, id);
        }
    }
}
