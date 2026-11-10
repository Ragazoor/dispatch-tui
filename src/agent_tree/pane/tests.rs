use super::*;
use std::time::Duration;

const LIMIT: Duration = Duration::from_secs(10);

fn found() -> TaskLookup {
    TaskLookup::Found {
        root: PathBuf::from("/wt"),
        base_branch: "main".into(),
    }
}

#[test]
fn a_missing_row_is_waited_for_inside_the_limit() {
    let step = startup_step(&TaskLookup::NotYet, Duration::from_secs(9), LIMIT);
    assert_eq!(step, StartupStep::Wait);
}

#[test]
fn a_row_that_arrives_proceeds() {
    let step = startup_step(&found(), Duration::from_secs(2), LIMIT);
    assert_eq!(
        step,
        StartupStep::Proceed {
            root: PathBuf::from("/wt"),
            base_branch: "main".into()
        }
    );
}

#[test]
fn a_row_arriving_at_the_limit_still_proceeds() {
    let step = startup_step(&found(), LIMIT, LIMIT);
    assert!(matches!(step, StartupStep::Proceed { .. }));
}

#[test]
fn a_row_that_never_arrives_fails_with_not_found_at_the_limit() {
    let step = startup_step(&TaskLookup::NotYet, LIMIT, LIMIT);
    assert_eq!(step, StartupStep::Fail("this task was not found".into()));
}

/// The pane is split off when the agent launches and the worktree is recorded
/// just after, so a row without one is waited out like a missing row.
#[test]
fn a_task_without_a_worktree_is_waited_for() {
    let step = startup_step(&TaskLookup::NoWorktree, Duration::ZERO, LIMIT);
    assert_eq!(step, StartupStep::Wait);
}

#[test]
fn a_task_that_never_gets_a_worktree_fails_at_the_limit() {
    let step = startup_step(&TaskLookup::NoWorktree, LIMIT, LIMIT);
    assert_eq!(step, StartupStep::Fail("this task has no worktree".into()));
}

fn drawn(step: &StartupStep) -> String {
    let backend = ratatui::backend::TestBackend::new(40, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_startup_notice(
                f,
                f.area(),
                step,
                crate::keybindings::KeyNamespace::AgentTreeTree,
            )
        })
        .unwrap();
    buffer_to_string(terminal.backend().buffer())
}

#[test]
fn the_pane_says_what_it_is_waiting_for() {
    assert!(drawn(&StartupStep::Wait).contains("waiting for the task"));
}

#[test]
fn the_pane_draws_the_failure_text() {
    let out = drawn(&StartupStep::Fail("this task was not found".into()));
    assert!(out.contains("this task was not found"), "{out}");
}

// -- PanesReadThroughTheBoard (task #4982) -------------------------------

use crate::hooks::wire::{PaneTask, PaneView};

const BOARD: &str = "127.0.0.1:8899";

fn unreachable() -> TaskLookup {
    TaskLookup::BoardUnreachable {
        address: BOARD.into(),
    }
}

/// A board that is mid-restart is waited out like a row that has not
/// arrived: it is most often back within the wait
/// (agent-tree.allium: "Companion Pane Startup").
#[test]
fn an_unreachable_board_is_waited_out_inside_the_limit() {
    let step = startup_step(&unreachable(), Duration::from_secs(3), LIMIT);
    assert_eq!(step, StartupStep::Wait);
}

/// AgentTreePaneTaskNeverArrives: when the last attempt could not reach
/// the board, the pane says THAT, naming the address it tried -- "not
/// found" would send the user looking for a task that may well exist.
#[test]
fn an_unreachable_board_at_the_limit_is_named_not_reported_as_a_missing_task() {
    let StartupStep::Fail(reason) = startup_step(&unreachable(), LIMIT, LIMIT) else {
        panic!("an unreachable board at the limit must end the wait");
    };
    assert!(
        reason.contains(BOARD),
        "must name the board address: {reason}"
    );
    assert!(
        !reason.contains("not found"),
        "an unreachable board is not a missing task: {reason}"
    );
}

/// One scripted answer per read. `Err` stands for a board that could not
/// be reached or answered something unreadable.
struct FakeBoard {
    answer: std::result::Result<PaneView, String>,
}

#[async_trait::async_trait]
impl PaneViewSource for FakeBoard {
    async fn pane_view(&self, _task_id: TaskId) -> Result<PaneView> {
        self.answer.clone().map_err(|e| anyhow::anyhow!(e))
    }
    async fn resume_agent(&self, _task_id: TaskId) -> Result<crate::hooks::wire::PaneResumeAnswer> {
        anyhow::bail!("not asked in these tests")
    }
    fn board_address(&self) -> String {
        BOARD.into()
    }
}

fn view(task: Option<PaneTask>) -> FakeBoard {
    FakeBoard {
        answer: Ok(PaneView {
            task,
            live_agents: vec![],
            detached_agents: vec![],
            hidden_agents: 0,
        }),
    }
}

#[tokio::test]
async fn a_task_the_board_holds_with_a_worktree_is_found() {
    let board = view(Some(PaneTask {
        worktree: Some("/wt/7".into()),
        base_branch: "develop".into(),
    }));
    assert_eq!(
        lookup_from_board(&board, TaskId(7)).await,
        TaskLookup::Found {
            root: PathBuf::from("/wt/7"),
            base_branch: "develop".into()
        }
    );
}

#[tokio::test]
async fn a_task_the_board_holds_without_a_worktree_has_none() {
    let board = view(Some(PaneTask {
        worktree: None,
        base_branch: "main".into(),
    }));
    assert_eq!(
        lookup_from_board(&board, TaskId(7)).await,
        TaskLookup::NoWorktree
    );
}

/// PaneView.task = null: the row has not reached the board's rows yet.
#[tokio::test]
async fn a_task_the_board_does_not_hold_yet_is_not_yet() {
    assert_eq!(
        lookup_from_board(&view(None), TaskId(7)).await,
        TaskLookup::NotYet
    );
}

/// A failed read is its own answer, carrying the address -- never read as
/// "no such task".
#[tokio::test]
async fn a_board_that_does_not_answer_is_unreachable_not_missing() {
    let board = FakeBoard {
        answer: Err("connection refused".into()),
    };
    assert_eq!(lookup_from_board(&board, TaskId(7)).await, unreachable());
}

// ---- RefreshSchedule ---------------------------------------------------

/// Held keys must not starve the refresh: it is due on elapsed time alone.
#[test]
fn a_refresh_is_due_once_the_interval_has_elapsed_whatever_arrived_meanwhile() {
    use std::time::{Duration, Instant};
    let start = Instant::now();
    let interval = Duration::from_secs(1);
    let mut schedule = RefreshSchedule::new(interval, start);

    assert!(!schedule.due(start + Duration::from_millis(900)));
    assert_eq!(
        schedule.wait(start + Duration::from_millis(900)),
        Duration::from_millis(100)
    );
    assert!(schedule.due(start + interval));
    assert_eq!(
        schedule.wait(start + Duration::from_secs(5)),
        Duration::ZERO
    );

    schedule.mark(start + Duration::from_secs(5));
    assert!(!schedule.due(start + Duration::from_millis(5500)));
    assert!(schedule.due(start + Duration::from_secs(6)));
}
