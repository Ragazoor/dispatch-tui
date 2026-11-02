use super::*;

/// **Test 1 of Phase 11 (task #4915), against a real server**, as the private
/// flip (task #98691) left it: a board sees the usage events it recorded and
/// no one else's (`own_usage_summary`, workspaces.allium "THE PRIVATE FLIP").
///
/// Another identity records first, through the CLI — a different process,
/// the same convention `a_row_written_elsewhere_arrives_through_the_subscription`
/// uses. This board records second. The store applies the two in order, so
/// once this board's event is recorded, the other one would be in the
/// answer if the procedure served it.
#[test]
fn a_board_sees_the_usage_events_it_recorded_and_no_one_elses() {
    if !spacetime_available_or_skip() {
        return;
    }
    use dispatch_tui::spacetime::bindings;
    use dispatch_tui::sync::{ReducerCaller, SdkReducerCaller, SettledIdentity};

    let instance = Instance::start("module-test");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let rows = Arc::new(SharedRows::new());
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

    runtime.block_on(async {
        let connector = Arc::new(SpacetimeSdkConnector::new(
            instance.database(),
            rows.clone(),
        ));
        let accepted = connector
            .connect(&instance.host(), None)
            .await
            .unwrap_or_else(|e| panic!("connect: {e}"));
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                "host-b",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));
        let caller = SdkReducerCaller::new(connector, Arc::new(SettledIdentity::default()));

        let elsewhere = instance.call(
            "record_usage_event",
            &[
                &usage_event_json("keybinding", "somebody_elses", Some("x"), "human").to_string(),
                "100",
            ],
        );
        assert!(elsewhere.status.success(), "{}", describe(&elsewhere));

        caller
            .record_usage_event(
                bindings::UsageEvent {
                    id: 0,
                    recorded_at: "2026-09-26 10:00:00".into(),
                    category: "keybinding".into(),
                    action: "dispatch_task".into(),
                    detail: Some("d".into()),
                    actor: "human".into(),
                    recorded_by: String::new(),
                },
                100,
            )
            .await
            .unwrap_or_else(|e| panic!("record_usage_event: {e}"))
            .applied()
            .unwrap_or_else(|why| panic!("record_usage_event refused: {why}"));

        // Served on demand by the `own_usage_summary` procedure: the store
        // answers once the reducer has applied, so nothing to wait for.
        let summary = caller
            .own_usage_summary(None, None, None, 50)
            .await
            .unwrap_or_else(|e| panic!("own_usage_summary: {e}"));
        assert_eq!(summary.len(), 1, "only this board's event: {summary:?}");
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
        // The store stamps the caller over whatever is sent.
        "recorded_by": "",
    })
}
