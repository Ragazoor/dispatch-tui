use super::*;

/// **Test 1 of Phase 11 (task #4915), against a real server.** A usage event
/// recorded on one host's board is visible from another's.
///
/// Host A is the `record_usage_event` reducer call — a different process,
/// the same convention `a_row_written_elsewhere_arrives_through_the_subscription`
/// uses. Host B is this test's own `SharedRows`, subscribed with nothing
/// followed: `usage_events` needs no `epic_id` to see a row, because it is
/// unconditionally subscribed, the same as `learnings`.
#[test]
fn a_usage_event_recorded_elsewhere_is_visible_from_here() {
    if !spacetime_available_or_skip() {
        return;
    }
    use dispatch_tui::store::UsageQuery;

    let instance = Instance::start("module-test");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let rows = Arc::new(SharedRows::new());
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

    runtime.block_on(async {
        let connector = SpacetimeSdkConnector::new(instance.database(), rows.clone());
        let accepted = connector
            .connect(&instance.host(), None)
            .await
            .unwrap_or_else(|e| panic!("connect: {e}"));
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                vec![],
                "host-b",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));

        assert!(
            rows.usage_summary(&UsageQuery::default()).is_empty(),
            "nothing has been written yet, so nothing may have arrived"
        );

        let mut woken = rows.changed();
        woken.mark_unchanged();

        // Host A records the event directly through the reducer.
        let recorded = instance.call(
            "record_usage_event",
            &[
                &usage_event_json("keybinding", "dispatch_task", Some("d"), "human").to_string(),
                "100",
            ],
        );
        assert!(recorded.status.success(), "{}", describe(&recorded));

        woken
            .changed()
            .await
            .expect("the subscription must deliver the usage event");

        let summary = rows.usage_summary(&UsageQuery::default());
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].action, "dispatch_task");
        assert_eq!(summary[0].count, 1);
    });
}

/// A `usage_events` row, JSON-encoded for `record_usage_event`'s CLI call.
fn usage_event_json(
    category: &str,
    action: &str,
    detail: Option<&str>,
    actor: &str,
) -> serde_json::Value {
    serde_json::json!({
        "id": 0,
        "recorded_at": "2026-09-26 10:00:00",
        "category": category,
        "action": action,
        "detail": match detail {
            Some(d) => serde_json::json!({"some": d}),
            None => serde_json::json!({"none": []}),
        },
        "actor": actor,
    })
}
