//! An orchestrated epic's memory: its log, its notes and its waiting items.
//! Mirrors the module's `orchestrator_memory.rs`, whose checks it calls.

use anyhow::Result;

use dispatch_spacetime_module as module;

use crate::models::TaskId;

use super::super::writes::ReducerOutcome;
use super::{MemoryReducerCaller, Tables};

impl MemoryReducerCaller {
    pub(super) fn apply_orchestrator_request_dispatch(
        &self,
        task: TaskId,
        requester: String,
        name: String,
        reason: String,
    ) -> Result<ReducerOutcome> {
        let entry = {
            let tables = self.lock();
            match module::check_orchestrator_start(&*tables, &name, &requester, task.0, &reason) {
                Ok(entry) => entry,
                Err(why) => return Ok(ReducerOutcome::Refused(why)),
            }
        };
        let outcome = self.apply_request_dispatch(task, requester, "mcp".into(), String::new())?;
        if let ReducerOutcome::Applied(_) = &outcome {
            let mut tables = self.lock();
            self.insert_log_entry(&mut tables, entry);
        }
        Ok(outcome)
    }

    pub(super) fn apply_record_orchestrator_answer(
        &self,
        name: String,
        task: TaskId,
        question: String,
        answer: String,
        reason: String,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let entry = match module::check_orchestrator_answer(
            &*tables, &name, task.0, &question, &answer, &reason,
        ) {
            Ok(entry) => entry,
            Err(why) => return Ok(ReducerOutcome::Refused(why)),
        };
        if let Err(why) = self.acting_for(&tables, &entry.host) {
            return Ok(ReducerOutcome::Refused(why));
        }
        self.insert_log_entry(&mut tables, entry);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_record_orchestrator_trouble(
        &self,
        name: String,
        task: TaskId,
        trouble: String,
        text: String,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let entry =
            match module::check_orchestrator_trouble(&*tables, &name, task.0, &trouble, &text) {
                Ok(entry) => entry,
                Err(why) => return Ok(ReducerOutcome::Refused(why)),
            };
        if let Err(why) = self.acting_for(&tables, &entry.host) {
            return Ok(ReducerOutcome::Refused(why));
        }
        self.insert_log_entry(&mut tables, entry);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_record_orchestrator_summary(
        &self,
        name: String,
        text: String,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let entry = match module::check_orchestrator_summary(&*tables, &name, &text) {
            Ok(entry) => entry,
            Err(why) => return Ok(ReducerOutcome::Refused(why)),
        };
        if let Err(why) = self.acting_for(&tables, &entry.host) {
            return Ok(ReducerOutcome::Refused(why));
        }
        self.insert_log_entry(&mut tables, entry);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_raise_waiting_item(
        &self,
        name: String,
        task: i64,
        about: String,
        tried: String,
        recommendation: String,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let raised = match module::check_raise_waiting_item(
            &*tables,
            &name,
            task,
            &about,
            &tried,
            &recommendation,
        ) {
            Ok(raised) => raised,
            Err(why) => return Ok(ReducerOutcome::Refused(why)),
        };
        if let Err(why) = self.acting_for(&tables, &raised.0.host) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let id = self.insert_raised(&mut tables, raised);
        Ok(ReducerOutcome::Applied(vec![id]))
    }

    pub(super) fn apply_raise_orchestrator_down_item(
        &self,
        id: i64,
        host: String,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        if let Err(why) = self.acting_for(&tables, &host) {
            return Ok(ReducerOutcome::Refused(why));
        }
        match module::check_orchestrator_down_item(&*tables, id, &host) {
            Ok(Some(raised)) => {
                self.insert_raised(&mut tables, raised);
                Ok(ReducerOutcome::Applied(vec![]))
            }
            Ok(None) => Ok(ReducerOutcome::Applied(vec![])),
            Err(why) => Ok(ReducerOutcome::Refused(why)),
        }
    }

    pub(super) fn apply_resolve_waiting_item(&self, id: i64) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        match module::check_resolve_waiting_item(&*tables, id) {
            Ok(Some(item)) => {
                self.resolve_all(&mut tables, vec![item]);
                Ok(ReducerOutcome::Applied(vec![]))
            }
            Ok(None) => Ok(ReducerOutcome::Applied(vec![])),
            Err(why) => Ok(ReducerOutcome::Refused(why)),
        }
    }

    pub(super) fn apply_write_epic_note(
        &self,
        name: String,
        text: String,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let note = match module::check_write_epic_note(&*tables, &name, &text) {
            Ok(note) => note,
            Err(why) => return Ok(ReducerOutcome::Refused(why)),
        };
        if let Err(why) = self.acting_for_orchestrator(&tables, &name) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let id = Self::assign_id(0, &mut tables.next_epic_note_id);
        let note = module::EpicNote {
            id,
            written_at: self.now(),
            ..note
        };
        self.write_epic_note_row(&mut tables, note);
        Ok(ReducerOutcome::Applied(vec![id]))
    }

    pub(super) fn apply_change_epic_note(
        &self,
        name: String,
        id: i64,
        text: Option<String>,
        done: Option<bool>,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        match module::check_change_epic_note(&*tables, &name, id, text.as_deref(), done) {
            Ok(note) => {
                if let Err(why) = self.acting_for_orchestrator(&tables, &name) {
                    return Ok(ReducerOutcome::Refused(why));
                }
                self.write_epic_note_row(&mut tables, note);
                Ok(ReducerOutcome::Applied(vec![]))
            }
            Err(why) => Ok(ReducerOutcome::Refused(why)),
        }
    }

    pub(super) fn apply_remove_epic_note(&self, name: String, id: i64) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        if let Err(why) = module::check_remove_epic_note(&*tables, &name, id)
            .and_then(|()| self.acting_for_orchestrator(&tables, &name))
        {
            return Ok(ReducerOutcome::Refused(why));
        }
        tables.epic_notes.remove(&id);
        self.rows.remove_epic_note(id);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    /// Mirrors the module's `require_acting_for_orchestrator`.
    fn acting_for_orchestrator(&self, tables: &Tables, name: &str) -> Result<(), String> {
        let o = module::orchestrator_writing(tables, name)?;
        self.acting_for(tables, &o.host)
    }

    /// Mirrors the module's `insert_log_entry`.
    pub(super) fn insert_log_entry(
        &self,
        tables: &mut Tables,
        entry: module::OrchestratorLogEntry,
    ) {
        let id = Self::assign_id(0, &mut tables.next_log_entry_id);
        let row = module::OrchestratorLogEntry {
            id,
            at: self.now(),
            ..entry
        };
        tables.orchestrator_log.insert(id, row.clone());
        self.rows.upsert_orchestrator_log_entry(&row.into());
    }

    fn insert_raised(
        &self,
        tables: &mut Tables,
        (item, entry): (module::WaitingItem, module::OrchestratorLogEntry),
    ) -> i64 {
        let id = Self::assign_id(0, &mut tables.next_waiting_item_id);
        self.write_waiting_item(
            tables,
            module::WaitingItem {
                id,
                raised_at: self.now(),
                ..item
            },
        );
        self.insert_log_entry(tables, entry);
        id
    }

    fn write_waiting_item(&self, tables: &mut Tables, row: module::WaitingItem) {
        tables.waiting_items.insert(row.id, row.clone());
        self.rows.upsert_waiting_item(&row.into());
    }

    fn write_epic_note_row(&self, tables: &mut Tables, row: module::EpicNote) {
        tables.epic_notes.insert(row.id, row.clone());
        self.rows.upsert_epic_note(&row.into());
    }

    fn resolve_all(&self, tables: &mut Tables, items: Vec<module::WaitingItem>) {
        let stamp = self.now();
        for item in items {
            self.write_waiting_item(
                tables,
                module::WaitingItem {
                    resolved_at: stamp.clone(),
                    ..item
                },
            );
        }
    }

    /// Mirrors the module's `resolve_items_on_finish`.
    pub(super) fn resolve_items_on_finish(
        &self,
        tables: &mut Tables,
        prior: &module::Task,
        next: &module::Task,
    ) {
        if module::finishes(&prior.status, &next.status) {
            let open = module::open_items_of_task(&*tables, next.id);
            self.resolve_all(tables, open);
        }
    }

    /// Mirrors the module's `resolve_items_of_deleted_task`.
    pub(super) fn resolve_items_of_deleted_task(&self, tables: &mut Tables, task_id: i64) {
        let open = module::open_items_of_task(&*tables, task_id);
        self.resolve_all(tables, open);
    }

    /// Mirrors the module's `resolve_items_of_epic`.
    pub(super) fn resolve_items_of_epic(&self, tables: &mut Tables, epic_id: i64) {
        let open = module::open_items_of_epic(&*tables, epic_id);
        self.resolve_all(tables, open);
    }

    /// Mirrors the module's `delete_orchestrator_memory`.
    pub(super) fn delete_orchestrator_memory(&self, tables: &mut Tables, epic_id: i64) {
        let log: Vec<i64> = tables
            .orchestrator_log
            .values()
            .filter(|e| e.epic_id == epic_id)
            .map(|e| e.id)
            .collect();
        for id in log {
            tables.orchestrator_log.remove(&id);
            self.rows.remove_orchestrator_log_entry(id);
        }
        let notes: Vec<i64> = tables
            .epic_notes
            .values()
            .filter(|n| n.epic_id == epic_id)
            .map(|n| n.id)
            .collect();
        for id in notes {
            tables.epic_notes.remove(&id);
            self.rows.remove_epic_note(id);
        }
        let items: Vec<i64> = tables
            .waiting_items
            .values()
            .filter(|i| i.epic_id == epic_id)
            .map(|i| i.id)
            .collect();
        for id in items {
            tables.waiting_items.remove(&id);
            self.rows.remove_waiting_item(id);
        }
    }
}
