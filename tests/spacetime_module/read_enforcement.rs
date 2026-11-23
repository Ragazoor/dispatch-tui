//! What a board receives narrows by follows and ownership (`sync.allium:
//! SubscribeOnceIdentityIsSettled`), proved against a live store with a second
//! and a third identity (task #98692). That narrowing is delivery, not access
//! (`docs/plans/2026-11-23-remove-workspaces.md`, R4), but the tables behind
//! the views stay private, so a caller's own SQL cannot reach them.
//!
//! The scenario is `views::seed_scenario`: I follow my epic 1, bob made epics
//! and tasks under it and an unfollowed epic 5. `carol` follows nothing and
//! made nothing. She must receive none of that work, by subscription or by
//! one-off SQL. Positive controls run the same asks as me and bob, so an empty
//! answer for carol cannot be an empty answer for everyone.

use super::views::seed_scenario;
use super::*;

/// Strings that appear only in rows about the scenario's epics and tasks.
const SECRETS: [&str; 8] = [
    "epic 1",
    "epic 2",
    "epic 3",
    "epic 4",
    "epic 5",
    "\"visible\"",
    "\"hidden\"",
    "\"elsewhere\"",
];

/// Every per-caller view, with the same asks the board makes.
const VIEWS: [&str; 15] = [
    "own_epics",
    "own_tasks",
    "own_packages",
    "covered_epics",
    "covered_tasks",
    "visible_agents",
    "visible_dispatch_requests",
    "visible_session_requests",
    "own_unseen_tasks",
    "visible_task_subagents",
    "visible_task_watchers",
    "visible_learning_retrievals",
    "visible_learning_verdicts",
    "visible_poll_owners",
    "visible_work_package_links",
];

/// Tables that hold work and are not public: only the module's owner may read
/// them, and a caller's own SQL must not reach them.
const PRIVATE_TABLES: [&str; 9] = [
    "subscriptions",
    "tasks",
    "epics",
    "agents",
    "task_watchers",
    "task_subagents",
    "work_packages",
    "workspaces",
    "memberships",
];

fn board_queries() -> Vec<String> {
    let mut queries: Vec<String> = VIEWS.iter().map(|v| format!("SELECT * FROM {v}")).collect();
    queries.push("SELECT * FROM visible_learnings".into());
    queries.push("SELECT * FROM own_subscriptions".into());
    queries
}

fn text(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn a_person_who_follows_and_made_nothing_receives_none_of_anothers_work() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let me = instance.identity();
    for who in ["bob", "carol"] {
        let named = instance.call_as(who, "set_display_name", &[&format!("\"{who}\"")]);
        assert!(named.status.success(), "{}", describe(&named));
    }
    let bob = instance.identity_as("bob");
    seed_scenario(&instance, &me, &bob);

    // Controls: bob receives what he made; I, who follow epic 1, receive the
    // tasks under it, bob's included, but not bob's unfollowed epic 5.
    let as_bob = instance.subscribe_as(Some("bob"), &board_queries());
    assert!(text(&as_bob).contains("own_tasks"), "{}", text(&as_bob));
    assert!(text(&as_bob).contains("\"hidden\""), "{}", text(&as_bob));
    assert!(text(&as_bob).contains("\"elsewhere\""), "{}", text(&as_bob));
    let as_me = instance.subscribe_as(None, &board_queries());
    assert!(text(&as_me).contains("covered_tasks"), "{}", text(&as_me));
    assert!(text(&as_me).contains("\"visible\""), "{}", text(&as_me));
    assert!(text(&as_me).contains("\"hidden\""), "{}", text(&as_me));
    assert!(!text(&as_me).contains("\"elsewhere\""), "{}", text(&as_me));

    // Subscription: carol's whole board ask comes back without a trace.
    let as_carol = instance.subscribe_as(Some("carol"), &board_queries());
    assert!(text(&as_carol).contains("own_tasks"), "{}", text(&as_carol));
    let got = text(&as_carol);
    for secret in SECRETS {
        assert!(!got.contains(secret), "carol received {secret}: {got}");
    }

    // One-off SQL: every view is empty for her, and the tables behind them
    // are not hers to read at all.
    for view in VIEWS {
        let out = instance.sql_as("carol", &format!("SELECT * FROM {view}"));
        assert!(out.status.success(), "{view}: {}", describe(&out));
        for secret in SECRETS {
            assert!(
                !text(&out).contains(secret),
                "{view} showed carol {secret}: {}",
                text(&out)
            );
        }
        let rows = serde_json::from_str::<serde_json::Value>(
            text(&out)
                .lines()
                .find(|l| l.trim_start().starts_with('['))
                .unwrap_or("[]"),
        )
        .expect("parse sql output");
        assert!(
            rows[0]["rows"].as_array().is_none_or(|r| r.is_empty()),
            "{view} gave carol rows: {rows}"
        );
    }
    for table in PRIVATE_TABLES {
        let out = instance.sql_as("carol", &format!("SELECT * FROM {table}"));
        assert!(
            !out.status.success(),
            "carol read the private table {table}: {}",
            text(&out)
        );
        for secret in SECRETS {
            assert!(
                !text(&out).contains(secret),
                "{table} refusal leaked {secret}"
            );
        }
    }
}

/// The `subscriptions` table is private: bob follows his own epic, and carol's
/// one-off SQL cannot read who follows what. The view serves each caller only
/// their own follows.
#[test]
fn another_person_cannot_see_who_follows_an_epic() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let me = instance.identity();
    for who in ["bob", "carol"] {
        let named = instance.call_as(who, "set_display_name", &[&format!("\"{who}\"")]);
        assert!(named.status.success(), "{}", describe(&named));
    }
    let bob = instance.identity_as("bob");
    seed_scenario(&instance, &me, &bob);
    call_ok(
        &instance,
        "seed_subscriptions",
        serde_json::json!([{"id": format!("{bob}/2"), "epic_id": 2, "subscriber": bob}]),
    );

    // The table is private, so carol's SQL is refused outright.
    let out = instance.sql_as("carol", "SELECT * FROM subscriptions");
    assert!(
        !out.status.success() && !text(&out).contains(&bob),
        "carol read the subscriptions table: {}",
        text(&out)
    );
    // The view serves each caller only their own follows: bob sees his,
    // carol sees none of them.
    let as_carol = instance.sql_as("carol", "SELECT * FROM own_subscriptions");
    assert!(as_carol.status.success(), "{}", describe(&as_carol));
    assert!(
        !text(&as_carol).contains(&bob),
        "carol read bob's follow: {}",
        text(&as_carol)
    );
    let as_bob = instance.sql_as("bob", "SELECT * FROM own_subscriptions");
    assert!(
        text(&as_bob).contains(&bob),
        "bob cannot see his own follow: {}",
        text(&as_bob)
    );
}
