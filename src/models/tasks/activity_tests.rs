use super::*;
use chrono::{Duration, Utc};

fn at(min_ago: i64, now: chrono::DateTime<Utc>) -> chrono::DateTime<Utc> {
    now - Duration::minutes(min_ago)
}

#[test]
fn no_events_classifies_stale() {
    let now = Utc::now();
    assert_eq!(
        classify_agent_activity(None, None, 0, now),
        AgentActivity::Stale
    );
}

#[test]
fn recent_pre_tool_use_classifies_active() {
    let now = Utc::now();
    assert_eq!(
        classify_agent_activity(Some(at(1, now)), None, 0, now),
        AgentActivity::Active
    );
}

#[test]
fn old_pre_tool_use_classifies_stale() {
    let now = Utc::now();
    let past = now - ACTIVE_THRESHOLD - Duration::seconds(1);
    assert_eq!(
        classify_agent_activity(Some(past), None, 0, now),
        AgentActivity::Stale
    );
}

#[test]
fn notification_after_pre_tool_use_classifies_waiting() {
    let now = Utc::now();
    assert_eq!(
        classify_agent_activity(Some(at(5, now)), Some(at(1, now)), 0, now),
        AgentActivity::Waiting
    );
}

#[test]
fn pre_tool_use_after_notification_classifies_active() {
    let now = Utc::now();
    assert_eq!(
        classify_agent_activity(Some(at(1, now)), Some(at(5, now)), 0, now),
        AgentActivity::Active
    );
}

#[test]
fn notification_only_classifies_waiting() {
    let now = Utc::now();
    assert_eq!(
        classify_agent_activity(None, Some(at(1, now)), 0, now),
        AgentActivity::Waiting
    );
}

#[test]
fn boundary_exactly_at_threshold_classifies_active() {
    let now = Utc::now();
    let exactly = now - ACTIVE_THRESHOLD;
    assert_eq!(
        classify_agent_activity(Some(exactly), None, 0, now),
        AgentActivity::Active
    );
}

#[test]
fn just_past_threshold_classifies_stale() {
    let now = Utc::now();
    let past = now - ACTIVE_THRESHOLD - Duration::seconds(1);
    assert_eq!(
        classify_agent_activity(Some(past), None, 0, now),
        AgentActivity::Stale
    );
}

#[test]
fn live_subagents_beat_staleness() {
    let now = Utc::now();
    let long_ago = at(60, now);
    assert_eq!(
        classify_agent_activity(Some(long_ago), None, 0, now),
        AgentActivity::Stale,
        "baseline: no subagents and a cold timestamp is stale"
    );
    assert_eq!(
        classify_agent_activity(Some(long_ago), None, 3, now),
        AgentActivity::Active,
        "live subagents keep the agent active past the threshold"
    );
}

#[test]
fn live_subagents_lose_to_needs_input() {
    let now = Utc::now();
    assert_eq!(
        classify_agent_activity(Some(at(30, now)), Some(at(1, now)), 3, now),
        AgentActivity::Waiting,
        "a permission prompt still needs a human even while subagents run"
    );
}

#[test]
fn live_subagents_with_no_timestamps_at_all_is_active() {
    let now = Utc::now();
    assert_eq!(
        classify_agent_activity(None, None, 1, now),
        AgentActivity::Active
    );
}
