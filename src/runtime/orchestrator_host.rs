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

use super::orchestrator_wake::{self as wake, SummaryAsk, SummaryStep};
use super::*;
use crate::models::{Epic, EpicId, EpicOrchestration, Orchestrator, Task, TaskStatus, TmuxWindow};

/// What this board remembers of one live orchestrator between ticks
/// (`agent-health.allium: WakeTheOrchestrator`).
#[derive(Default)]
struct WakeState {
    /// The subtree at the last tick; `None` until the first one.
    seen: Option<wake::Subtree>,
    woken: wake::Woken,
    /// This board started the session, so its prompt shows the board.
    started_here: bool,
    /// This board's ask for the closing summary.
    summary_ask: Option<SummaryAsk>,
}

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
    /// The window of a row this board saw on this machine that has gone (its
    /// switch went off).
    CloseGone(TmuxWindow),
}

/// The epics with open work: every epic that holds, in its subtree, a task
/// that is not done (`has_open_work`).
fn epics_with_open_work(epics: &[Epic], tasks: &[Task]) -> HashSet<EpicId> {
    let parent: std::collections::HashMap<EpicId, Option<EpicId>> =
        epics.iter().map(|e| (e.id, e.parent_epic_id)).collect();
    let mut open = HashSet::new();
    for task in tasks.iter().filter(|t| t.status != TaskStatus::Done) {
        let mut next = task.epic_id;
        while let Some(id) = next {
            if !open.insert(id) {
                break;
            }
            next = parent.get(&id).copied().flatten();
        }
    }
    open
}

/// Decide this tick's actions for `host`. `windows` is every tmux window
/// name on this machine, or `None` when tmux could not be asked: then no
/// window counts as lost. `seen_here` is the epics of the rows this board
/// saw on this host at its last tick, so the window of one that has gone is
/// closed (`SwitchOrchestratorOff`); a window this board never saw a row for
/// is not its to close.
pub(super) fn plan_orchestrators(
    host: &str,
    epics: &[Epic],
    orchestrations: &[EpicOrchestration],
    tasks: &[Task],
    orchestrators: &[Orchestrator],
    windows: Option<&HashSet<String>>,
    seen_here: &BTreeSet<EpicId>,
) -> Vec<OrchestratorAction> {
    let open = epics_with_open_work(epics, tasks);
    let mut actions = Vec::new();
    for on in orchestrations {
        let has_row = orchestrators
            .iter()
            .any(|o| o.epic_id == on.epic_id && o.host == host);
        if !has_row && open.contains(&on.epic_id) {
            actions.push(OrchestratorAction::Ensure(on.epic_id));
        }
    }
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
        let wanted = open.contains(&o.epic_id);
        if wanted && !live {
            if o.restarted_once && died {
                actions.push(OrchestratorAction::GiveUp(o.clone()));
            } else {
                // The start sees the death this tick found, so it can start
                // fresh after one.
                actions.push(OrchestratorAction::Start(Orchestrator {
                    session_died: died,
                    ..o.clone()
                }));
            }
        } else if !wanted && live {
            actions.push(OrchestratorAction::End(o.clone()));
        }
    }
    for epic in seen_here {
        let still_here = orchestrators
            .iter()
            .any(|o| o.host == host && o.epic_id == *epic);
        if !still_here {
            actions.push(OrchestratorAction::CloseGone(TmuxWindow::for_orchestrator(
                *epic,
            )));
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
    emb_svc: Arc<crate::embeddings::EmbeddingService>,
    /// The epics of the rows on this host at the last tick
    /// (`plan_orchestrators`' `seen_here`).
    seen_here: Arc<std::sync::Mutex<BTreeSet<EpicId>>>,
    wakes: Arc<std::sync::Mutex<std::collections::HashMap<EpicId, WakeState>>>,
    /// Where the board hears what a tick did (`NotifyEpicFinished`, an
    /// open that found nothing to start). `None` in tests that do not
    /// listen.
    board: Option<tokio::sync::mpsc::UnboundedSender<crate::tui::Message>>,
    opener: OrchestratorOpener,
}

/// The board's handle on the runner: the orchestrator key asks for a
/// session and its window now (`OrchestratorKey`), not at the next check.
#[derive(Clone, Default)]
pub(super) struct OrchestratorOpener {
    now: Arc<tokio::sync::Notify>,
    /// The epics whose window is switched to once their session starts.
    open: Arc<std::sync::Mutex<BTreeSet<EpicId>>>,
}

impl OrchestratorOpener {
    /// Run the next check now, and switch to `epic`'s window if it starts
    /// its session.
    pub(super) fn open_after_start(&self, epic: EpicId) {
        self.pending().insert(epic);
        self.now.notify_one();
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, BTreeSet<EpicId>> {
        self.open.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl OrchestratorHost {
    pub(super) fn new(
        database: Arc<dyn store::TaskReadStore>,
        runner: Arc<dyn ProcessRunner>,
        data_dir: PathBuf,
        claude_json: PathBuf,
        emb_svc: Arc<crate::embeddings::EmbeddingService>,
    ) -> Self {
        Self {
            database,
            runner,
            data_dir,
            claude_json,
            emb_svc,
            seen_here: Arc::new(std::sync::Mutex::new(BTreeSet::new())),
            wakes: Arc::default(),
            board: None,
            opener: OrchestratorOpener::default(),
        }
    }

    /// Tell the board what the ticks do.
    pub(super) fn reporting_to(
        mut self,
        board: tokio::sync::mpsc::UnboundedSender<crate::tui::Message>,
    ) -> Self {
        self.board = Some(board);
        self
    }

    pub(super) fn opener(&self) -> OrchestratorOpener {
        self.opener.clone()
    }

    fn tell_board(&self, message: crate::tui::Message) {
        if let Some(board) = &self.board {
            let _ = board.send(message);
        }
    }

    /// Run [`Self::tick`] every `every`, one tick at a time, for as long as
    /// the board runs.
    pub(super) fn start(self, every: Duration) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut clock = tokio::time::interval(every);
            let now = self.opener.now.clone();
            loop {
                tokio::select! {
                    _ = clock.tick() => {}
                    () = now.notified() => {}
                }
                self.tick().await;
            }
        })
    }

    /// One `BoardRunsItsOrchestrators` check.
    pub(super) async fn tick(&self) {
        let db = &self.database;
        let host = db.host_id().to_string();
        let orchestrations = db.list_epic_orchestrations().await;
        let orchestrators = db.list_orchestrators().await;
        let seen_here = std::mem::replace(
            &mut *self.seen_here.lock().unwrap_or_else(|e| e.into_inner()),
            orchestrators
                .iter()
                .filter(|o| o.host == host)
                .map(|o| o.epic_id)
                .collect(),
        );
        let here = orchestrators.iter().any(|o| o.host == host);
        if orchestrations.is_empty() && !here && seen_here.is_empty() {
            return;
        }
        let epics = db.list_epics().await;
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
            &seen_here,
        );
        let now = chrono::Utc::now();
        let mut acted_on = HashSet::new();
        for action in actions {
            match &action {
                OrchestratorAction::Start(o) => {
                    acted_on.insert(o.epic_id);
                    self.wake_states().insert(
                        o.epic_id,
                        WakeState {
                            started_here: true,
                            ..WakeState::default()
                        },
                    );
                }
                OrchestratorAction::WindowLost(o) | OrchestratorAction::GiveUp(o) => {
                    acted_on.insert(o.epic_id);
                    self.wake_states().remove(&o.epic_id);
                }
                OrchestratorAction::End(o) => {
                    acted_on.insert(o.epic_id);
                    if !self.summary_lets_it_end(o, &epics, now).await {
                        continue;
                    }
                    self.wake_states().remove(&o.epic_id);
                }
                OrchestratorAction::Ensure(_) | OrchestratorAction::CloseGone(_) => {}
            }
            self.apply(&host, action, &epics, &orchestrations, &tasks)
                .await;
        }
        let live: Vec<&Orchestrator> = orchestrators
            .iter()
            .filter(|o| o.host == host && o.is_live() && !acted_on.contains(&o.epic_id))
            .collect();
        if !live.is_empty() {
            self.wake_live(&host, &live, &epics, &tasks, now).await;
        }
        let unopened = std::mem::take(&mut *self.opener.pending());
        for epic in unopened {
            let title = epics
                .iter()
                .find(|e| e.id == epic)
                .map_or_else(|| format!("epic {}", epic.0), |e| e.title.clone());
            self.tell_board(crate::tui::Message::System(
                crate::tui::messages::SystemMessage::StatusInfo(format!(
                    "The orchestrator of {title} did not start: its epic has no open work"
                )),
            ));
        }
    }

    fn wake_states(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::HashMap<EpicId, WakeState>> {
        self.wakes.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `BoardWaitsForTheSummary`: on a finished epic, ask for the closing
    /// summary and keep the window until a turn completes or the wait passes.
    /// `true` when the session may end now.
    async fn summary_lets_it_end(
        &self,
        o: &Orchestrator,
        epics: &[Epic],
        now: chrono::DateTime<chrono::Utc>,
    ) -> bool {
        let done = epics
            .iter()
            .any(|e| e.id == o.epic_id && e.status == TaskStatus::Done);
        let Some(window) = o.tmux_window.clone().filter(|_| done) else {
            return true;
        };
        let asked = self
            .wake_states()
            .get(&o.epic_id)
            .and_then(|s| s.summary_ask);
        let log = self.database.orchestrator_log_of_epic(o.epic_id).await;
        match wake::summary_step(asked, &log, now) {
            SummaryStep::End => true,
            SummaryStep::Wait => false,
            SummaryStep::Ask => {
                let line = wake::wake_line(o.epic_id, &[wake::summary_event(o.epic_id)]);
                if !self.type_into(window, line).await {
                    tracing::warn!(orchestrator = %o.name, "could not ask for the closing summary");
                    return true;
                }
                self.wake_states().entry(o.epic_id).or_default().summary_ask = Some(SummaryAsk {
                    at: now,
                    log_then: wake::newest_entry(&log),
                });
                false
            }
        }
    }

    /// `WakeTheOrchestrator` for this machine's live orchestrators.
    async fn wake_live(
        &self,
        host: &str,
        live: &[&Orchestrator],
        epics: &[Epic],
        tasks: &[Task],
        now: chrono::DateTime<chrono::Utc>,
    ) {
        let db = &self.database;
        let (items, requests, ready_ids) = tokio::join!(
            db.list_waiting_items(),
            db.dispatch_requests(),
            db.ready_task_ids()
        );
        let ready_ids: HashSet<_> = ready_ids.into_iter().collect();
        for o in live {
            let ids = crate::models::epics::descendant_epic_ids(o.epic_id, epics);
            let mine: Vec<&Task> = tasks
                .iter()
                .filter(|t| t.epic_id.is_some_and(|e| ids.contains(&e)))
                .collect();
            let ready = mine
                .iter()
                .map(|t| t.id)
                .filter(|id| ready_ids.contains(id))
                .collect();
            let log = db.orchestrator_log_of_epic(o.epic_id).await;
            let reads = wake::EpicReads {
                tasks: mine,
                ready,
                items: items.iter().filter(|i| i.epic_id == o.epic_id).collect(),
                log: &log,
                requests: &requests,
            };
            let current = wake::subtree(host, &reads);
            let events = {
                let mut states = self.wake_states();
                let state = states.entry(o.epic_id).or_default();
                let mut events = match &state.seen {
                    Some(prev) => wake::changes(host, prev, &current, &reads),
                    None if state.started_here => Vec::new(),
                    None => vec![wake::restart_event()],
                };
                events.extend(wake::due(host, &reads, now, &mut state.woken));
                state.seen = Some(current);
                events
            };
            if events.is_empty() {
                continue;
            }
            let Some(window) = o.tmux_window.clone() else {
                continue;
            };
            if !self
                .type_into(window, wake::wake_line(o.epic_id, &events))
                .await
            {
                tracing::warn!(orchestrator = %o.name, "could not wake the orchestrator");
            }
        }
    }

    /// Type one line into an orchestrator's window off the async runtime;
    /// `true` when it was sent.
    async fn type_into(&self, window: TmuxWindow, line: String) -> bool {
        let runner = self.runner.clone();
        let sent =
            tokio::task::spawn_blocking(move || tmux::send_keys(&window, &line, &*runner)).await;
        matches!(sent, Ok(Ok(())))
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
                self.start_session(host, &o, epics, orchestrations, tasks)
                    .await;
                if self.opener.pending().remove(&o.epic_id) {
                    let runner = self.runner.clone();
                    let window = TmuxWindow::for_orchestrator(o.epic_id);
                    let shown =
                        tokio::task::spawn_blocking(move || tmux::select_window(&window, &*runner))
                            .await;
                    if !matches!(shown, Ok(Ok(()))) {
                        tracing::warn!(orchestrator = %o.name, "could not switch to the orchestrator's window");
                    }
                }
            }
            OrchestratorAction::End(o) => {
                if let Some(window) = o.tmux_window.clone() {
                    if !self.close_window(window).await {
                        tracing::warn!(orchestrator = %o.name, "could not close the orchestrator's window");
                    }
                }
                if let Err(e) = db.end_orchestrator_session(o.id, host, false).await {
                    tracing::warn!(orchestrator = %o.name, "could not end the orchestrator's session: {e:#}");
                    return;
                }
                let done = epics
                    .iter()
                    .any(|e| e.id == o.epic_id && e.status == TaskStatus::Done);
                if done {
                    self.tell_board(crate::tui::Message::Epic(
                        crate::tui::messages::EpicMessage::EpicFinished(o.epic_id),
                    ));
                }
            }
            OrchestratorAction::GiveUp(o) => {
                // The store raises the item once per bout, and again if it was
                // resolved meanwhile; a tick that already holds it asks nothing.
                let held = db.list_waiting_items().await.into_iter().any(|i| {
                    i.epic_id == o.epic_id && i.host == host && i.task_id.is_none() && i.is_open()
                });
                if held {
                    return;
                }
                if let Err(e) = db.raise_orchestrator_down_item(o.id, host).await {
                    tracing::warn!(orchestrator = %o.name, "could not raise the down orchestrator's item: {e:#}");
                }
            }
            OrchestratorAction::CloseGone(window) => {
                let name = window.as_str().to_string();
                if self.close_window(window).await {
                    tracing::info!(window = %name, "closed the window of an orchestrator that was switched off");
                }
            }
        }
    }

    /// Close an orchestrator's window off the async runtime; `true` when it
    /// is gone.
    async fn close_window(&self, window: TmuxWindow) -> bool {
        let runner = self.runner.clone();
        let closed = tokio::task::spawn_blocking(move || {
            crate::dispatch::end_orchestrator(&window, &*runner)
        })
        .await;
        matches!(closed, Ok(Ok(())))
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
        let dir = crate::dispatch::orchestrator_dir(&self.data_dir, o.epic_id);
        // A session that died may have died of its conversation; the next one
        // starts fresh, and the prompt carries what it needs (O13).
        let resume =
            !o.session_died && crate::dispatch::holds_a_conversation(&self.claude_json, &dir);
        if let Err(e) = db.start_orchestrator_session(o.id, host, resume).await {
            tracing::warn!(orchestrator = %o.name, "the store refused the orchestrator's start: {e:#}");
            return;
        }
        let prompt = self.prompt(o, epic, on, epics, tasks).await;
        let window = TmuxWindow::for_orchestrator(o.epic_id);
        let runner = self.runner.clone();
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
                    resume,
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
        let ids = crate::models::epics::descendant_epic_ids(o.epic_id, epics);
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
        let memory = self.memory(epic).await;
        crate::dispatch::build_orchestrator_prompt(&o.name, epic, on, &listed, &memory)
    }

    /// `orchestrators.allium: EnoughForAFreshStart`, `GivenItsLearnings`.
    async fn memory(&self, epic: &Epic) -> crate::dispatch::OrchestratorMemory {
        let db = &self.database;
        let (notes, open_items, log, learnings) = tokio::join!(
            db.list_epic_notes(),
            db.list_waiting_items(),
            db.orchestrator_log_of_epic(epic.id),
            crate::dispatch::list_learnings_for_epic(&**db, epic, &self.emb_svc)
        );
        crate::dispatch::OrchestratorMemory {
            notes,
            open_items,
            log,
            learnings,
        }
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
        plan_orchestrators(HOST, epics, switches, tasks, rows, names, &BTreeSet::new())
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

        let died = Orchestrator {
            session_died: true,
            ..rows[0].clone()
        };
        assert_eq!(actions, vec![WindowLost(rows[0].clone()), Start(died)]);
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
    fn the_window_of_a_row_seen_here_that_went_is_closed() {
        let names = windows(&["orch-7", "orch-1"]);
        let epics = [EpicBuilder::new(1).build()];
        let tasks = [task(10, 1, TaskStatus::Running)];
        let rows = [live(row(5, 1, HOST))];
        let seen = BTreeSet::from([EpicId(1), EpicId(7)]);

        let actions = plan_orchestrators(
            HOST,
            &epics,
            &[switch(1)],
            &tasks,
            &rows,
            Some(&names),
            &seen,
        );

        assert_eq!(
            actions,
            vec![CloseGone(TmuxWindow::for_orchestrator(EpicId(7)))]
        );
    }

    /// Another board on the same tmux server, or this board before its rows
    /// arrive, sees windows it never saw a row for: it closes none of them.
    #[test]
    fn a_window_this_board_never_saw_a_row_for_is_left_alone() {
        let names = windows(&["orch-7", "orch-1"]);
        let epics = [EpicBuilder::new(1).build()];

        assert!(plan(&epics, &[], &[], &[], Some(&names)).is_empty());
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
                // Tick 3: the plan's listing; the epic is done, so the summary
                // ask's two send-keys.
                listing(&shown),
                MockProcessRunner::ok(),
                MockProcessRunner::ok(),
                // Tick 4: the plan's listing, the end's window check, the kill.
                listing(&shown),
                listing(&shown),
                MockProcessRunner::ok(),
            ])
            .with_windows(&[window.as_str()]),
        );
        let tmp = tempfile::tempdir().unwrap();
        let claude_json = tmp.path().join(".claude.json");
        let (board, mut heard) = tokio::sync::mpsc::unbounded_channel();
        let host = OrchestratorHost::new(
            db.clone(),
            runner.clone(),
            tmp.path().join("data"),
            claude_json,
            crate::embeddings::EmbeddingService::new_noop(),
        )
        .reporting_to(board);
        let finished = |heard: &mut tokio::sync::mpsc::UnboundedReceiver<crate::tui::Message>| {
            std::iter::from_fn(|| heard.try_recv().ok())
                .filter(|m| {
                    matches!(
                        m,
                        crate::tui::Message::Epic(crate::tui::messages::EpicMessage::EpicFinished(
                            _
                        ))
                    )
                })
                .count()
        };

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
        assert!(row.is_live(), "it waits for the closing summary");
        assert!(
            runner
                .flattened_calls()
                .iter()
                .any(|c| c.contains("record your closing summary")),
            "{:?}",
            runner.flattened_calls()
        );

        assert_eq!(finished(&mut heard), 0, "not before the session ends");
        db.record_orchestrator_summary(&row.name, "delivered the step")
            .await
            .unwrap();
        host.tick().await;
        // NotifyEpicFinished: the board hears of the finish once.
        assert_eq!(finished(&mut heard), 1);
        let row = db.list_orchestrators().await.remove(0);
        assert!(!row.is_live());
        assert!(!row.session_died, "the board ended it");
        assert!(runner
            .flattened_calls()
            .iter()
            .any(|c| c.contains("kill-window")));
    }

    /// agent-health.allium: WakeTheOrchestrator. A live window the board did
    /// not start is told the board restarted; after that, what changed.
    #[tokio::test]
    async fn an_adopted_session_is_told_of_the_restart_then_of_what_changed() {
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
        let host_id = crate::store::HostStore::host_id(&*db).to_string();
        db.ensure_orchestrator(epic.id, &host_id).await.unwrap();
        let id = db.list_orchestrators().await[0].id;
        db.start_orchestrator_session(id, &host_id, false)
            .await
            .unwrap();
        let window = TmuxWindow::for_orchestrator(epic.id);
        let shown = format!("{}\n", window.as_str());
        let runner = Arc::new(
            MockProcessRunner::new(vec![
                // Tick 1: the plan's listing, then the wake's two send-keys.
                listing(&shown),
                MockProcessRunner::ok(),
                MockProcessRunner::ok(),
                // Tick 2: the same.
                listing(&shown),
                MockProcessRunner::ok(),
                MockProcessRunner::ok(),
                // Tick 3: the plan's listing only: nothing changed.
                listing(&shown),
            ])
            .with_windows(&[window.as_str()]),
        );
        let tmp = tempfile::tempdir().unwrap();
        let host = OrchestratorHost::new(
            db.clone(),
            runner.clone(),
            tmp.path().join("data"),
            tmp.path().join(".claude.json"),
            crate::embeddings::EmbeddingService::new_noop(),
        );
        let typed = |runner: &MockProcessRunner| -> Vec<String> {
            runner
                .flattened_calls()
                .into_iter()
                .filter(|c| c.contains("[dispatch]"))
                .collect()
        };

        host.tick().await;
        let lines = typed(&runner);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("the board restarted"), "{lines:?}");

        db.mark_task_ready(task, crate::models::ReadyMarker::Operator, None)
            .await
            .unwrap();
        host.tick().await;
        let lines = typed(&runner);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines[1].contains(&format!("task #{} was marked ready", task.0)),
            "{lines:?}"
        );

        host.tick().await;
        assert_eq!(typed(&runner).len(), 2, "a quiet tick types nothing");
    }

    /// orchestrators.allium: OrchestratorKey — the board asks for the session
    /// now, and its window is switched to once it starts.
    #[tokio::test]
    async fn an_open_starts_the_session_and_switches_to_its_window() {
        let db = crate::runtime::tests::test_db().await;
        let epic = db.create_epic("E", "", None).await.unwrap();
        db.switch_orchestrator_on(epic.id, None, None)
            .await
            .unwrap();
        db.create_task(crate::models::CreateTaskRequest {
            epic_id: Some(epic.id),
            ..crate::models::CreateTaskRequest::fixture("t", "/repo")
        })
        .await
        .unwrap();
        let host_id = crate::store::HostStore::host_id(&*db).to_string();
        db.ensure_orchestrator(epic.id, &host_id).await.unwrap();
        let runner = Arc::new(MockProcessRunner::new(vec![
            // The plan's listing, the launch's two window checks, the new
            // window, then the switch to it.
            listing(""),
            listing(""),
            listing(""),
            MockProcessRunner::ok(),
            MockProcessRunner::ok(),
        ]));
        let tmp = tempfile::tempdir().unwrap();
        let host = OrchestratorHost::new(
            db.clone(),
            runner.clone(),
            tmp.path().join("data"),
            tmp.path().join(".claude.json"),
            crate::embeddings::EmbeddingService::new_noop(),
        );

        host.opener().open_after_start(epic.id);
        host.tick().await;

        let calls = runner.flattened_calls();
        assert!(
            calls.iter().any(|c| c.contains("select-window")),
            "{calls:?}"
        );
        assert!(host.opener().pending().is_empty());
    }

    /// An open that finds nothing to start says so to the board.
    #[tokio::test]
    async fn an_open_with_no_open_work_tells_the_board() {
        let db = crate::runtime::tests::test_db().await;
        let epic = db.create_epic("Quiet", "", None).await.unwrap();
        db.switch_orchestrator_on(epic.id, None, None)
            .await
            .unwrap();
        let runner = Arc::new(MockProcessRunner::new(vec![listing("")]));
        let tmp = tempfile::tempdir().unwrap();
        let (board, mut heard) = tokio::sync::mpsc::unbounded_channel();
        let host = OrchestratorHost::new(
            db.clone(),
            runner,
            tmp.path().join("data"),
            tmp.path().join(".claude.json"),
            crate::embeddings::EmbeddingService::new_noop(),
        )
        .reporting_to(board);

        host.opener().open_after_start(epic.id);
        host.tick().await;

        let said: Vec<String> = std::iter::from_fn(|| heard.try_recv().ok())
            .filter_map(|m| match m {
                crate::tui::Message::System(crate::tui::messages::SystemMessage::StatusInfo(s)) => {
                    Some(s)
                }
                _ => None,
            })
            .collect();
        assert!(
            said.iter().any(|s| s.contains("Quiet did not start")),
            "{said:?}"
        );
    }

    /// orchestrators.allium: OrchestratorThatKeepsDyingWaitsForTheOperator.
    /// Every tick that finds the orchestrator down asks, and the store raises
    /// one item for the bout.
    #[tokio::test]
    async fn a_down_orchestrator_raises_one_item_however_many_ticks_see_it() {
        let db = crate::runtime::tests::test_db().await;
        let epic = db.create_epic("E", "", None).await.unwrap();
        db.switch_orchestrator_on(epic.id, None, None)
            .await
            .unwrap();
        db.create_task(crate::models::CreateTaskRequest {
            epic_id: Some(epic.id),
            ..crate::models::CreateTaskRequest::fixture("t", "/repo")
        })
        .await
        .unwrap();
        let host_id = crate::store::HostStore::host_id(&*db).to_string();
        db.ensure_orchestrator(epic.id, &host_id).await.unwrap();
        let id = db.list_orchestrators().await[0].id;
        for _ in 0..2 {
            db.start_orchestrator_session(id, &host_id, false)
                .await
                .unwrap();
            db.end_orchestrator_session(id, &host_id, true)
                .await
                .unwrap();
        }
        let runner = Arc::new(MockProcessRunner::new(vec![listing(""), listing("")]));
        let tmp = tempfile::tempdir().unwrap();
        let host = OrchestratorHost::new(
            db.clone(),
            runner,
            tmp.path().join("data"),
            tmp.path().join(".claude.json"),
            crate::embeddings::EmbeddingService::new_noop(),
        );

        host.tick().await;
        host.tick().await;

        let items = db.list_waiting_items().await;
        assert_eq!(items.len(), 1, "{items:?}");
        assert!(items[0].is_open() && items[0].task_id.is_none());
        assert_eq!(items[0].about, dispatch_spacetime_module::DOWN_ABOUT);
    }
}
