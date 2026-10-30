//! The per-caller views, served from this store's rows. Mirrors the module's
//! `views.rs` (`workspaces.allium`: "THE PER-CALLER VIEWS"): the module's own
//! view bodies run here over [`Tables`], so the double and the module cannot
//! disagree about who sees what. `tests/memory_caller_conformance.rs` compares
//! the two against a live store.

use dispatch_spacetime_module as module;
use dispatch_spacetime_module::StoreReads;

use super::{MemoryReducerCaller, Tables};

fn keep<T: Clone>(rows: impl Iterator<Item = T>, f: impl Fn(&T) -> bool) -> Vec<T> {
    rows.filter(|r| f(r)).collect()
}

impl StoreReads for Tables {
    fn epic(&self, id: i64) -> Option<module::Epic> {
        self.epics.get(&id).cloned()
    }
    fn task(&self, id: i64) -> Option<module::Task> {
        self.tasks.get(&id).cloned()
    }
    fn package(&self, id: i64) -> Option<module::WorkPackage> {
        self.work_packages.get(&id).cloned()
    }
    fn workspace(&self, id: i64) -> Option<module::Workspace> {
        self.workspaces.get(&id).cloned()
    }
    fn profile(&self, who: &str) -> Option<module::UserProfile> {
        self.user_profiles.get(who).cloned()
    }
    fn members_of(&self, workspace_id: i64) -> Vec<module::Membership> {
        keep(self.memberships.values().cloned(), |m| {
            m.workspace_id == workspace_id
        })
    }
    fn memberships_of(&self, user: &str) -> Vec<module::Membership> {
        keep(self.memberships.values().cloned(), |m| m.user == user)
    }
    fn subscriptions_of(&self, subscriber: &str) -> Vec<module::Subscription> {
        keep(self.subscriptions.values().cloned(), |s| {
            s.subscriber == subscriber
        })
    }
    fn epics_under(&self, parent: i64) -> Vec<module::Epic> {
        keep(self.epics.values().cloned(), |e| e.parent_epic_id == parent)
    }
    fn epics_in_workspace(&self, workspace_id: i64) -> Vec<module::Epic> {
        keep(self.epics.values().cloned(), |e| {
            e.workspace_id == workspace_id
        })
    }
    fn epics_created_by(&self, who: &str) -> Vec<module::Epic> {
        keep(self.epics.values().cloned(), |e| e.created_by == who)
    }
    fn tasks_in_epic(&self, epic_id: i64) -> Vec<module::Task> {
        keep(self.tasks.values().cloned(), |t| t.epic_id == epic_id)
    }
    fn tasks_owned_by(&self, who: &str) -> Vec<module::Task> {
        keep(self.tasks.values().cloned(), |t| t.owner == who)
    }
    fn tasks_created_by(&self, who: &str) -> Vec<module::Task> {
        keep(self.tasks.values().cloned(), |t| t.created_by == who)
    }
    fn packages_in_workspace(&self, workspace_id: i64) -> Vec<module::WorkPackage> {
        keep(self.work_packages.values().cloned(), |p| {
            p.workspace_id == workspace_id
        })
    }
    fn packages_created_by(&self, who: &str) -> Vec<module::WorkPackage> {
        keep(self.work_packages.values().cloned(), |p| {
            p.created_by == who
        })
    }
    fn links_of_package(&self, package_id: i64) -> Vec<module::WorkPackageLink> {
        keep(self.work_package_links.values().cloned(), |l| {
            l.work_package_id == package_id
        })
    }
    fn agents_of(&self, task_id: i64) -> Vec<module::Agent> {
        keep(self.agents.values().cloned(), |a| a.task_id == task_id)
    }
    fn dispatch_requests_of(&self, task_id: i64) -> Vec<module::DispatchRequest> {
        keep(self.dispatch_requests.values().cloned(), |d| {
            d.task_id == task_id
        })
    }
    fn session_requests_of(&self, task_id: i64) -> Vec<module::SessionRequest> {
        keep(self.session_requests.values().cloned(), |s| {
            s.task_id == task_id
        })
    }
    fn unseen_of(&self, subscriber: &str) -> Vec<module::UnseenTask> {
        keep(self.unseen_tasks.values().cloned(), |u| {
            u.subscriber == subscriber
        })
    }
    fn subagents_of(&self, task_id: i64) -> Vec<module::TaskSubagent> {
        keep(self.task_subagents.iter().cloned(), |s| {
            s.task_id == task_id
        })
    }
    /// `task_watchers` is dead upstream and this store keeps no rows for it.
    fn watchers_of(&self, _task_id: i64) -> Vec<module::TaskWatcher> {
        Vec::new()
    }
    fn retrievals_of(&self, task_id: i64) -> Vec<module::LearningRetrieval> {
        keep(self.learning_retrievals.values().cloned(), |l| {
            l.task_id == task_id
        })
    }
    fn verdicts_of(&self, task_id: i64) -> Vec<module::LearningVerdict> {
        keep(self.learning_verdicts.values().cloned(), |l| {
            l.task_id == task_id
        })
    }
    fn poll_owners_of(&self, scope_id: i64) -> Vec<module::PollOwner> {
        keep(self.poll_owners.values().cloned(), |p| {
            p.scope_id == scope_id
        })
    }
    fn learnings_in_workspace(&self, workspace_id: i64) -> Vec<module::Learning> {
        keep(self.learnings.values().cloned(), |l| {
            l.workspace_id == workspace_id
        })
    }
    fn learnings_by_author(&self, author: &str) -> Vec<module::Learning> {
        keep(self.learnings.values().cloned(), |l| l.author == author)
    }
    fn hosts_owned_by(&self, owner: &str) -> Vec<module::Host> {
        keep(self.hosts.values().cloned(), |h| h.owner == owner)
    }
    fn settings_of(&self, host: &str) -> Vec<module::Setting> {
        keep(self.settings.values().cloned(), |s| s.host == host)
    }
}

impl MemoryReducerCaller {
    /// The row keys the named view would hold for this store's sender, sorted,
    /// for the conformance suite to compare with the live view
    /// (`SELECT * FROM <view>`). A row's key is its id, or its derived id for a
    /// table keyed by a string. `None` for a name that is not a view.
    pub fn view_keys(&self, view: &str) -> Option<Vec<String>> {
        let who = self.sender.as_str();
        let t = &*self.lock();
        let ids = |v: Vec<i64>| v.into_iter().map(|i| i.to_string()).collect::<Vec<_>>();
        let mut keys = match view {
            "own_epics" => ids(module::own_epics_of(t, who).iter().map(|e| e.id).collect()),
            "own_tasks" => ids(module::own_tasks_of(t, who).iter().map(|e| e.id).collect()),
            "own_packages" => ids(module::own_packages_of(t, who)
                .iter()
                .map(|e| e.id)
                .collect()),
            "covered_epics" => ids(module::covered_epics_of(t, who)
                .iter()
                .map(|e| e.id)
                .collect()),
            "covered_tasks" => ids(module::covered_tasks_of(t, who)
                .iter()
                .map(|e| e.id)
                .collect()),
            "visible_epics" => ids(module::visible_epics_of(t, who)
                .iter()
                .map(|e| e.id)
                .collect()),
            "visible_tasks" => ids(module::visible_tasks_of(t, who)
                .iter()
                .map(|e| e.id)
                .collect()),
            "visible_packages" => ids(module::visible_packages_of(t, who)
                .iter()
                .map(|e| e.id)
                .collect()),
            "visible_agents" => ids(module::agents_of_held(t, who)
                .iter()
                .map(|e| e.id)
                .collect()),
            "visible_dispatch_requests" => ids(module::dispatch_requests_of_held(t, who)
                .iter()
                .map(|e| e.id)
                .collect()),
            "visible_session_requests" => ids(module::session_requests_of_held(t, who)
                .iter()
                .map(|e| e.id)
                .collect()),
            "own_unseen_tasks" => module::own_unseen_of(t, who)
                .into_iter()
                .map(|u| u.id)
                .collect(),
            "visible_task_subagents" => module::subagents_of_held(t, who)
                .into_iter()
                .map(|s| format!("{}/{}/{}", s.task_id, s.agent_id, s.session_id))
                .collect(),
            "visible_task_watchers" => ids(module::watchers_of_held(t, who)
                .iter()
                .map(|e| e.id)
                .collect()),
            "visible_learning_retrievals" => ids(module::retrievals_of_held(t, who)
                .iter()
                .map(|e| e.id)
                .collect()),
            "visible_learning_verdicts" => ids(module::verdicts_of_held(t, who)
                .iter()
                .map(|e| e.id)
                .collect()),
            "visible_poll_owners" => ids(module::poll_owners_visible_to(t, who)
                .iter()
                .map(|e| e.id)
                .collect()),
            "visible_work_package_links" => module::links_visible_to(t, who)
                .into_iter()
                .map(|l| l.id)
                .collect(),
            "visible_learnings" => ids(module::learnings_visible_to(t, who)
                .iter()
                .map(|e| e.id)
                .collect()),
            "own_settings" => module::settings_visible_to(t, who)
                .into_iter()
                .map(|s| s.id)
                .collect(),
            "visible_workspaces" => ids(module::workspaces_visible_to(t, who)
                .iter()
                .map(|e| e.id)
                .collect()),
            "visible_memberships" => ids(module::memberships_visible_to(t, who)
                .iter()
                .map(|e| e.id)
                .collect()),
            _ => return None,
        };
        keys.sort();
        Some(keys)
    }
}
