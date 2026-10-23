//! `apply_learning_verdicts`: one verdict stands per (task, learning), the
//! latest wins, and a verdict needs a retrieval for the same task
//! (`learnings.allium: RateLearningViaMcp`).

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

fn learning(id: i64) -> Learning {
    Learning {
        id,
        kind: "pitfall".into(),
        summary: "a learning".into(),
        detail: None,
        scope: "repo".into(),
        scope_ref: None,
        tags: "[]".into(),
        status: "approved".into(),
        source_task_id: None,
        upvote_count: 0,
        last_upvoted_at: None,
        created_at: "2026-09-19 10:00:00".into(),
        updated_at: "2026-09-19 10:00:00".into(),
        embedding: None,
    }
}

fn retrieval(id: i64, task_id: i64, learning_id: i64) -> LearningRetrieval {
    LearningRetrieval {
        id,
        task_id,
        learning_id,
        source: "prompt_injection".into(),
        retrieved_at: "2026-09-19 10:00:00".into(),
    }
}

fn verdict(learning_id: i64, verdict: &str) -> LearningVerdictInput {
    LearningVerdictInput {
        learning_id,
        verdict: verdict.into(),
    }
}

/// Learning 1, retrieved for tasks 7 and 8.
fn store_with_a_retrieved_learning() -> Store {
    let store = Store::new();
    store
        .call("alice", |ctx| seed_learnings(ctx, vec![learning(1)]))
        .expect("seed_learnings");
    store
        .call("alice", |ctx| {
            seed_learning_retrievals(ctx, vec![retrieval(1, 7, 1), retrieval(2, 8, 1)])
        })
        .expect("seed_learning_retrievals");
    store
}

fn score(store: &Store) -> i64 {
    store
        .db()
        .learnings()
        .id()
        .find(1)
        .expect("the learning")
        .upvote_count
}

fn rate(store: &Store, task_id: i64, verdicts: Vec<LearningVerdictInput>) -> Result<(), String> {
    store.call("alice", |ctx| {
        apply_learning_verdicts(ctx, task_id, verdicts)
    })
}

#[test]
fn a_repeat_of_the_same_verdict_from_one_task_counts_once() {
    let store = store_with_a_retrieved_learning();
    rate(&store, 7, vec![verdict(1, "helped")]).expect("first");
    rate(&store, 7, vec![verdict(1, "helped")]).expect("repeat");
    assert_eq!(score(&store), 1);
}

#[test]
fn a_changed_verdict_from_one_task_replaces_the_earlier_one() {
    let store = store_with_a_retrieved_learning();
    rate(&store, 7, vec![verdict(1, "helped")]).expect("helped");
    rate(&store, 7, vec![verdict(1, "wrong")]).expect("wrong");
    assert_eq!(score(&store), -1, "helped (+1) is replaced by wrong (-1)");
    rate(&store, 7, vec![verdict(1, "helped")]).expect("helped again");
    assert_eq!(score(&store), 1);
    assert_eq!(store.db().learning_verdicts().iter().count(), 1);
}

#[test]
fn two_entries_for_one_learning_in_one_batch_latest_wins() {
    let store = store_with_a_retrieved_learning();
    rate(&store, 7, vec![verdict(1, "helped"), verdict(1, "wrong")]).expect("batch");
    assert_eq!(score(&store), -1);
}

#[test]
fn each_task_gets_its_own_verdict() {
    let store = store_with_a_retrieved_learning();
    rate(&store, 7, vec![verdict(1, "helped")]).expect("task 7");
    rate(&store, 8, vec![verdict(1, "helped")]).expect("task 8");
    assert_eq!(score(&store), 2);
}

#[test]
fn a_verdict_needs_a_retrieval_for_the_same_task() {
    let store = store_with_a_retrieved_learning();
    let refused = rate(&store, 9, vec![verdict(1, "helped")]);
    let message = refused.expect_err("task 9 never saw the learning");
    assert!(message.contains("was not retrieved during task 9"), "{message}");
    assert_eq!(score(&store), 0);
    assert_eq!(store.db().learning_verdicts().iter().count(), 0);
}

#[test]
fn an_unknown_verdict_refuses_the_whole_batch() {
    let store = store_with_a_retrieved_learning();
    let refused = rate(&store, 7, vec![verdict(1, "helped"), verdict(1, "maybe")]);
    assert!(refused.is_err());
    assert_eq!(score(&store), 0, "the helped before it rolled back");
}

#[test]
fn deleting_a_learning_removes_its_verdicts() {
    let store = store_with_a_retrieved_learning();
    rate(&store, 7, vec![verdict(1, "helped")]).expect("rate");
    store
        .call("alice", |ctx| delete_learning(ctx, 1))
        .expect("delete_learning");
    assert_eq!(store.db().learning_verdicts().iter().count(), 0);
}
