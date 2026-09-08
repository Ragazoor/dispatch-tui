#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::prompts::{
    build_prompt, build_quick_dispatch_prompt, build_research_prompt, EpicContext, PromptContext,
};
use crate::models::{EpicId, TaskId, TaskTag};

fn fixture_epic() -> EpicContext {
    EpicContext {
        epic_id: EpicId(7),
        epic_title: "Auth overhaul".to_string(),
    }
}

#[test]
fn snapshot_dispatch_prompt_no_plan() {
    let prompt = build_prompt(
        TaskId(42),
        "Fix the authentication bug",
        "Users cannot log in after the password hash migration",
        None,
        None,
        &PromptContext::default(),
    );
    insta::assert_snapshot!(prompt);
}

#[test]
fn snapshot_dispatch_prompt_with_plan() {
    let prompt = build_prompt(
        TaskId(42),
        "Fix the authentication bug",
        "Users cannot log in after the password hash migration",
        Some("/home/user/repo/docs/plans/fix-auth.md"),
        None,
        &PromptContext::default(),
    );
    insta::assert_snapshot!(prompt);
}

#[test]
fn snapshot_dispatch_prompt_with_plan_auto_run() {
    let ctx = PromptContext {
        auto_run_plan: true,
        ..PromptContext::default()
    };
    let prompt = build_prompt(
        TaskId(42),
        "Fix the authentication bug",
        "Users cannot log in after the password hash migration",
        Some("/home/user/repo/docs/plans/fix-auth.md"),
        None,
        &ctx,
    );
    insta::assert_snapshot!(prompt);
}

#[test]
fn snapshot_dispatch_prompt_dependabot() {
    // With a pr_url AND from_feed, which is the production shape: only a feed
    // creates a dependabot task, and it sets `url` at insert time for every one
    // (AReviewTaggedFeedItemNamesItsPr in docs/specs/feeds.allium requires it).
    // So this snapshot carries the author bullet; the hand-created variant that
    // does not is asserted in prompts.rs's own tests rather than snapshotted.
    let ctx = PromptContext {
        tag: Some(TaskTag::Dependabot),
        pr_url: Some("https://github.com/example/repo/pull/42"),
        from_feed: true,
        ..PromptContext::default()
    };
    let prompt = build_prompt(
        TaskId(42),
        "Bump serde from 1.0.195 to 1.0.197",
        "https://github.com/example/repo/pull/42",
        None,
        None,
        &ctx,
    );
    insta::assert_snapshot!(prompt);
}

#[test]
fn snapshot_dispatch_prompt_with_epic() {
    let epic = fixture_epic();
    let prompt = build_prompt(
        TaskId(42),
        "Fix the authentication bug",
        "Users cannot log in after the password hash migration",
        None,
        Some(&epic),
        &PromptContext::default(),
    );
    insta::assert_snapshot!(prompt);
}

#[test]
fn snapshot_dispatch_prompt_with_plan_and_epic() {
    let epic = fixture_epic();
    let prompt = build_prompt(
        TaskId(42),
        "Fix the authentication bug",
        "Users cannot log in after the password hash migration",
        Some("/home/user/repo/docs/plans/fix-auth.md"),
        Some(&epic),
        &PromptContext::default(),
    );
    insta::assert_snapshot!(prompt);
}

#[test]
fn snapshot_quick_dispatch_prompt() {
    let prompt = build_quick_dispatch_prompt(
        TaskId(42),
        "Quick task",
        "",
        None,
        &PromptContext::default(),
    );
    insta::assert_snapshot!(prompt);
}

#[test]
fn snapshot_research_prompt() {
    let prompt = build_research_prompt(
        TaskId(42),
        "Research async runtimes",
        "Compare tokio vs async-std for our use case",
        None,
        &PromptContext::default(),
    );
    insta::assert_snapshot!(prompt);
}

#[test]
fn snapshot_dispatch_prompt_pr_review() {
    let ctx = PromptContext {
        tag: Some(TaskTag::PrReview),
        ..PromptContext::default()
    };
    let prompt = build_prompt(
        TaskId(42),
        "Review PR: Add new login flow",
        "https://github.com/example/repo/pull/99",
        None,
        None,
        &ctx,
    );
    insta::assert_snapshot!(prompt);
}
