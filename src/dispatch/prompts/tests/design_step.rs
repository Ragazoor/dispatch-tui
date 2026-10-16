use super::*;

#[test]
fn learning_instruction_references_learnings_skill() {
    let text = learning_tools_instruction();
    assert!(
        text.contains("/learnings"),
        "learning instruction should reference the /learnings skill, got: {text}"
    );
}

/// Every MCP tool this line used to name now carries the same WHEN in its
/// own schema, so naming them here restates three descriptions the agent
/// already has — see `ThePromptNamesNoToolMerelyToSayItExists` in
/// `docs/specs/dispatch-prompt.allium`. The skill survives: a skill listing
/// carries a description, not a nudge to invoke it.
///
/// Scanned over the whole trailing block, not just this one line, so the
/// guard also catches a tool name reintroduced into a neighbouring shared
/// line. `dispatch:wrap-up` is the skill, spelled with a hyphen, and does not match
/// the `wrap_up` tool; every registry name is snake_case, so none collides
/// with ordinary prose.
///
/// Derived from `TOOL_NAMES` — the registry `mcp_tools!` generates — not a
/// hand-written list. A hand-written one covered 8 of the 23 tools and
/// would have grown a blind spot with every tool added, which is the same
/// staleness this line was rewritten to escape.
#[test]
fn prompt_trailing_lines_name_no_mcp_tool() {
    for preceding in Preceding::ALL {
        let text = trailing_block(preceding);
        for tool in crate::mcp::handlers::TOOL_NAMES {
            assert!(
                !text.contains(tool),
                "{preceding:?}: the trailing block must not name the {tool} \
tool — its own description carries what the prose would say, got: {text}"
            );
        }
    }
}

/// The epic section states the epic and how to reach siblings, and names no
/// dispatch MCP tool: `list_tasks` has its own schema, so naming it restates it.
/// See `ThePromptNamesNoToolMerelyToSayItExists` in
/// `docs/specs/dispatch-prompt.allium`.
#[test]
fn epic_section_names_no_mcp_tool() {
    let epic = EpicContext {
        epic_id: EpicId(7),
        epic_title: "My Epic".to_string(),
        under_cve_feed: false,
    };
    let text = epic.prompt_section();
    assert!(
        text.contains("#7") && text.contains("My Epic"),
        "got: {text}"
    );
    assert!(
        text.contains("ListAgents") && text.contains("task-<id>"),
        "got: {text}"
    );
    for tool in crate::mcp::handlers::TOOL_NAMES {
        assert!(
            !text.contains(tool),
            "the epic section must not name the {tool} tool, got: {text}"
        );
    }
}

/// `TheResearchPromptGivesItsReasonNotARule`: the research addendum says, in
/// one sentence of reasoning, why the agent stops where it does — it presents
/// its findings interactively, and code changes and wrap-up are the user's call
/// once they have read them. That sentence is the whole constraint (research
/// launches with no permission mode), so it keeps both halves.
#[test]
fn research_prompt_gives_one_reason_covering_code_changes_and_wrap_up() {
    let text = build_research_prompt(
        TaskId(7),
        "Research async runtimes",
        "Compare tokio vs async-std",
        None,
        &PromptContext::default(),
    );
    let reason = text
        .split_terminator(['.', '\n'])
        .map(str::to_lowercase)
        .find(|s| s.contains("code") && s.contains("wrap") && s.contains("user"))
        .unwrap_or_else(|| {
            panic!(
                "one sentence must say that both code changes and wrap-up are the \
user's call, got: {text}"
            )
        });
    assert!(
        reason.contains("change"),
        "the reason must cover code changes, got: {reason:?}"
    );
}

/// The same guarantee's other half: no capitalised prohibitions. An agent
/// given a bare rule follows the letter and loses the reason.
#[test]
fn research_prompt_carries_no_capitalised_prohibition() {
    let text = build_research_prompt(
        TaskId(7),
        "Research async runtimes",
        "Compare tokio vs async-std",
        None,
        &PromptContext::default(),
    );
    assert!(
        !text.contains("NOT"),
        "the research prompt must give its reason, not a capitalised rule, got: {text}"
    );
}

#[test]
fn learning_instruction_omits_deleted_action_skills() {
    let text = learning_tools_instruction();
    for skill in [
        "/codebase-knowledge",
        "/code-conventions",
        "/test-conventions",
        "/pr-workflow",
        "/dispatch-workflow",
        "/troubleshoot",
        "/improvement",
    ] {
        assert!(
            !text.contains(skill),
            "learning instruction should no longer reference deleted skill {skill}, got: {text}"
        );
    }
}

#[test]
fn learning_instruction_in_task_prompts_with_plan() {
    let text = build_prompt(
        TaskId(1),
        "title",
        "desc",
        Some("/path/to/plan.md"),
        None,
        &PromptContext::default(),
    );
    assert!(
        text.contains("/learnings"),
        "build_prompt (with plan) should reference /learnings skill"
    );
}

#[test]
fn build_prompt_with_plan_and_auto_run_skips_confirmation() {
    let ctx = PromptContext {
        auto_run_plan: true,
        ..PromptContext::default()
    };
    let text = build_prompt(
        TaskId(1),
        "title",
        "desc",
        Some("/path/to/plan.md"),
        None,
        &ctx,
    );
    assert!(
        !text.contains("Shall I proceed with implementation?"),
        "auto_run_plan should skip the ask-permission addendum, got: {text}"
    );
    assert!(
        text.contains("/path/to/plan.md"),
        "the plan path should still be referenced, got: {text}"
    );
}

#[test]
fn build_prompt_with_plan_default_still_asks_permission() {
    let text = build_prompt(
        TaskId(1),
        "title",
        "desc",
        Some("/path/to/plan.md"),
        None,
        &PromptContext::default(),
    );
    assert!(
        text.contains("ask the user to confirm"),
        "default (auto_run_plan: false) must keep asking, got: {text}"
    );
    assert!(
        text.contains("Make no changes until they do"),
        "the confirmation must gate changes, not just request a summary, got: {text}"
    );
}

#[test]
fn spec_first_instruction_frames_the_spec_as_an_intermediate_step() {
    let text = spec_first_instruction();
    assert!(
        text.contains("not the end of the task"),
        "spec_first_instruction should make clear writing the spec \
doesn't finish the task, got: {text}"
    );
    assert!(
        text.contains("implement") && text.contains("same session"),
        "spec_first_instruction should say implementation follows in the \
same session, got: {text}"
    );
}

/// `TheDesignStepNamesSkillsNotProcedure`: the design step is ONE sentence
/// naming allium:elicit, allium:tend, allium:propagate, the implementation and
/// allium:weed, in that order. The skills carry their own process; a prompt
/// that restates it can only drift from them.
#[test]
fn spec_first_instruction_names_the_skills_in_order_in_one_sentence() {
    let text = spec_first_instruction();
    const SKILLS: [&str; 4] = [
        "allium:elicit",
        "allium:tend",
        "allium:propagate",
        "allium:weed",
    ];
    let sentence = text
        .split_terminator(['.', '\n'])
        .find(|s| s.contains("allium:elicit"))
        .unwrap_or_else(|| panic!("spec_first_instruction must name allium:elicit, got: {text}"));
    for skill in SKILLS {
        assert!(
            sentence.contains(skill),
            "the sentence naming allium:elicit must also name {skill} — one sentence \
names every skill, got: {sentence:?} in {text}"
        );
    }
    let idx = |needle: &str| sentence.find(needle).expect("named above");
    assert!(
        idx("allium:elicit") < idx("allium:tend")
            && idx("allium:tend") < idx("allium:propagate")
            && idx("allium:propagate") < idx("allium:weed"),
        "the skills must be named in order elicit, tend, propagate, weed, got: {sentence:?}"
    );
    // Test-first is stated by the order: propagate derives the tests before
    // the implementation, and weed follows it.
    let implement = sentence[idx("allium:propagate")..]
        .find("implement")
        .map(|i| i + idx("allium:propagate"))
        .unwrap_or_else(|| {
            panic!("the implementation must sit between propagate and weed, got: {sentence:?}")
        });
    assert!(
        implement < idx("allium:weed"),
        "the implementation must come after propagate and before weed, got: {sentence:?}"
    );
    assert!(
        !text.contains("/brainstorming"),
        "spec_first_instruction must not name the retired /brainstorming skill, got: {text}"
    );
}

/// `TheDesignStepNamesSkillsNotProcedure`: the design step applies only when
/// the task changes behaviour, and the agent judges that. Both design steps
/// open with the same condition, so a research-shaped chore is not told to
/// tend a spec with no subject.
#[test]
fn design_steps_apply_only_when_the_task_changes_behaviour() {
    for (name, text) in [
        ("spec_first_instruction", spec_first_instruction()),
        ("brainstorm_instruction", brainstorm_instruction()),
    ] {
        assert!(
            text.starts_with("If the task changes what the system does, "),
            "{name} must open with the behaviour-change condition, got: {text}"
        );
    }
}

/// It is not a numbered procedure: the five-step list paraphrased each skill
/// it named.
#[test]
fn spec_first_instruction_is_not_a_numbered_procedure() {
    let text = spec_first_instruction();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let numbered = trimmed
            .split_once(". ")
            .is_some_and(|(head, _)| !head.is_empty() && head.chars().all(|c| c.is_ascii_digit()));
        assert!(
            !numbered,
            "spec_first_instruction must not be a numbered list, found {line:?} in: {text}"
        );
    }
}

/// It does not tell the agent to interview the user: who is there to ask
/// differs by launch, and an unattended dispatch may have nobody watching.
/// Whether elicit asks anyone is the skill's judgement.
#[test]
fn spec_first_instruction_does_not_mandate_interviewing_the_user() {
    let text = spec_first_instruction().to_lowercase();
    assert!(
        !text.contains("interview"),
        "spec_first_instruction must not tell the agent to interview the user, got: {text}"
    );
}

/// `/allium-loop` is no longer named as an alternative way to run the
/// propagate/implement/weed steps — its own description says when to reach
/// for it, and naming it here was a second trigger for one skill.
#[test]
fn spec_first_instruction_does_not_name_allium_loop() {
    let text = spec_first_instruction();
    assert!(
        !text.contains("allium-loop"),
        "spec_first_instruction must not name /allium-loop, got: {text}"
    );
}

/// The two clauses both design steps share survive the rewrite: the plan doc
/// is the agent's judgement call, attached via update_tasks, and only when it
/// is worth it.
#[test]
fn spec_first_instruction_keeps_the_plan_doc_a_judgement_call() {
    let text = spec_first_instruction();
    assert!(
        text.contains("judgement call") || text.contains("not a requirement"),
        "spec_first_instruction should mark the plan doc as the agent's call, got: {text}"
    );
    assert!(
        text.contains("docs/plans/") && text.contains("update_tasks"),
        "spec_first_instruction should still say where an optional plan goes \
and how to attach it, got: {text}"
    );
    assert!(
        text.contains("work packages"),
        "the epic-decomposition carve-out must survive, got: {text}"
    );
}

// -----------------------------------------------------------------------
// DesignStepMatchesTheReposSpecs (task #4409)
//
// A repo with no `docs/specs/*.allium` cannot be sent through an
// Allium-first design step, so its prompts name
// `superpowers:brainstorming` instead and drop `allium_instruction`.
// -----------------------------------------------------------------------

/// The fallback names the skill and states why it was chosen. It must not
/// paraphrase brainstorming's own process — the behaviour belongs to the
/// skill the agent loads, and a paraphrase can only drift from it.
#[test]
fn brainstorm_instruction_names_the_skill_and_says_why() {
    let text = brainstorm_instruction();
    assert!(
        text.contains("superpowers:brainstorming"),
        "the fallback should name the brainstorming skill, got: {text}"
    );
    assert!(
        text.contains("no Allium specs"),
        "the fallback should say why it is not the spec-first sequence, got: {text}"
    );
}

#[test]
fn brainstorm_instruction_does_not_restate_the_skill_or_tdd() {
    let text = brainstorm_instruction();
    for token in [
        "allium:elicit",
        "allium:tend",
        "allium:propagate",
        "allium:weed",
        "docs/specs/",
        "/allium-loop",
    ] {
        assert!(
            !text.contains(token),
            "the fallback must not name {token} — there is no spec to work \
from, got: {text}"
        );
    }
    assert!(
        !text.contains("failing test") && !text.contains("TDD"),
        "TDD is in the trailing block either way; the fallback must not \
restate it, got: {text}"
    );
}

/// The two design steps differ only in the design artefact. Everything
/// about what is optional and what is not is identical.
#[test]
fn brainstorm_instruction_shares_the_optional_plan_and_no_stopping_point_clauses() {
    let text = brainstorm_instruction();
    assert!(
        text.contains("judgement call") && text.contains("docs/plans/"),
        "the plan doc should still be offered as the agent's call, got: {text}"
    );
    assert!(
        text.contains("update_tasks"),
        "the fallback should say how an optional plan gets attached, got: {text}"
    );
    assert!(
        text.contains("not the end of the task"),
        "the design step must not read as a stopping point, got: {text}"
    );
    assert!(
        text.contains("work packages"),
        "the epic-decomposition carve-out should match spec_first_instruction, \
got: {text}"
    );
}

#[test]
fn prompt_context_defaults_to_the_spec_first_branch() {
    assert!(
        PromptContext::default().has_allium_specs,
        "an unknown repo must not be silently downgraded to brainstorming"
    );
}

/// The whole point of task #4409: the no-plan design step follows the repo.
#[test]
fn no_plan_addendum_follows_the_repos_specs() {
    let with_specs = build_prompt(TaskId(1), "t", "d", None, None, &PromptContext::default());
    assert!(
        with_specs.contains("allium:elicit") && !with_specs.contains("brainstorming"),
        "a spec-keeping repo gets the spec-first sequence, got: {with_specs}"
    );

    let no_specs = PromptContext {
        has_allium_specs: false,
        ..PromptContext::default()
    };
    let text = build_prompt(TaskId(1), "t", "d", None, None, &no_specs);
    assert!(
        text.contains("superpowers:brainstorming"),
        "a repo with no specs gets the brainstorming design step, got: {text}"
    );
    assert!(
        !text.contains("allium:elicit"),
        "a repo with no specs must not be sent to elicit a spec, got: {text}"
    );
}

#[test]
fn quick_dispatch_design_step_follows_the_repos_specs() {
    let no_specs = PromptContext {
        has_allium_specs: false,
        ..PromptContext::default()
    };
    let text = build_quick_dispatch_prompt(TaskId(1), "t", "d", None, &no_specs);
    assert!(
        text.contains("superpowers:brainstorming") && !text.contains("allium:elicit"),
        "quick dispatch should share the no-spec design step, got: {text}"
    );
    // The rename step is unchanged — only the design step swaps.
    assert!(
        text.contains("call `update_tasks` with a"),
        "quick dispatch should still ask the agent to rename the task, got: {text}"
    );
}

/// The whole `trailing_block` contract as one table: which of the two
/// conditional lines each `Preceding` carries, plus the two that are
/// unconditional. Stated once rather than as four tests each re-asserting
/// the invariants, so a fifth state means adding a row instead of deciding
/// which test owns what.
///
/// The omissions have different reasons, and the two axes are independent.
/// The two spec-less states drop the Allium line because pointing an agent
/// at `docs/specs/` is false in a repo with no such directory. `SpecFirst`
/// drops BOTH the Allium and TDD lines because that sequence already states
/// them as numbered steps, and the trailing wordings are the weaker of the
/// two. The stopping-point line turns on the plan question alone: both plan
/// states carry it whether the repo keeps specs or not, because its wording
/// names no spec directory and no design step preceded it. See
/// NoLineRestatesTheDesignStep in `docs/specs/dispatch-prompt.allium`.
///
/// The CVE states reach the same two omissions from the other direction:
/// their runbook rules test-first OUT rather than in. See
/// CveRemediationSkipsTheDesignStep in the same spec.
#[test]
fn trailing_block_carries_each_line_exactly_where_it_is_not_a_restatement() {
    let rows = [
        (Preceding::Brainstorm, true, false, false),
        (Preceding::PlanWithSpecs, true, true, true),
        (Preceding::PlanWithoutSpecs, true, false, true),
        (Preceding::SpecFirst, false, false, false),
        // The two CVE states drop both lines for the opposite reason —
        // the runbook rules test-first out by name, and a version pin
        // changes no domain behaviour for `docs/specs/` to record. They
        // differ only on the plan axis, which is unchanged here. See
        // CveRemediationSkipsTheDesignStep.
        (Preceding::CveRunbook, false, false, false),
        (Preceding::CveWithPlan, false, false, true),
    ];
    // The table speaks for every state, not for the ones someone
    // remembered. Without this, a state added later takes each predicate's
    // false arm and is asserted nowhere.
    for preceding in Preceding::ALL {
        assert!(
            rows.iter().any(|(p, ..)| *p == preceding),
            "{preceding:?} has no row in the table"
        );
    }
    for (preceding, want_tdd, want_allium, want_stopping_point) in rows {
        let text = trailing_block(preceding);
        assert_eq!(
            text.contains(tdd_instruction()),
            want_tdd,
            "{preceding:?}: tdd presence, got: {text}"
        );
        assert_eq!(
            text.contains(allium_instruction()),
            want_allium,
            "{preceding:?}: allium presence, got: {text}"
        );
        // Both design steps close by saying the design is not the end of
        // the task, so only the plan path — which has no design step —
        // carries this line. Asserted on the substring rather than the
        // whole line, so a reworded restatement is caught too.
        assert_eq!(
            text.contains("not a stopping point"),
            want_stopping_point,
            "{preceding:?}: stopping-point presence, got: {text}"
        );
        if want_stopping_point {
            assert!(
                text.contains(plan_not_a_stopping_point_instruction()),
                "{preceding:?}: should carry the line verbatim, got: {text}"
            );
        }
        // No path may point a spec-less repo at the spec directory.
        if !preceding.keeps_specs() {
            assert!(
                !text.contains("docs/specs/"),
                "{preceding:?}: no line may point at docs/specs/, got: {text}"
            );
        }
        // The two unconditional lines, on every path.
        assert!(
            text.contains("/learnings"),
            "{preceding:?}: the knowledge-base nudge is unconditional, got: {text}"
        );
        assert!(
            text.contains(wrap_up_instruction()),
            "{preceding:?}: wrap-up is unconditional, got: {text}"
        );
    }
}

/// `Preceding::resolve` is the single place the design-step branch is
/// derived, so the two builders cannot disagree about which one they took.
/// All four combinations map to a state of their own.
#[test]
fn preceding_resolves_the_design_branch_from_plan_and_specs() {
    assert_eq!(Preceding::resolve(false, true, false), Preceding::SpecFirst);
    assert_eq!(
        Preceding::resolve(true, true, false),
        Preceding::PlanWithSpecs
    );
    assert_eq!(
        Preceding::resolve(false, false, false),
        Preceding::Brainstorm
    );
    // A plan in a spec-less repo is its own state, not the no-plan one.
    assert_eq!(
        Preceding::resolve(true, false, false),
        Preceding::PlanWithoutSpecs
    );
}

/// The de-duplication is conditional on spec-first actually being present.
/// A repo with no specs gets `brainstorm_instruction`, which names neither
/// TDD nor Allium, so dropping the trailing lines there would leave the
/// prompt with no statement of either.
#[test]
fn brainstorm_path_keeps_the_tdd_line_spec_first_would_have_replaced() {
    let no_specs = PromptContext {
        has_allium_specs: false,
        ..PromptContext::default()
    };
    let text = build_prompt(TaskId(1), "t", "d", None, None, &no_specs);
    assert!(
        text.contains("superpowers:brainstorming"),
        "sanity: this is the brainstorm path, got: {text}"
    );
    assert!(
        text.contains(tdd_instruction()),
        "the brainstorm path has no other statement of TDD, got: {text}"
    );
}

/// A task WITH a plan attached never reaches the spec-first sequence, so
/// both lines stay there too.
#[test]
fn with_plan_path_keeps_tdd_and_allium() {
    let text = build_prompt(
        TaskId(1),
        "t",
        "d",
        Some("/tmp/plan.md"),
        None,
        &PromptContext::default(),
    );
    assert!(
        text.contains(tdd_instruction()),
        "the with-plan path has no other statement of TDD, got: {text}"
    );
    assert!(
        text.contains(allium_instruction()),
        "the with-plan path has no other statement of the tend/weed cycle, got: {text}"
    );
}

/// `allium_instruction` is dropped in BOTH plan states of a spec-less repo,
/// so one repo never gets contradictory prompts depending on whether a plan
/// is attached.
///
/// The stopping-point line is the counter-case, asserted here end to end
/// rather than only on `trailing_block`: this is the one prompt that hands
/// over a plan, names no design step, and keeps no specs, so nothing else
/// in it can tell the agent a plan-only state is unfinished. Gating that
/// line on the spec question dropped it here, reinstating the #4188
/// regression in every repo with no `docs/specs/`.
#[test]
fn with_plan_prompt_omits_the_allium_instruction_when_the_repo_has_no_specs() {
    let no_specs = PromptContext {
        has_allium_specs: false,
        ..PromptContext::default()
    };
    let text = build_prompt(TaskId(1), "t", "d", Some("/p.md"), None, &no_specs);
    assert!(
        !text.contains("source of truth"),
        "with-plan prompt should drop the allium instruction too, got: {text}"
    );
    // The plan addendum itself is untouched.
    assert!(
        text.contains("Plan: /p.md"),
        "the plan addendum is unchanged, got: {text}"
    );
    assert!(
        !text.contains("superpowers:brainstorming"),
        "a task with a plan needs no design step, got: {text}"
    );
    assert!(
        text.contains(plan_not_a_stopping_point_instruction()),
        "a spec-less repo drops the spec directory, not the rule that a \
plan is not a stopping point, got: {text}"
    );
}

/// Review addenda already emit a trimmed trailing block with no allium
/// line and no design step, so they need no branch — and must not grow one.
#[test]
fn review_addenda_are_unaffected_by_the_repos_specs() {
    for tag in [
        TaskTag::Dependabot,
        TaskTag::PrReview,
        TaskTag::ReleasePlease,
    ] {
        let ctx = PromptContext {
            tag: Some(tag),
            has_allium_specs: false,
            ..PromptContext::default()
        };
        let text = build_prompt(TaskId(1), "t", "d", None, None, &ctx);
        assert!(
            !text.contains("brainstorming"),
            "{tag:?}: a review agent gets no design step, got: {text}"
        );
    }
}

/// Brainstorming must not surface in *any* prompt for a repo that keeps
/// Allium specs — there it is the retired design step (task #4366), and the
/// spec-first sequence is the only one named. The no-spec branch that *does*
/// name it (task #4409) is covered by
/// `no_plan_addendum_follows_the_repos_specs`. A single named test cannot be
/// blanket-accepted the way an `INSTA_UPDATE=always` snapshot can.
#[test]
fn no_prompt_variant_for_a_spec_keeping_repo_names_brainstorming() {
    let plain = PromptContext::default();
    assert!(
        plain.has_allium_specs,
        "this test only speaks for the spec-keeping branch"
    );
    let dependabot = PromptContext {
        tag: Some(TaskTag::Dependabot),
        ..PromptContext::default()
    };
    let pr_review = PromptContext {
        tag: Some(TaskTag::PrReview),
        ..PromptContext::default()
    };
    let release_please = PromptContext {
        tag: Some(TaskTag::ReleasePlease),
        ..PromptContext::default()
    };
    let variants = [
        (
            "dispatch no-plan",
            build_prompt(TaskId(1), "t", "d", None, None, &plain),
        ),
        (
            "dispatch with-plan",
            build_prompt(TaskId(1), "t", "d", Some("/p.md"), None, &plain),
        ),
        (
            "quick dispatch",
            build_quick_dispatch_prompt(TaskId(1), "t", "d", None, &plain),
        ),
        (
            "research",
            build_research_prompt(TaskId(1), "t", "d", None, &plain),
        ),
        (
            "dependabot",
            build_prompt(TaskId(1), "t", "d", None, None, &dependabot),
        ),
        (
            "pr review",
            build_prompt(TaskId(1), "t", "d", None, None, &pr_review),
        ),
        (
            "release please",
            build_prompt(TaskId(1), "t", "d", None, None, &release_please),
        ),
    ];
    for (variant, text) in variants {
        assert!(
            !text.contains("brainstorm"),
            "{variant} prompt must not mention brainstorming, got: {text}"
        );
    }
}

#[test]
fn no_plan_addendum_instructs_implementation_for_every_working_tag() {
    // spec_first_instruction is reused verbatim for every tag that
    // reaches it with no plan — Bug, Feature, Chore, Fix, and no tag.
    // Research never reaches this addendum with no plan (DispatchMode
    // diverts it to build_research_prompt instead), so no per-tag branch
    // is needed here.
    for tag in [
        None,
        Some(TaskTag::Bug),
        Some(TaskTag::Feature),
        Some(TaskTag::Chore),
        Some(TaskTag::Fix),
    ] {
        let ctx = PromptContext {
            tag,
            ..PromptContext::default()
        };
        let text = build_prompt(TaskId(1), "Task", "Desc", None, None, &ctx);
        assert!(
            text.contains("not the end of the task"),
            "tag {tag:?}: no-plan prompt should instruct implementation to \
follow plan-attach, got: {text}"
        );
    }
}

/// The bug this guards: prose that lists "attaching a plan" alongside
/// "finishing implementation" as equally valid stopping points reads as
/// permission to stop at a plan for bug/feature/chore/fix tasks (see task
/// #4188). The rule now has its own line, so this asserts on that line —
/// and on the fact that the plan path is where it is emitted, since that
/// is the path #4188 was about.
#[test]
fn a_plan_alone_is_never_a_sufficient_stopping_point() {
    let text = plan_not_a_stopping_point_instruction();
    assert!(
        text.contains("not a stopping point"),
        "the line should say attaching a plan alone is not a stopping \
point, got: {text}"
    );
    assert!(
        text.contains("same session"),
        "the line should require implementation in the same session, got: {text}"
    );
}

#[test]
fn learning_instruction_in_task_prompts_no_plan() {
    let text = build_prompt(
        TaskId(1),
        "title",
        "desc",
        None,
        None,
        &PromptContext::default(),
    );
    assert!(
        text.contains("/learnings"),
        "build_prompt (no plan) should reference /learnings skill"
    );
}

#[test]
fn learning_instruction_in_quick_dispatch_prompt() {
    let text =
        build_quick_dispatch_prompt(TaskId(1), "title", "desc", None, &PromptContext::default());
    assert!(
        text.contains("/learnings"),
        "quick dispatch prompt should reference /learnings skill"
    );
}

#[test]
fn trailing_block_includes_knowledge_base_nudge() {
    let text = trailing_block(Preceding::PlanWithSpecs);
    assert!(
        text.contains("/learnings"),
        "trailing block should point at the /learnings skill, got: {text}"
    );
}

#[test]
fn research_prompt_includes_knowledge_block_when_learnings_injected() {
    // Regression: build_research_prompt used to silently omit the
    // validated-knowledge block that build_prompt/build_quick_dispatch_prompt
    // both include — research tasks get RAG-injected learnings too, so the
    // block must appear here as well.
    let l = seed(20, LearningScope::Repo, 1);
    let ctx = PromptContext {
        learnings: LearningInjections { ranked: vec![&l] },
        ..PromptContext::default()
    };
    let text = build_research_prompt(
        TaskId(7),
        "Research async runtimes",
        "Compare tokio vs async-std",
        None,
        &ctx,
    );
    assert!(
            text.contains("## Validated knowledge for this task"),
            "research prompt should include the knowledge block when learnings are injected, got: {text}"
        );
    assert!(text.contains("[#20 repo, \u{2191}1]"));
}

#[test]
fn research_prompt_content() {
    let text = build_research_prompt(
        TaskId(7),
        "Research async runtimes",
        "Compare tokio vs async-std",
        None,
        &PromptContext::default(),
    );
    assert!(
        text.contains("research agent"),
        "research prompt should identify the agent role"
    );
    assert!(
        text.contains("present") || text.contains("findings"),
        "research prompt should instruct presenting findings"
    );
}
