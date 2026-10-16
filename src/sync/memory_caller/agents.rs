//! Agents (agents A1): the in-memory twin of `spacetime/module/src/agent.rs`.
//!
//! Every agent rule is the module's own generic function, run here over
//! [`MemAgentStore`]; what is reimplemented is only the row storage. Each
//! write is pushed into `SharedRows`, as a real agent row would arrive over
//! the subscription (`sync.allium: SubscribeToAgents`). The ledger is not
//! pushed: no board subscribes to it.

use dispatch_spacetime_module as module;

use crate::models::AgentId;

use super::{MemoryReducerCaller, Tables};

/// [`module::AgentStore`] over the twin's own rows.
pub(super) struct MemAgentStore<'a> {
    pub(super) caller: &'a MemoryReducerCaller,
    pub(super) tables: &'a mut Tables,
}

impl module::AgentStore for MemAgentStore<'_> {
    fn agents_of_task(&self, task_id: i64) -> Vec<module::Agent> {
        self.tables
            .agents
            .values()
            .filter(|a| a.task_id == task_id)
            .cloned()
            .collect()
    }
    /// The ledger, and the agent rows' names as the module's backstop does.
    fn name_is_handed_out(&self, name: &str) -> bool {
        self.tables.handed_out_agent_names.contains_key(name)
            || self.tables.agents.values().any(|a| a.name == name)
    }
    fn insert_agent(&mut self, row: module::Agent) {
        let id = MemoryReducerCaller::assign_id(0, &mut self.tables.next_agent_id);
        let row = module::Agent { id, ..row };
        self.tables.agents.insert(id, row.clone());
        self.caller.rows.upsert_agent(&row.into());
    }
    fn update_agent(&mut self, row: module::Agent) {
        self.tables.agents.insert(row.id, row.clone());
        self.caller.rows.upsert_agent(&row.into());
    }
    fn delete_agent(&mut self, id: i64) {
        if self.tables.agents.remove(&id).is_some() {
            self.caller.rows.remove_agent(AgentId(id));
        }
    }
    fn insert_handed_out_name(&mut self, row: module::HandedOutAgentName) {
        self.tables
            .handed_out_agent_names
            .insert(row.name.clone(), row);
    }
    fn now(&self) -> String {
        self.caller.now()
    }
    fn name_seed(&self) -> u64 {
        self.caller.clock.now().timestamp_micros() as u64
    }
}

impl MemoryReducerCaller {
    pub(super) fn agent_store<'a>(&'a self, tables: &'a mut Tables) -> MemAgentStore<'a> {
        MemAgentStore {
            caller: self,
            tables,
        }
    }

    /// Mirrors the module's `backfill_agents` reducer: an inherent method,
    /// because no board calls it (only `store import` does, over the CLI).
    pub fn backfill_agents(&self) -> anyhow::Result<super::ReducerOutcome> {
        let mut tables = self.lock();
        let tasks: Vec<module::Task> = tables
            .tasks
            .values()
            .filter(|t| !t.worktree.is_empty())
            .cloned()
            .collect();
        module::backfill_agents_into(&mut self.agent_store(&mut tables), tasks);
        Ok(super::ReducerOutcome::Applied(vec![]))
    }

    /// Every name the ledger holds, sorted: the conformance suite's view of a
    /// table no board subscribes to.
    pub fn handed_out_agent_names(&self) -> Vec<String> {
        self.lock().handed_out_agent_names.keys().cloned().collect()
    }
}
