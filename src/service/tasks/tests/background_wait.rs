//! A Stop that lists background work keeps the task running until a later
//! Stop lists none or the cap runs out (`agent-health.allium: HookStop`,
//! `ExpireBackgroundWait`, `HookNotification`), board side, against the
//! in-memory store and a fixed clock.

use super::*;

use chrono::TimeZone;

use crate::clock::FixedClock;
use crate::models::{BackgroundWork, NotificationKind, BACKGROUND_WAIT_CAP};
use crate::process::MockProcessRunner;
use crate::sync::{MemoryReducerCaller, SettledIdentity, SharedRows};

const HOST: &str = "m5-host";

struct Fx {
    db: Arc<dyn store::TaskStore>,
    clock: FixedClock,
    _host_dir: tempfile::TempDir,
}

fn fx() -> Fx {
    let rows = Arc::new(SharedRows::new());
    let clock = FixedClock::new(
        chrono::Utc
            .with_ymd_and_hms(2026, 10, 12, 12, 0, 0)
            .unwrap(),
    );
    let caller = Arc::new(
        MemoryReducerCaller::new(rows.clone(), Arc::new(clock.clone()))
            .with_sender("test-user")
            .with_registered_host(HOST, "test-user"),
    );
    let identity = Arc::new(SettledIdentity::default());
    identity.settle("test-user");
    let host_dir = tempfile::tempdir().unwrap();
    let db: Arc<dyn store::TaskStore> = Arc::new(Store::new(
        rows,
        caller,
        identity,
        Arc::new(clock.clone()),
        HOST.to_string(),
        host_dir.path(),
    ));
    Fx {
        db,
        clock,
        _host_dir: host_dir,
    }
}

impl Fx {
    fn svc(&self) -> TaskService {
        TaskService::new(self.db.clone(), MockProcessRunner::unused())
            .with_clock(Arc::new(self.clock.clone()))
    }

    /// A task dispatched on this host and provisioned: running, with a
    /// current agent and a window.
    async fn running_task(&self) -> TaskId {
        let svc = self.svc();
        let id = svc
            .create_task(make_task_params_on_branch("/repo", "main"))
            .await
            .unwrap();
        let request = match svc
            .request_dispatch(
                id,
                crate::models::DispatchOrigin::User,
                Some(crate::models::DispatchMode::Dispatch),
            )
            .await
            .unwrap()
        {
            crate::store::RequestDispatchAnswer::Requested(r) => r,
            other => panic!("the dispatch request must apply: {other:?}"),
        };
        assert!(self.db.claim_dispatch_request(request).await.unwrap());
        assert!(self
            .db
            .report_dispatch_provisioned(
                request,
                "/repo/.worktrees/1-t",
                &crate::models::test_tmux_window(&format!("task-{}", id.0)),
                false
            )
            .await
            .unwrap());
        assert_eq!(svc.get_task(id).await.unwrap().status, TaskStatus::Running);
        id
    }
}

fn one_shell() -> BackgroundWork {
    BackgroundWork {
        shells: 1,
        ..BackgroundWork::default()
    }
}

/// `HookStop`: background work in the list keeps the task running and the
/// board sees what it waits on.
#[tokio::test]
async fn a_stop_listing_background_work_keeps_the_task_running() {
    let fx = fx();
    let id = fx.running_task().await;
    let svc = fx.svc();
    let work = BackgroundWork {
        shells: 2,
        workflows: 1,
        monitors: 0,
        mcp_tasks: 1,
    };

    svc.record_stop(id, work).await.unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert!(task.waiting_on_background());
    assert_eq!(task.background_work, work);
}

/// `HookStop`: the wake-up turn's Stop lists nothing and ends the wait.
#[tokio::test]
async fn the_next_stop_with_an_empty_list_moves_the_task_to_review() {
    let fx = fx();
    let id = fx.running_task().await;
    let svc = fx.svc();
    svc.record_stop(id, one_shell()).await.unwrap();
    svc.record_hook_event(id, HookEventKind::UserPromptSubmit)
        .await
        .unwrap();

    svc.record_stop(id, BackgroundWork::default())
        .await
        .unwrap();

    assert_eq!(svc.get_task(id).await.unwrap().status, TaskStatus::Review);
}

/// The plain `stop` hook event carries no list and behaves as before.
#[tokio::test]
async fn a_stop_event_without_counts_moves_the_task_to_review() {
    let fx = fx();
    let id = fx.running_task().await;
    let svc = fx.svc();

    svc.record_hook_event(id, HookEventKind::Stop)
        .await
        .unwrap();

    assert_eq!(svc.get_task(id).await.unwrap().status, TaskStatus::Review);
}

/// The observed chain: Stop(shell), idle_prompt a minute later, the
/// wake-up prompt, Stop(nothing). Running until the last step.
#[tokio::test]
async fn the_observed_wake_up_chain_stays_running_until_the_last_stop() {
    let fx = fx();
    let id = fx.running_task().await;
    let svc = fx.svc();

    svc.record_stop(id, one_shell()).await.unwrap();
    fx.clock.advance(chrono::Duration::seconds(60));
    svc.record_hook_event(
        id,
        HookEventKind::Notification(Some(NotificationKind::IdlePrompt)),
    )
    .await
    .unwrap();
    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_ne!(task.sub_status, SubStatus::NeedsInput);
    assert!(task.last_notification_at.is_none());

    fx.clock.advance(chrono::Duration::seconds(60));
    svc.record_hook_event(id, HookEventKind::UserPromptSubmit)
        .await
        .unwrap();
    assert_eq!(svc.get_task(id).await.unwrap().status, TaskStatus::Running);

    fx.clock.advance(chrono::Duration::seconds(5));
    svc.record_stop(id, BackgroundWork::default())
        .await
        .unwrap();
    assert_eq!(svc.get_task(id).await.unwrap().status, TaskStatus::Review);
}

/// `ExpireBackgroundWait`: inside the cap the task keeps waiting.
#[tokio::test]
async fn the_wait_holds_inside_the_cap() {
    let fx = fx();
    let id = fx.running_task().await;
    let svc = fx.svc();
    svc.record_stop(id, one_shell()).await.unwrap();

    fx.clock
        .advance(BACKGROUND_WAIT_CAP - chrono::Duration::seconds(1));
    svc.expire_background_wait(id).await.unwrap();

    assert_eq!(svc.get_task(id).await.unwrap().status, TaskStatus::Running);
}

/// `ExpireBackgroundWait`: once the cap has passed, the Stop applies.
#[tokio::test]
async fn the_wait_ends_in_review_once_the_cap_passes() {
    let fx = fx();
    let id = fx.running_task().await;
    let svc = fx.svc();
    svc.record_stop(id, one_shell()).await.unwrap();

    fx.clock
        .advance(BACKGROUND_WAIT_CAP + chrono::Duration::seconds(1));
    svc.expire_background_wait(id).await.unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    assert!(!task.waiting_on_background());
}

/// The cap counts from the latest tool call too.
#[tokio::test]
async fn a_tool_call_during_the_wait_restarts_the_cap() {
    let fx = fx();
    let id = fx.running_task().await;
    let svc = fx.svc();
    svc.record_stop(id, one_shell()).await.unwrap();
    fx.clock.advance(chrono::Duration::minutes(20));
    svc.record_hook_event(id, HookEventKind::PreToolUse)
        .await
        .unwrap();

    fx.clock.advance(chrono::Duration::minutes(20));
    svc.expire_background_wait(id).await.unwrap();

    assert_eq!(svc.get_task(id).await.unwrap().status, TaskStatus::Running);
}
