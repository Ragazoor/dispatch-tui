// ---------------------------------------------------------------------------
// commands::dispatch — Command → side-effect coverage
// ---------------------------------------------------------------------------
//
// `commands::dispatch` (`src/runtime/commands.rs`) is the single entry point
// `execute_commands` uses to turn a `Command` into a side effect. The rest of
// this file exercises the `exec_*` handlers directly, which leaves the
// dispatcher itself — the wiring from variant to handler, and which arms feed
// follow-on commands back into the queue — unexecuted.
//
// Assertions here are on *observable effects* (DB rows, `App` state, returned
// follow-on commands), never on the shape of the match, so they survive a
// refactor of the dispatcher's internals.
use super::*;
use crate::models::test_tmux_window;

use crate::tui::commands::{
    EditorCommand, FeedCommand, PersistFields, PrCommand, RepoSyncCommand, SplitCommand,
    SystemCommand, TaskCommand,
};

/// Mirror of `execute_commands`' drain loop without the terminal/key-rx
/// plumbing: run `cmd` and every command it cascades into.
async fn drain(rt: &TuiRuntime, app: &mut App, cmd: Command) {
    let mut queue = std::collections::VecDeque::from(vec![cmd]);
    while let Some(command) = queue.pop_front() {
        queue.extend(commands::dispatch(command, app, rt).await);
    }
}

async fn seed(rt: &TuiRuntime, title: &str, status: models::TaskStatus) -> models::Task {
    create_task_returning(&**rt.db_write(), title, "desc", "/repo", None, status)
        .await
        .unwrap()
}

// -----------------------------------------------------------------------
// The process-effect half
// -----------------------------------------------------------------------
//
// Everything above asserts on a DB row or on `App`. The arms below reach
// tmux, git, `gh`, `notify-send` or `xdg-open` instead, and nearly all of
// them do it from a detached `spawn_blocking` whose `JoinHandle` the
// dispatcher `drop`s. Two consequences shape this section:
//
// - There is nothing to `await`, so every test needs a completion signal.
//   Most arms send a `Message`; the ones that succeed silently
//   (`Split::FocusPane`, `Task::KillTmuxWindow`, `System::*`) are covered
//   through [`Harness::await_call`] instead.
// - A `MockProcessRunner` panic on a detached thread does *not* fail the
//   test (KB #336), so an under-scripted runner reads as a pass. Waiting on
//   a signal that only the arm under test can produce is what closes that
//   hole: the runner panicking means the signal never arrives and the wait
//   times out.

/// A `ProcessRunner` that announces every call it forwards.
///
/// The completion signal for the arms that report nothing. It wraps a real
/// [`MockProcessRunner`] rather than replacing it, so the queue semantics,
/// the out-of-band window lookup and `recorded_calls` all still apply.
struct AnnouncingRunner {
    inner: Arc<MockProcessRunner>,
    tx: mpsc::UnboundedSender<String>,
}

impl AnnouncingRunner {
    /// Announce *after* the inner call returns, so observing the signal
    /// implies the side effect already happened.
    fn announce(&self, program: &str, args: &[&str]) {
        let _ = self.tx.send(format!("{program} {}", args.join(" ")));
    }
}

impl ProcessRunner for AnnouncingRunner {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<std::process::Output> {
        let result = self.inner.run(program, args);
        self.announce(program, args);
        result
    }

    fn run_with_timeout(
        &self,
        program: &str,
        args: &[&str],
        timeout: Duration,
    ) -> anyhow::Result<std::process::Output> {
        let result = self.inner.run_with_timeout(program, args, timeout);
        self.announce(program, args);
        result
    }

    fn agent_binaries(&self) -> crate::process::AgentBinaries {
        self.inner.agent_binaries()
    }
}

/// A runtime, its board, and the two channels a detached arm reports on.
struct Harness {
    rt: TuiRuntime,
    app: App,
    msgs: mpsc::UnboundedReceiver<Message>,
    calls: mpsc::UnboundedReceiver<String>,
    mock: Arc<MockProcessRunner>,
    db: Arc<Store>,
}

async fn harness(mock: MockProcessRunner) -> Harness {
    let db = test_db().await;
    let (tx, msgs) = mpsc::unbounded_channel();
    let (call_tx, calls) = mpsc::unbounded_channel();
    let mock = Arc::new(mock);
    let runner: Arc<dyn ProcessRunner> = Arc::new(AnnouncingRunner {
        inner: Arc::clone(&mock),
        tx: call_tx,
    });
    let rt = make_runtime(db.clone(), tx, runner).await;
    let app = App::new(db.list_all().await);
    Harness {
        rt,
        app,
        msgs,
        calls,
        mock,
        db,
    }
}

/// A harness whose runner is expected to run nothing at all.
async fn quiet_harness() -> Harness {
    harness(MockProcessRunner::new(vec![])).await
}

impl Harness {
    async fn dispatch(&mut self, cmd: Command) -> Vec<Command> {
        commands::dispatch(cmd, &mut self.app, &self.rt).await
    }

    /// Serve the dispatch requests the arms under test wrote, the way this
    /// host's reactor would once the row pump delivers them
    /// (`store-split.allium: HostClaimsDispatch`), and wait for each to land.
    /// Since store-split M4 no launch arm provisions inline: it only asks.
    async fn serve_requests(&self) {
        for handle in self.rt.serve_dispatch_requests().await {
            handle.await.unwrap();
        }
    }

    /// Put `task` on the board, which several arms require before their
    /// result message is applied at all.
    fn seed_board(&mut self, task: models::Task) {
        self.app
            .update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
                vec![task],
            )));
    }

    async fn next_msg(&mut self) -> Message {
        tokio::time::timeout(TEST_TIMEOUT, self.msgs.recv())
            .await
            .expect("the arm under test must report well within the timeout")
            .expect("the runtime's message sender should still be alive")
    }

    /// Await the first announced call whose command line contains `needle`.
    ///
    /// The completion signal for an arm that succeeds silently. A timeout
    /// here means the arm never ran, never reached the runner, or panicked
    /// the mock on a detached thread — all of which would otherwise pass.
    async fn await_call(&mut self, needle: &str) -> String {
        let found = tokio::time::timeout(TEST_TIMEOUT, async {
            while let Some(call) = self.calls.recv().await {
                if call.contains(needle) {
                    return Some(call);
                }
            }
            None
        })
        .await;
        match found {
            Ok(Some(call)) => call,
            Ok(None) => panic!("the runner was dropped before any call contained {needle:?}"),
            Err(_) => panic!(
                "no subprocess call containing {needle:?} was made; recorded: {:?}",
                self.mock.flattened_calls()
            ),
        }
    }
}

// --- LearningCommand ---------------------------------------------------

/// Records that the sweep reached it. Only `archive_stale_learnings` is
/// implemented; every other seam method keeps the panicking stub default,
/// which is what proves the arm called this one and nothing else.
struct CountingSweep {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl crate::service::LearningServiceApiStub for CountingSweep {
    async fn archive_stale_learnings(
        &self,
        _cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64, crate::service::ServiceError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(0)
    }
}

crate::learning_service_api!(service_api_stub_bridge, CountingSweep);

// -----------------------------------------------------------------------
// Delete pre-check: refuse before any side effect
// -----------------------------------------------------------------------

fn refused(msg: &Message) -> bool {
    matches!(
        msg,
        Message::Task(crate::tui::messages::TaskMessage::DeleteRefused { .. })
    )
}

mod agent_dispatch;
mod guards;
mod host_effects;
mod split_pane;
mod store_writes;
mod task_windows;
