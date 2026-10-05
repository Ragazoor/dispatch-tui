//! The real connector and caller with no store behind them.
//!
//! Spec: `docs/specs/sync.allium`'s `AWriteWithNoConnectionIsRefused` and the
//! `StoreConnector` contract (one attempt per call, `take_drop` takes rather
//! than peeks).
//!
//! These run without `spacetime` on `PATH`, so the Coverage job counts them —
//! unlike `tests/memory_caller_conformance.rs`, which skips there. They cover
//! what the transport does when nothing answers, which is the half a live
//! instance cannot reach.

use std::sync::Arc;

use crate::models::TaskId;
use crate::sync::writes::ReducerCaller;
use crate::sync::{
    SdkReducerCaller, SettledIdentity, SharedRows, SpacetimeSdkConnector, StoreConnector,
    SubscriptionRequest,
};

fn unconnected() -> (Arc<SpacetimeSdkConnector>, Arc<SettledIdentity>) {
    let connector = Arc::new(SpacetimeSdkConnector::new(
        "dispatch",
        Arc::new(SharedRows::new()),
    ));
    (connector, Arc::new(SettledIdentity::default()))
}

#[tokio::test]
async fn a_write_with_no_connection_is_refused_and_says_nothing_was_queued() {
    let (connector, status) = unconnected();
    let caller = SdkReducerCaller::new(connector, status);

    let err = caller.delete_task(TaskId(1)).await.unwrap_err().to_string();

    assert!(err.contains("not connected"), "{err}");
    assert!(err.contains("nothing was queued"), "{err}");
}

#[tokio::test]
async fn a_refused_write_names_the_outage_the_session_recorded() {
    let (connector, status) = unconnected();
    status.set_last_error(Some("connection refused".to_string()));
    let caller = SdkReducerCaller::new(connector, status.clone());

    let err = caller.delete_task(TaskId(1)).await.unwrap_err().to_string();

    assert!(err.contains("unreachable (connection refused)"), "{err}");

    status.settle("c0ffee");
    let err = caller.delete_task(TaskId(1)).await.unwrap_err().to_string();
    assert!(
        !err.contains("connection refused"),
        "a settled connection clears the outage: {err}"
    );
}

#[tokio::test]
async fn subscribing_before_connecting_is_an_error_not_a_panic() {
    let (connector, _) = unconnected();

    let err = connector
        .subscribe(&SubscriptionRequest::new("c0ffee", vec![], "host-a"))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("before connecting"), "{err}");
}

#[tokio::test]
async fn an_unconnected_transport_has_no_drop_and_disconnects_quietly() {
    let (connector, _) = unconnected();

    assert_eq!(connector.take_drop().await, None);
    connector.disconnect().await;
    connector.disconnect().await;
    assert_eq!(connector.take_drop().await, None);
}
