use super::*;

// -----------------------------------------------------------------------
// The CVE runbook. A CVE task is a targeted dependency fix, so it never
// reaches the design step and never carries the TDD line — see
// `CveRemediationSkipsTheDesignStep` in `docs/specs/dispatch-prompt.allium`.
// -----------------------------------------------------------------------

/// An epic context that says the task hangs under the managed CVE root.
fn cve_epic() -> EpicContext {
    EpicContext {
        epic_id: EpicId(9),
        epic_title: "CVE".to_string(),
        under_cve_feed: true,
    }
}

/// The same shape, for an ordinary epic.
fn plain_epic() -> EpicContext {
    EpicContext {
        epic_id: EpicId(9),
        epic_title: "Dispatch".to_string(),
        under_cve_feed: false,
    }
}

#[test]
fn cve_runbook_states_the_advisory_steps_and_the_backport_trap() {
    let text = cve_runbook();
    assert!(
        text.contains("advisory"),
        "the runbook must send the agent to the advisory, got: {text}"
    );
    assert!(
        text.contains("fixed"),
        "the runbook must name the advisory's fixed set, got: {text}"
    );
    assert!(
        text.contains("backported"),
        "a higher version number is not automatically patched — the runbook \
must say why, got: {text}"
    );
    assert!(
        text.contains("verify command"),
        "the runbook must ask for the repo's verify command, got: {text}"
    );
}

/// The carve-out is test-FIRST, not testing. A patch we wrote ourselves
/// still earns a test.
#[test]
fn cve_runbook_rules_out_test_first_without_ruling_out_tests() {
    let text = cve_runbook();
    assert!(
        text.contains("do not write a failing test first"),
        "the runbook must rule test-first out by name, got: {text}"
    );
    assert!(
        text.contains("our own code"),
        "the runbook must keep the carve-out for a hand-written patch, got: {text}"
    );
}

#[test]
fn cve_task_prompt_replaces_the_design_step() {
    let ctx = PromptContext::default();
    let text = build_prompt(
        TaskId(1),
        "[HIGH] repo: CVE-1",
        "d",
        None,
        Some(&cve_epic()),
        &ctx,
    );
    assert!(
        text.contains("CVE remediation task"),
        "a CVE task must get the runbook, got: {text}"
    );
    for token in [
        "allium:elicit",
        "If the task changes what the system does",
        "Always use TDD",
    ] {
        assert!(
            !text.contains(token),
            "a CVE task must not be sent to {token}, got: {text}"
        );
    }
}

/// The runbook is not the review runbooks: a CVE task authors its own PR
/// and finishes through `dispatch:wrap-up`, so the closing lines are owed to it.
#[test]
fn cve_task_prompt_keeps_wrap_up_and_the_knowledge_base() {
    let ctx = PromptContext::default();
    let text = build_prompt(TaskId(1), "t", "d", None, Some(&cve_epic()), &ctx);
    assert!(
        text.contains("dispatch:wrap-up"),
        "a CVE task finishes through wrap-up, got: {text}"
    );
    assert!(
        text.contains("/learnings"),
        "a CVE task keeps the knowledge-base line, got: {text}"
    );
}

/// The runbook replaces the DESIGN step and nothing else, so an attached
/// plan still wins the addendum — unlike the review arms, which ignore it.
#[test]
fn cve_task_with_a_plan_renders_the_plan_and_still_drops_tdd() {
    let ctx = PromptContext::default();
    let text = build_prompt(
        TaskId(1),
        "t",
        "d",
        Some("docs/plans/x.md"),
        Some(&cve_epic()),
        &ctx,
    );
    assert!(
        text.contains("docs/plans/x.md"),
        "an attached plan must still be read, got: {text}"
    );
    assert!(
        !text.contains("CVE remediation task"),
        "the plan replaces the runbook, got: {text}"
    );
    assert!(
        !text.contains("Always use TDD"),
        "a CVE task never carries the TDD line, plan or no plan, got: {text}"
    );
    assert!(
        text.contains("not the end of the task") || text.contains("stopping point"),
        "a plan path still needs the stopping-point line, got: {text}"
    );
}

/// A CVE-epic task retagged `dependabot` or `pr-review` reviews someone
/// else's PR. That is different work, and the review runbook wins.
#[test]
fn a_review_tag_wins_over_the_cve_epic() {
    for tag in [
        TaskTag::Dependabot,
        TaskTag::PrReview,
        TaskTag::ReleasePlease,
    ] {
        let ctx = PromptContext {
            tag: Some(tag),
            ..PromptContext::default()
        };
        let text = build_prompt(
            TaskId(1),
            "Bump serde from 1.0.0 to 1.0.1",
            "d",
            None,
            Some(&cve_epic()),
            &ctx,
        );
        assert!(
            !text.contains("CVE remediation task"),
            "{tag:?} must keep its own review runbook, got: {text}"
        );
    }
}

#[test]
fn an_ordinary_epic_still_gets_the_design_step() {
    let ctx = PromptContext::default();
    let text = build_prompt(TaskId(1), "t", "d", None, Some(&plain_epic()), &ctx);
    assert!(
        text.contains("allium:elicit"),
        "only a CVE epic diverts the design step, got: {text}"
    );
    assert!(
        !text.contains("CVE remediation task"),
        "an ordinary task must not get the runbook, got: {text}"
    );
}

/// `fix` is a kanban label with no routing meaning. The security feeds set
/// it on every task they create, and that is still not what diverts one.
#[test]
fn the_fix_tag_alone_does_not_reach_the_runbook() {
    let ctx = PromptContext {
        tag: Some(TaskTag::Fix),
        ..PromptContext::default()
    };
    let text = build_prompt(TaskId(1), "t", "d", None, None, &ctx);
    assert!(
        !text.contains("CVE remediation task"),
        "the fix tag is not a routing key, got: {text}"
    );
    assert!(
        text.contains("allium:elicit"),
        "a fix-tagged task keeps the design step, got: {text}"
    );
}

/// Quick dispatch reaches the same branch: a placeholder task created
/// inside the CVE epic is CVE work too.
#[test]
fn quick_dispatch_under_the_cve_epic_takes_the_runbook() {
    let ctx = PromptContext::default();
    let text = build_quick_dispatch_prompt(TaskId(1), "t", "d", Some(&cve_epic()), &ctx);
    assert!(
        text.contains("CVE remediation task"),
        "quick dispatch under the CVE epic must get the runbook, got: {text}"
    );
    assert!(
        !text.contains("Always use TDD"),
        "and must not carry the TDD line, got: {text}"
    );
}

/// The corner a plan re-opens. `DispatchMode::for_task` routes ANY planned
/// task to `Dispatch`, research tag included, so the divert that normally
/// keeps a research task away from this branch does not fire — and a
/// research task is not remediating anything.
#[test]
fn a_research_tag_keeps_the_cve_epic_out_even_with_a_plan() {
    let ctx = PromptContext {
        tag: Some(TaskTag::Research),
        ..PromptContext::default()
    };
    // The ordinary marker differs by plan state: without one the spec-first
    // sequence is the addendum (and states test-first as its own steps);
    // with one the trailing TDD line is what survives.
    for (plan, ordinary_marker) in [
        (None, "allium:elicit"),
        (Some("docs/plans/x.md"), "Always use TDD"),
    ] {
        let text = build_prompt(TaskId(1), "t", "d", plan, Some(&cve_epic()), &ctx);
        assert!(
            !text.contains("CVE remediation task"),
            "a research task must not get the runbook (plan: {plan:?}), got: {text}"
        );
        assert!(
            text.contains(ordinary_marker),
            "and must keep the ordinary trailing block (plan: {plan:?}), got: {text}"
        );
    }
}

/// The guard is one function, so quick dispatch cannot disagree with
/// `build_prompt` about which tags claim a task first — including today,
/// when quick dispatch happens never to set one.
#[test]
fn the_cve_guard_excludes_the_same_three_tags_on_both_paths() {
    for tag in [
        TaskTag::Dependabot,
        TaskTag::PrReview,
        TaskTag::ReleasePlease,
        TaskTag::Research,
    ] {
        assert!(
            !is_cve_task(Some(tag), Some(&cve_epic())),
            "{tag:?} must claim the task before the CVE branch"
        );
    }
    for tag in [TaskTag::Bug, TaskTag::Feature, TaskTag::Chore, TaskTag::Fix] {
        assert!(
            is_cve_task(Some(tag), Some(&cve_epic())),
            "{tag:?} is a kanban label and must not block the CVE branch"
        );
    }
    assert!(
        is_cve_task(None, Some(&cve_epic())),
        "an untagged task under the CVE epic takes the branch"
    );
    assert!(
        !is_cve_task(None, None),
        "a task with no epic is not CVE work"
    );
}

#[test]
fn resolve_maps_the_cve_states_independently_of_the_repos_specs() {
    assert_eq!(Preceding::resolve(false, true, true), Preceding::CveRunbook);
    assert_eq!(
        Preceding::resolve(false, false, true),
        Preceding::CveRunbook
    );
    assert_eq!(Preceding::resolve(true, true, true), Preceding::CveWithPlan);
    assert_eq!(
        Preceding::resolve(true, false, true),
        Preceding::CveWithPlan
    );
    // And the four non-CVE answers are unchanged.
    assert_eq!(Preceding::resolve(false, true, false), Preceding::SpecFirst);
    assert_eq!(
        Preceding::resolve(false, false, false),
        Preceding::Brainstorm
    );
    assert_eq!(
        Preceding::resolve(true, true, false),
        Preceding::PlanWithSpecs
    );
    assert_eq!(
        Preceding::resolve(true, false, false),
        Preceding::PlanWithoutSpecs
    );
}
