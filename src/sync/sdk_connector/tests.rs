use super::answer::{answer_once, awaiting_answer, fire};
use super::outcome::*;
use crate::spacetime::bindings;
use crate::sync::writes::ReducerOutcome;
use spacetimedb_sdk::__codegen::InternalError;
use tokio::sync::oneshot;

type Raw = std::result::Result<std::result::Result<(), String>, InternalError>;

fn transport_error() -> Raw {
    Err(InternalError::failed_parse("Row", "table"))
}

fn task(title: &str) -> bindings::Task {
    bindings::Task {
        title: title.into(),
        repo_path: "/repo".into(),
        created_at: "2026-10-05 10:00:00".into(),
        created_by: "alice".into(),
        ..dispatch_spacetime_module::blank_task().into()
    }
}

fn epic(title: &str) -> bindings::Epic {
    bindings::Epic {
        title: title.into(),
        created_at: "2026-10-05 10:00:00".into(),
        created_by: "alice".into(),
        ..dispatch_spacetime_module::blank_epic().into()
    }
}

fn learning(summary: &str) -> bindings::Learning {
    bindings::Learning {
        id: 0,
        kind: "pitfall".into(),
        summary: summary.into(),
        detail: None,
        scope: "repo".into(),
        scope_ref: None,
        tags: String::new(),
        status: "active".into(),
        source_task_id: Some(7),
        upvote_count: 0,
        last_upvoted_at: None,
        created_at: "2026-10-05 10:00:00".into(),
        updated_at: "2026-10-05 10:00:00".into(),
        embedding: None,
    }
}

// --- outcome folding ---

#[test]
fn an_accepted_call_is_applied_with_no_ids() {
    assert_eq!(outcome_of(Ok(Ok(()))), ReducerOutcome::Applied(vec![]));
}

#[test]
fn a_refusal_and_a_transport_error_both_fold_to_refused() {
    assert_eq!(
        outcome_of(Ok(Err("no such task".into()))),
        ReducerOutcome::Refused("no such task".into())
    );
    assert!(matches!(
        outcome_of(transport_error()),
        ReducerOutcome::Refused(why) if why.contains("Failed to parse")
    ));
}

#[test]
fn a_create_scans_for_ids_only_when_it_was_accepted() {
    assert_eq!(
        outcome_with_ids(Ok(Ok(())), || vec![3, 4]),
        ReducerOutcome::Applied(vec![3, 4])
    );
    let refused = outcome_with_ids(Ok(Err("nope".into())), || {
        panic!("the scan must not run on a refusal")
    });
    assert_eq!(refused, ReducerOutcome::Refused("nope".into()));
    let failed = outcome_with_ids(transport_error(), || panic!("no scan on an error"));
    assert!(matches!(failed, ReducerOutcome::Refused(_)));
}

#[test]
fn the_newest_generated_id_is_the_one_read_back() {
    assert_eq!(
        generated_id(ReducerOutcome::Applied(vec![4, 9, 2]), "task").unwrap(),
        9
    );
}

#[test]
fn an_applied_create_with_no_matching_row_names_the_subscription_gap() {
    let err = generated_id(ReducerOutcome::Applied(vec![]), "epic").unwrap_err();
    let text = err.to_string();
    assert!(text.contains("created the epic"), "{text}");
    assert!(
        text.contains("outside this board's subscriptions"),
        "{text}"
    );
}

#[test]
fn a_refused_create_carries_the_stores_reason() {
    let err = generated_id(ReducerOutcome::Refused("bad epic".into()), "task").unwrap_err();
    assert!(err.to_string().contains("refused: bad epic"));
}

#[test]
fn value_or_bail_separates_a_refusal_from_a_transport_failure() {
    assert_eq!(value_or_bail(Ok(Ok(())), "x", || 5).unwrap(), 5);
    let refused = value_or_bail(Ok(Err("why".into())), "the drain", || 5).unwrap_err();
    assert!(refused.to_string().contains("refused the drain"));
    assert!(refused.to_string().contains("why"));
    let failed = value_or_bail(transport_error(), "the drain", || 5).unwrap_err();
    assert!(failed.to_string().contains("could not answer the drain"));
}

#[test]
fn a_flag_is_none_for_a_refusal_and_for_a_transport_error() {
    assert_eq!(flag_or_refused(Ok(Ok(())), || 1), Some(1));
    assert_eq!(flag_or_refused(Ok(Err("no".into())), || 1), None);
    assert_eq!(flag_or_refused(transport_error(), || 1), None);
}

#[test]
fn only_the_review_spelling_counts_as_review() {
    assert!(is_review("review"));
    assert!(!is_review("Review"));
    assert!(!is_review("running"));
}

// --- create read-back matching ---

#[test]
fn a_task_matches_only_on_every_client_chosen_field() {
    let sent = task("A");
    assert!(matches_create(&task("A"), &sent));
    assert!(!matches_create(&task("B"), &sent));
    let mut other_author = task("A");
    other_author.created_by = "bob".into();
    assert!(!matches_create(&other_author, &sent));
    let mut other_time = task("A");
    other_time.created_at = "2026-10-05 10:00:01".into();
    assert!(!matches_create(&other_time, &sent));
    let mut other_epic = task("A");
    other_epic.epic_id = 2;
    assert!(!matches_create(&other_epic, &sent));
}

#[test]
fn an_epic_matches_only_its_own_creator_title_parent_and_time() {
    let sent = epic("E");
    assert!(matches_created_epic(&epic("E"), &sent));
    assert!(!matches_created_epic(&epic("F"), &sent));
    let mut colleague = epic("E");
    colleague.created_by = "bob".into();
    assert!(!matches_created_epic(&colleague, &sent));
    let mut nested = epic("E");
    nested.parent_epic_id = 9;
    assert!(!matches_created_epic(&nested, &sent));
}

#[test]
fn a_learning_matches_on_content_not_on_the_generated_id() {
    let sent = learning("L");
    let mut stored = learning("L");
    stored.id = 42;
    assert!(matches_created_learning(&stored, &sent));
    assert!(!matches_created_learning(&learning("M"), &sent));
    let mut other_source = learning("L");
    other_source.source_task_id = Some(8);
    assert!(!matches_created_learning(&other_source, &sent));
}

// --- answering ---

#[test]
fn the_first_answer_wins_and_a_second_is_a_no_op() {
    let (answer, mut rx) = answer_once::<i32>();
    let again = answer.clone();
    answer(1);
    again(2);
    assert_eq!(rx.try_recv().unwrap(), 1);
}

#[test]
fn firing_into_a_closed_receiver_does_not_panic() {
    let (tx, rx) = oneshot::channel::<i32>();
    drop(rx);
    fire(tx, 1);
}

#[tokio::test]
async fn a_reducer_answer_is_returned_to_the_waiting_caller() {
    let got = awaiting_answer("a save", |tx| {
        fire(tx, 11);
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(got, 11);
}

#[tokio::test]
async fn a_dropped_callback_says_the_write_may_have_landed() {
    let err = awaiting_answer::<i32, _>("a save", |tx| {
        drop(tx);
        Ok(())
    })
    .await
    .unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("dropped before a save was answered"),
        "{text}"
    );
    assert!(text.contains("may or may not"), "{text}");
}

#[tokio::test]
async fn a_request_that_cannot_be_sent_names_what_was_not_sent() {
    let err = awaiting_answer::<i32, _>("a save", |_| Err(spacetimedb_sdk::Error::Disconnected))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("could not send a save"));
}

// ---- The connector and caller with no live connection -----------------------
//
// Everything above exercises pure folding. These drive the `StoreConnector` and
// `ReducerCaller` surfaces on a connector that was never connected, which is the
// state every refusal path starts from and needs no server.

mod unconnected {
    use super::*;
    use crate::models::TaskId;
    use crate::sync::writes::{ReducerCaller, SettledIdentity};
    use crate::sync::{SharedRows, StoreConnector, SubscriptionRequest};
    use std::sync::Arc;

    use super::super::{SdkReducerCaller, SpacetimeSdkConnector};

    fn connector() -> Arc<SpacetimeSdkConnector> {
        Arc::new(SpacetimeSdkConnector::new(
            "dispatch",
            Arc::new(SharedRows::new()),
        ))
    }

    #[tokio::test]
    async fn subscribing_before_connecting_is_refused_by_name() {
        let error = connector()
            .subscribe(&SubscriptionRequest::new("ab12", vec![], "host-1"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("before connecting"), "{error}");
    }

    #[tokio::test]
    async fn a_reported_drop_is_handed_over_once() {
        let connector = connector();
        *connector.dropped.lock().unwrap() = Some("socket closed".into());
        assert_eq!(
            connector.take_drop().await.as_deref(),
            Some("socket closed")
        );
        assert_eq!(connector.take_drop().await, None);
    }

    #[tokio::test]
    async fn disconnecting_forgets_a_pending_drop() {
        let connector = connector();
        *connector.dropped.lock().unwrap() = Some("socket closed".into());
        connector.disconnect().await;
        assert_eq!(connector.take_drop().await, None);
    }

    #[tokio::test]
    async fn connecting_to_nothing_reports_a_failure_not_a_connection() {
        let connector = connector();
        let result = connector.connect("http://127.0.0.1:1", None).await;
        assert!(result.is_err());
        assert!(connector.current().is_none());
    }

    #[tokio::test]
    async fn a_write_with_no_connection_says_nothing_was_queued() {
        let status = Arc::new(SettledIdentity::default());
        let caller = SdkReducerCaller::new(connector(), status);
        let error = caller.delete_task(TaskId(1)).await.unwrap_err();
        let text = error.to_string();
        assert!(text.contains("not connected"), "{text}");
        assert!(text.contains("nothing was queued"), "{text}");
    }

    #[tokio::test]
    async fn a_write_with_no_connection_quotes_the_recorded_outage() {
        let status = Arc::new(SettledIdentity::default());
        status.set_last_error(Some("connection refused".into()));
        let caller = SdkReducerCaller::new(connector(), status);
        let error = caller.delete_task(TaskId(1)).await.unwrap_err();
        assert!(error.to_string().contains("connection refused"), "{error}");
    }

    #[tokio::test]
    async fn a_create_with_no_connection_is_refused_before_anything_is_sent() {
        let caller = SdkReducerCaller::new(connector(), Arc::new(SettledIdentity::default()));
        let error = caller.create_task(task("t")).await.unwrap_err();
        assert!(error.to_string().contains("not connected"), "{error}");
    }

    /// Every call checks for a connection before it builds anything, so a
    /// board with no store refuses each one the same way. Lists the calls the
    /// tests above leave out, so none can skip the check unnoticed.
    #[tokio::test]
    async fn every_write_with_no_connection_is_refused_alike() {
        use crate::models::{EpicId, LearningId};
        use chrono::Utc;

        let caller = SdkReducerCaller::new(connector(), Arc::new(SettledIdentity::default()));
        let refused = |what: &str, error: anyhow::Error| {
            let text = error.to_string();
            assert!(text.contains("not connected"), "{what}: {text}");
        };
        let host = || "host-1".to_string();

        refused(
            "claim_backlog_task",
            caller
                .claim_backlog_task(TaskId(1), host())
                .await
                .unwrap_err(),
        );
        refused(
            "release_backlog_claim",
            caller.release_backlog_claim(TaskId(1)).await.unwrap_err(),
        );
        refused(
            "create_epic",
            caller.create_epic(epic("e")).await.unwrap_err(),
        );
        refused(
            "delete_epic",
            caller.delete_epic(EpicId(1)).await.unwrap_err(),
        );
        refused(
            "batch_delete",
            caller
                .batch_delete(vec![TaskId(1)], vec![EpicId(2)])
                .await
                .unwrap_err(),
        );
        refused(
            "recalculate_epic_status",
            caller.recalculate_epic_status(EpicId(1)).await.unwrap_err(),
        );
        refused(
            "delete_repo_path",
            caller.delete_repo_path("/repo".into()).await.unwrap_err(),
        );
        refused(
            "clear_setting",
            caller
                .clear_setting(host(), "key".into())
                .await
                .unwrap_err(),
        );
        refused(
            "create_learning",
            caller.create_learning(learning("l")).await.unwrap_err(),
        );
        refused(
            "delete_learning",
            caller.delete_learning(LearningId(1)).await.unwrap_err(),
        );
        refused(
            "subagent_start",
            caller
                .subagent_start(TaskId(1), "a".into(), "s".into(), Utc::now())
                .await
                .unwrap_err(),
        );
        refused(
            "subagent_stop",
            caller
                .subagent_stop(TaskId(1), "a".into(), "s".into())
                .await
                .unwrap_err(),
        );
        refused(
            "subagent_clear",
            caller.subagent_clear(TaskId(1)).await.unwrap_err(),
        );
        refused(
            "subagent_clear_and_void_pending_stop",
            caller
                .subagent_clear_and_void_pending_stop(TaskId(1))
                .await
                .unwrap_err(),
        );
        refused(
            "try_record_stop",
            caller
                .try_record_stop(TaskId(1), Utc::now())
                .await
                .unwrap_err(),
        );
        refused(
            "create_repo_group_sub_epic",
            caller
                .create_repo_group_sub_epic(EpicId(1), "t".into(), "alice".into())
                .await
                .unwrap_err(),
        );
        refused(
            "create_managed_role_epic",
            caller
                .create_managed_role_epic(
                    "t".into(),
                    None,
                    "role".into(),
                    "cmd".into(),
                    60,
                    "alice".into(),
                )
                .await
                .unwrap_err(),
        );
        refused(
            "respawn_phoenix_successor",
            caller
                .respawn_phoenix_successor(TaskId(1), task("t"))
                .await
                .unwrap_err(),
        );
        refused(
            "register_host",
            caller
                .register_host(host(), "label".into(), "alice".into())
                .await
                .unwrap_err(),
        );
    }
}
