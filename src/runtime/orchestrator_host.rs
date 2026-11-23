//! The board's orchestrator runner (`orchestrators.allium:
//! BoardRunsItsOrchestrators`, `OrchestratorWindowGone`,
//! `StartOrchestratorSession`, `OrchestratorThatKeepsDyingWaitsForTheOperator`,
//! `EndOrchestratorSession`).
//!
//! On every board tick it reads the switches, the tasks and the orchestrator
//! rows, asks tmux which windows exist, and decides what this machine's
//! orchestrators need ([`plan_orchestrators`], pure). The store decides each
//! change to a row; this runner only opens and closes windows around it.

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use super::*;
use crate::models::{Epic, EpicId, EpicOrchestration, Orchestrator, Task, TaskStatus, TmuxWindow};

/// What one tick does for this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum OrchestratorAction {
    /// Create the row of an orchestrated epic with open work.
    Ensure(EpicId),
    /// The recorded window is gone: the session died.
    WindowLost(Orchestrator),
    /// Start a session for a wanted orchestrator.
    Start(Orchestrator),
    /// End the session of an orchestrator whose epic has no open work.
    End(Orchestrator),
    /// A wanted orchestrator died twice in a row and waits for the operator.
    GiveUp(Orchestrator),
    /// An orchestrator window no live row of this machine holds (its switch
    /// went off).
    CloseOrphan(TmuxWindow),
}

/// The ids of `root` and every epic below it.
fn subtree(epics: &[Epic], root: EpicId) -> BTreeSet<EpicId> {
    let mut seen = BTreeSet::from([root]);
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        for e in epics.iter().filter(|e| e.parent_epic_id == Some(id)) {
            if seen.insert(e.id) {
                pending.push(e.id);
            }
        }
    }
    seen
}

/// `has_open_work(epic)`: some task in the epic's subtree is not done.
fn has_open_work(epics: &[Epic], tasks: &[Task], epic: EpicId) -> bool {
    let ids = subtree(epics, epic);
    tasks
        .iter()
        .any(|t| t.status != TaskStatus::Done && t.epic_id.is_some_and(|e| ids.contains(&e)))
}

/// Decide this tick's actions for `host`. `windows` is every tmux window
/// name on this machine, or `None` when tmux could not be asked: then no
/// window counts as lost and none as an orphan.
pub(super) fn plan_orchestrators(
    host: &str,
    epics: &[Epic],
    orchestrations: &[EpicOrchestration],
    tasks: &[Task],
    orchestrators: &[Orchestrator],
    windows: Option<&HashSet<String>>,
) -> Vec<OrchestratorAction> {
    let mut actions = Vec::new();
    for on in orchestrations {
        let has_row = orchestrators
            .iter()
            .any(|o| o.epic_id == on.epic_id && o.host == host);
        if !has_row && has_open_work(epics, tasks, on.epic_id) {
            actions.push(OrchestratorAction::Ensure(on.epic_id));
        }
    }
    let mut live_windows = HashSet::new();
    for o in orchestrators.iter().filter(|o| o.host == host) {
        let lost = match (&o.tmux_window, windows) {
            (Some(w), Some(names)) => !names.contains(w.as_str()),
            _ => false,
        };
        let live = o.is_live() && !lost;
        let died = o.session_died || lost;
        if lost {
            actions.push(OrchestratorAction::WindowLost(o.clone()));
        }
        let wanted = has_open_work(epics, tasks, o.epic_id);
        if wanted && !live {
            if o.restarted_once && died {
                actions.push(OrchestratorAction::GiveUp(o.clone()));
            } else {
                actions.push(OrchestratorAction::Start(o.clone()));
            }
        } else if !wanted && live {
            actions.push(OrchestratorAction::End(o.clone()));
        }
        // A row's own window is never an orphan: a live one is kept or ended
        // by the row, and a start opens it.
        live_windows.insert(TmuxWindow::for_orchestrator(o.epic_id).as_str().to_string());
    }
    if let Some(names) = windows {
        let mut orphans: Vec<&String> = names
            .iter()
            .filter(|n| TmuxWindow::is_orchestrator_window(n) && !live_windows.contains(n.as_str()))
            .collect();
        orphans.sort();
        for name in orphans {
            if let Some(w) = TmuxWindow::parse(name) {
                actions.push(OrchestratorAction::CloseOrphan(w));
            }
        }
    }
    actions
}

/// The runner, cloned into its own background task.
#[derive(Clone)]
pub(super) struct OrchestratorHost {
    database: Arc<dyn store::TaskReadStore>,
    runner: Arc<dyn ProcessRunner>,
    data_dir: PathBuf,
    claude_json: PathBuf,
    /// The rows already logged as waiting for the operator, so the log says
    /// it once per bout.
    given_up: Arc<std::sync::Mutex<HashSet<i64>>>,
}

impl OrchestratorHost {
    pub(super) fn new(
        database: Arc<dyn store::TaskReadStore>,
        runner: Arc<dyn ProcessRunner>,
        data_dir: PathBuf,
        claude_json: PathBuf,
    ) -> Self {
        Self {
            database,
            runner,
            data_dir,
            claude_json,
            given_up: Arc::new(std::sync::Mutex::new(HashSet::new())),
        }
    }

    /// Run [`Self::tick`] every `every`, one tick at a time, for as long as
    /// the board runs.
    pub(super) fn start(self, every: Duration) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut clock = tokio::time::interval(every);
            loop {
                clock.tick().await;
                self.tick().await;
            }
        })
    }

    /// One `BoardRunsItsOrchestrators` check.
    pub(super) async fn tick(&self) {
        let db = &self.database;
        let host = db.host_id().to_string();
        let epics = db.list_epics().await;
        let mut orchestrations = Vec::new();
        for e in &epics {
            if let Some(on) = db.epic_orchestration(e.id).await {
                orchestrations.push(on);
            }
        }
        let orchestrators = db.list_orchestrators().await;
        let tasks = db.list_all_tasks_with_epic_id().await;
        let runner = self.runner.clone();
        let windows = tokio::task::spawn_blocking(move || tmux::list_all_window_names(&*runner))
            .await
            .ok()
            .and_then(Result::ok)
            .map(|names| names.into_iter().collect::<HashSet<_>>());
        let actions = plan_orchestrators(
            &host,
            &epics,
            &orchestrations,
            &tasks,
            &orchestrators,
            windows.as_ref(),
        );
        for action in actions {
            self.apply(&host, action, &epics, &orchestrations, &tasks)
                .await;
        }
    }

    async fn apply(
        &self,
        host: &str,
        action: OrchestratorAction,
        epics: &[Epic],
        orchestrations: &[EpicOrchestration],
        tasks: &[Task],
    ) {
        let db = &self.database;
        match action {
            OrchestratorAction::Ensure(epic) => {
                if let Err(e) = db.ensure_orchestrator(epic, host).await {
                    tracing::warn!(
                        epic = epic.0,
                        "could not create the orchestrator row: {e:#}"
                    );
                }
            }
            OrchestratorAction::WindowLost(o) => {
                tracing::info!(orchestrator = %o.name, epic = o.epic_id.0, "orchestrator window gone");
                if let Err(e) = db.end_orchestrator_session(o.id, host, true).await {
                    tracing::warn!(orchestrator = %o.name, "could not record the lost window: {e:#}");
                }
            }
            OrchestratorAction::Start(o) => {
                self.given_up().remove(&o.id);
                self.start_session(host, &o, epics, orchestrations, tasks)
                    .await;
            }
            OrchestratorAction::End(o) => {
                if let Some(window) = o.tmux_window.clone() {
                    let runner = self.runner.clone();
                    let closed = tokio::task::spawn_blocking(move || {
                        crate::dispatch::end_orchestrator(&window, &*runner)
                    })
                    .await;
                    if !matches!(closed, Ok(Ok(()))) {
                        tracing::warn!(orchestrator = %o.name, "could not close the orchestrator's window");
                    }
                }
                if let Err(e) = db.end_orchestrator_session(o.id, host, false).await {
                    tracing::warn!(orchestrator = %o.name, "could not end the orchestrator's session: {e:#}");
                }
            }
            OrchestratorAction::GiveUp(o) => {
                // Until the items that wait for the operator exist (step 5),
                // the board's own log is where this is said.
                if self.given_up().insert(o.id) {
                    tracing::warn!(
                        orchestrator = %o.name,
                        epic = o.epic_id.0,
                        "the orchestrator's session died twice in a row; it is not started again \
                         until its epic's orchestrator is switched off and on"
                    );
                }
            }
            OrchestratorAction::CloseOrphan(window) => {
                let runner = self.runner.clone();
                let name = window.as_str().to_string();
                let closed = tokio::task::spawn_blocking(move || {
                    crate::dispatch::end_orchestrator(&window, &*runner)
                })
                .await;
                if matches!(closed, Ok(Ok(()))) {
                    tracing::info!(window = %name, "closed an orchestrator window with no orchestrator");
                }
            }
        }
    }

    fn given_up(&self) -> std::sync::MutexGuard<'_, HashSet<i64>> {
        self.given_up.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn start_session(
        &self,
        host: &str,
        o: &Orchestrator,
        epics: &[Epic],
        orchestrations: &[EpicOrchestration],
        tasks: &[Task],
    ) {
        let db = &self.database;
        let (Some(epic), Some(on)) = (
            epics.iter().find(|e| e.id == o.epic_id),
            orchestrations.iter().find(|on| on.epic_id == o.epic_id),
        ) else {
            return;
        };
        if let Err(e) = db.start_orchestrator_session(o.id, host).await {
            tracing::warn!(orchestrator = %o.name, "the store refused the orchestrator's start: {e:#}");
            return;
        }
        let prompt = self.prompt(o, epic, on, epics, tasks).await;
        let window = TmuxWindow::for_orchestrator(o.epic_id);
        let runner = self.runner.clone();
        let dir = crate::dispatch::orchestrator_dir(&self.data_dir, o.epic_id);
        let claude_json = self.claude_json.clone();
        let name = o.name.clone();
        let launched = tokio::task::spawn_blocking(move || {
            crate::dispatch::launch_orchestrator(
                &crate::dispatch::OrchestratorLaunch {
                    name: &name,
                    window: &window,
                    dir: &dir,
                    prompt: &prompt,
                    claude_json: &claude_json,
                },
                &*runner,
            )
        })
        .await;
        let failure = match launched {
            Ok(Ok(())) => return,
            Ok(Err(e)) => format!("{e:#}"),
            Err(e) => e.to_string(),
        };
        tracing::warn!(orchestrator = %o.name, "the orchestrator's session did not start: {failure}");
        if let Err(e) = db.end_orchestrator_session(o.id, host, true).await {
            tracing::warn!(orchestrator = %o.name, "could not record the failed start: {e:#}");
        }
    }

    async fn prompt(
        &self,
        o: &Orchestrator,
        epic: &Epic,
        on: &EpicOrchestration,
        epics: &[Epic],
        tasks: &[Task],
    ) -> String {
        let ids = subtree(epics, o.epic_id);
        let mut mine: Vec<&Task> = tasks
            .iter()
            .filter(|t| t.epic_id.is_some_and(|e| ids.contains(&e)))
            .collect();
        mine.sort_by_key(|t| (t.sort_order.unwrap_or(i64::MAX), t.id.0));
        let mut listed = Vec::with_capacity(mine.len());
        for t in mine {
            let ready = self.database.ready_mark(t.id).await.map(|m| m.marked_by);
            listed.push(crate::dispatch::OrchestratorPromptTask {
                id: t.id,
                title: t.title.clone(),
                status: t.status,
                sub_status: t.sub_status,
                ready,
                agent: t.agent_name.clone(),
            });
        }
        crate::dispatch::build_orchestrator_prompt(&o.name, epic, on, &listed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EpicBuilder, StartRule, TaskBuilder};
    use OrchestratorAction::*;

    const HOST: &str = "host-a";

    fn switch(epic: i64) -> EpicOrchestration {
        EpicOrchestration {
            epic_id: EpicId(epic),
            start_rule: StartRule::MarkedOnly,
            max_parallel: None,
            switched_on_at: chrono::Utc::now(),
        }
    }

    fn task(id: i64, epic: i64, status: TaskStatus) -> Task {
        TaskBuilder::new(id).epic(Some(epic)).status(status).build()
    }

    fn row(id: i64, epic: i64, host: &str) -> Orchestrator {
        Orchestrator {
            id,
            epic_id: EpicId(epic),
            host: host.into(),
            name: format!("o{id}"),
            tmux_window: None,
            started_at: None,
            restarted_once: false,
            session_died: false,
            last_turn_at: None,
        }
    }

    fn live(mut o: Orchestrator) -> Orchestrator {
        o.tmux_window = Some(TmuxWindow::for_orchestrator(o.epic_id));
        o
    }

    fn windows(names: &[&str]) -> HashSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    fn plan(
        epics: &[Epic],
        switches: &[EpicOrchestration],
        tasks: &[Task],
        rows: &[Orchestrator],
        names: Option<&HashSet<String>>,
    ) -> Vec<OrchestratorAction> {
        plan_orchestrators(HOST, epics, switches, tasks, rows, names)
    }

    #[test]
    fn an_orchestrated_epic_with_open_work_gets_a_row_here() {
        let epics = [EpicBuilder::new(1).build()];
        let tasks = [task(10, 1, TaskStatus::Backlog)];
        let other_host = [row(5, 1, "host-b")];

        let actions = plan(
            &epics,
            &[switch(1)],
            &tasks,
            &other_host,
            Some(&windows(&[])),
        );

        assert_eq!(actions, vec![Ensure(EpicId(1))]);
    }

    #[test]
    fn no_open_work_wants_no_row_and_no_session() {
        let epics = [EpicBuilder::new(1).build()];
        let done = [task(10, 1, TaskStatus::Done)];
        assert!(plan(&epics, &[switch(1)], &done, &[], Some(&windows(&[]))).is_empty());
        assert!(plan(&epics, &[switch(1)], &[], &[], Some(&windows(&[]))).is_empty());
    }

    #[test]
    fn open_work_in_a_sub_epic_counts() {
        let epics = [
            EpicBuilder::new(1).build(),
            EpicBuilder::new(2).parent(Some(1)).build(),
        ];
        let tasks = [task(10, 2, TaskStatus::Review)];
        let rows = [row(5, 1, HOST)];

        let actions = plan(&epics, &[switch(1)], &tasks, &rows, Some(&windows(&[])));

        assert_eq!(actions, vec![Start(rows[0].clone())]);
    }

    #[test]
    fn a_live_wanted_session_is_left_alone() {
        let epics = [EpicBuilder::new(1).build()];
        let tasks = [task(10, 1, TaskStatus::Running)];
        let rows = [live(row(5, 1, HOST))];

        let actions = plan(
            &epics,
            &[switch(1)],
            &tasks,
            &rows,
            Some(&windows(&["orch-1", "TUI"])),
        );

        assert!(actions.is_empty(), "{actions:?}");
    }

    #[test]
    fn a_live_session_with_nothing_open_is_ended() {
        let epics = [EpicBuilder::new(1).build()];
        let tasks = [task(10, 1, TaskStatus::Done)];
        let rows = [live(row(5, 1, HOST))];

        let actions = plan(
            &epics,
            &[switch(1)],
            &tasks,
            &rows,
            Some(&windows(&["orch-1"])),
        );

        assert_eq!(actions, vec![End(rows[0].clone())]);
    }

    #[test]
    fn a_lost_window_is_recorded_and_the_session_restarted() {
        let epics = [EpicBuilder::new(1).build()];
        let tasks = [task(10, 1, TaskStatus::Backlog)];
        let rows = [live(row(5, 1, HOST))];

        let actions = plan(&epics, &[switch(1)], &tasks, &rows, Some(&windows(&[])));

        assert_eq!(
            actions,
            vec![WindowLost(rows[0].clone()), Start(rows[0].clone())]
        );
    }

    #[test]
    fn a_second_death_in_a_row_waits_for_the_operator() {
        let epics = [EpicBuilder::new(1).build()];
        let tasks = [task(10, 1, TaskStatus::Backlog)];
        let mut restarted = live(row(5, 1, HOST));
        restarted.restarted_once = true;

        let actions = plan(
            &epics,
            &[switch(1)],
            &tasks,
            &[restarted.clone()],
            Some(&windows(&[])),
        );
        assert_eq!(
            actions,
            vec![WindowLost(restarted.clone()), GiveUp(restarted)]
        );

        let mut down = row(5, 1, HOST);
        down.restarted_once = true;
        down.session_died = true;
        let actions = plan(
            &epics,
            &[switch(1)],
            &tasks,
            &[down.clone()],
            Some(&windows(&[])),
        );
        assert_eq!(actions, vec![GiveUp(down)]);
    }

    #[test]
    fn an_unreadable_tmux_loses_no_window_and_closes_none() {
        let epics = [EpicBuilder::new(1).build()];
        let tasks = [task(10, 1, TaskStatus::Backlog)];
        let rows = [live(row(5, 1, HOST))];

        assert!(plan(&epics, &[switch(1)], &tasks, &rows, None).is_empty());
    }

    #[test]
    fn another_hosts_rows_are_not_this_boards() {
        let epics = [EpicBuilder::new(1).build()];
        let tasks = [task(10, 1, TaskStatus::Backlog)];
        let rows = [row(5, 1, HOST), live(row(6, 1, "host-b"))];

        let actions = plan(&epics, &[switch(1)], &tasks, &rows, Some(&windows(&[])));

        assert_eq!(actions, vec![Start(rows[0].clone())]);
    }

    #[test]
    fn a_window_with_no_live_row_here_is_closed() {
        let names = windows(&["orch-7", "orch-1", "task-3", "orchard", "TUI"]);
        let epics = [EpicBuilder::new(1).build()];
        let tasks = [task(10, 1, TaskStatus::Running)];
        let rows = [live(row(5, 1, HOST))];

        let actions = plan(&epics, &[switch(1)], &tasks, &rows, Some(&names));

        assert_eq!(
            actions,
            vec![CloseOrphan(TmuxWindow::for_orchestrator(EpicId(7)))]
        );
    }

    #[test]
    fn the_window_name_is_the_one_the_store_records() {
        assert_eq!(
            TmuxWindow::for_orchestrator(EpicId(42)).as_str(),
            dispatch_spacetime_module::orchestrator_window_name(42)
        );
    }

    // -- One tick after another, over the in-memory store --------------------

    use crate::process::MockProcessRunner;
    use crate::store::{EpicCrud, OrchestratorStore, TaskCrud};

    fn listing(names: &str) -> anyhow::Result<std::process::Output> {
        MockProcessRunner::ok_with_stdout(names.as_bytes())
    }

    #[tokio::test]
    async fn ticks_create_the_row_start_the_session_and_end_it_when_the_work_is_done() {
        let db = crate::runtime::tests::test_db().await;
        let epic = db.create_epic("E", "", None).await.unwrap();
        db.switch_orchestrator_on(epic.id, None, None)
            .await
            .unwrap();
        let task = db
            .create_task(crate::models::CreateTaskRequest {
                epic_id: Some(epic.id),
                ..crate::models::CreateTaskRequest::fixture("t", "/repo")
            })
            .await
            .unwrap();
        let window = TmuxWindow::for_orchestrator(epic.id);
        let shown = format!("{}\n", window.as_str());
        let runner = Arc::new(
            MockProcessRunner::new(vec![
                // Tick 1: the plan's listing; the row is created.
                listing(""),
                // Tick 2: the plan's listing, the launch's two window checks,
                // and the new window.
                listing(""),
                listing(""),
                listing(""),
                MockProcessRunner::ok(),
                // Tick 3: the plan's listing, the end's window check, the kill.
                listing(&shown),
                listing(&shown),
                MockProcessRunner::ok(),
            ])
            .with_windows(&[window.as_str()]),
        );
        let tmp = tempfile::tempdir().unwrap();
        let claude_json = tmp.path().join(".claude.json");
        let host = OrchestratorHost::new(
            db.clone(),
            runner.clone(),
            tmp.path().join("data"),
            claude_json,
        );

        host.tick().await;
        let rows = db.list_orchestrators().await;
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].is_live());

        host.tick().await;
        let row = db.list_orchestrators().await.remove(0);
        assert_eq!(row.tmux_window.as_ref(), Some(&window));
        assert!(row.started_at.is_some());
        let opened = runner.flattened_calls();
        assert!(
            opened
                .iter()
                .any(|c| c.contains("new-window") && c.contains(window.as_str())),
            "{opened:?}"
        );

        db.patch_task(
            task,
            &crate::models::TaskPatch::new().status(TaskStatus::Done),
        )
        .await
        .unwrap();
        host.tick().await;
        let row = db.list_orchestrators().await.remove(0);
        assert!(!row.is_live());
        assert!(!row.session_died, "the board ended it");
        assert!(runner
            .flattened_calls()
            .iter()
            .any(|c| c.contains("kill-window")));
    }
}
