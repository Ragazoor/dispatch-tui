use super::*;
use crate::models::{LearningKind, LearningScope};

#[test]
fn pr_rebase_preamble_targets_pr_branch_on_demand() {
    let text = pr_rebase_preamble("renovate/serde-1.x");
    assert!(
        text.contains("git fetch origin renovate/serde-1.x"),
        "should fetch the PR branch, got: {text}"
    );
    assert!(
        text.contains("git rebase origin/renovate/serde-1.x"),
        "should rebase onto origin/<pr-branch>, got: {text}"
    );
    assert!(
        !text.contains("rebase main"),
        "must not rebase onto the base branch, got: {text}"
    );
}

#[test]
fn reused_preamble_targets_a_local_start_point() {
    let sp = StartPoint::Local {
        base: "develop".to_string(),
    };
    let text = reused_rebase_preamble(&sp);
    assert!(text.contains("git fetch origin develop"), "got: {text}");
    assert!(text.contains("git rebase develop"), "got: {text}");
    assert!(
        !text.contains("git rebase origin/develop"),
        "must not drag a local-based branch back onto origin: {text}"
    );
    assert!(
        text.contains("git status"),
        "tells the agent to inspect first"
    );
    assert!(!text.contains("main"), "no literal main: {text}");
}

#[test]
fn reused_preamble_targets_a_remote_start_point() {
    let sp = StartPoint::Remote {
        base: "develop".to_string(),
    };
    let text = reused_rebase_preamble(&sp);
    assert!(text.contains("git fetch origin develop"), "got: {text}");
    assert!(text.contains("git rebase origin/develop"), "got: {text}");
}

#[test]
fn select_preamble_is_empty_for_a_fresh_worktree() {
    let sp = StartPoint::Remote {
        base: "main".to_string(),
    };
    assert_eq!(select_preamble(None, Some(&sp), false), "");
}

#[test]
fn select_preamble_uses_reuse_wording_for_a_reused_worktree() {
    let sp = StartPoint::Local {
        base: "main".to_string(),
    };
    let text = select_preamble(None, Some(&sp), true);
    assert!(
        text.contains("reused from a previous attempt"),
        "got: {text}"
    );
    assert!(
        text.contains("git rebase main"),
        "mirrors the start point: {text}"
    );
}

#[test]
fn select_preamble_prefers_the_pr_branch_regardless_of_reuse() {
    let sp = StartPoint::Remote {
        base: "renovate/serde-1.x".to_string(),
    };
    for reused in [true, false] {
        let text = select_preamble(Some("renovate/serde-1.x"), Some(&sp), reused);
        assert!(
            text.contains("git rebase origin/renovate/serde-1.x"),
            "reused={reused}, got: {text}"
        );
        assert!(
            !text.contains("reused from a previous attempt"),
            "reused={reused}"
        );
    }
}

#[test]
fn prompt_head_carries_the_warning_even_with_no_preamble() {
    // The fresh + no-origin-ref case: nothing to rebase onto, but the agent
    // must still be told its base is local-only.
    let head = compose_prompt_head("", Some("origin has no branch main"));
    assert!(
        head.contains("Note: origin has no branch main"),
        "got: {head}"
    );
    assert!(!head.starts_with('\n'), "no leading blank line: {head:?}");
}

#[test]
fn prompt_head_is_empty_when_there_is_nothing_to_say() {
    assert_eq!(compose_prompt_head("", None), "");
}

#[test]
fn prompt_head_combines_preamble_and_warning() {
    let head = compose_prompt_head("REBASE", Some("stale"));
    assert!(head.starts_with("REBASE"), "got: {head}");
    assert!(head.contains("Note: stale"), "got: {head}");
    assert!(head.ends_with("\n\n"), "separates from the body: {head:?}");
}

#[test]
fn fresh_worktree_with_no_origin_ref_gets_the_note_and_no_preamble() {
    // The decoupled case: nothing to rebase onto, but the agent must still be
    // told its base is local-only. An earlier design attached the warning to
    // the preamble, which silently dropped it on exactly this row.
    let sp = StartPoint::Local {
        base: "main".to_string(),
    };
    let preamble = select_preamble(None, Some(&sp), false);
    let head = compose_prompt_head(&preamble, Some("origin has no branch main"));

    assert!(
        preamble.is_empty(),
        "fresh worktree emits no preamble: {preamble:?}"
    );
    assert!(
        head.contains("Note: origin has no branch main"),
        "got: {head}"
    );
    assert!(
        !head.contains("git rebase"),
        "nothing to rebase onto: {head}"
    );
    assert!(
        !head.contains("reused from a previous attempt"),
        "got: {head}"
    );
}

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
/// line. `/wrap-up` is the skill, spelled with a hyphen, and does not match
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
/// is the agent's judgement call, attached via update_task, and only when it
/// is worth it.
#[test]
fn spec_first_instruction_keeps_the_plan_doc_a_judgement_call() {
    let text = spec_first_instruction();
    assert!(
        text.contains("judgement call") || text.contains("not a requirement"),
        "spec_first_instruction should mark the plan doc as the agent's call, got: {text}"
    );
    assert!(
        text.contains("docs/plans/") && text.contains("update_task"),
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
        text.contains("update_task"),
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
        text.contains("call `update_task` with a"),
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
    for tag in [TaskTag::Dependabot, TaskTag::PrReview] {
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

fn seed(id: i64, scope: LearningScope, count: i64) -> Learning {
    use crate::models::{LearningId, LearningStatus};
    use chrono::{TimeZone, Utc};
    Learning {
        id: LearningId(id),
        kind: LearningKind::Pitfall,
        summary: format!("learning {id}"),
        detail: None,
        scope,
        scope_ref: match scope {
            LearningScope::User => None,
            _ => Some("ref".into()),
        },
        tags: vec![],
        status: LearningStatus::Approved,
        source_task_id: None,
        upvote_count: count,
        last_upvoted_at: None,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        updated_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    }
}

#[test]
fn render_validated_knowledge_block_omits_when_empty() {
    assert_eq!(render_validated_knowledge_block(&[]), String::new());
}

#[test]
fn render_validated_knowledge_block_formats_entries() {
    let l = seed(7, LearningScope::Epic, 3);
    let out = render_validated_knowledge_block(&[&l]);
    assert!(out.contains("## Validated knowledge for this task"));
    assert!(out.contains("[#7 epic, \u{2191}3]"));
    assert!(out.contains("learning 7"));
    assert!(
        out.contains("rate_learning"),
        "validated-knowledge block should instruct rate_learning, got: {out}"
    );
    assert!(
        !out.contains("learning_verdicts"),
        "validated-knowledge block should no longer reference wrap_up verdicts, got: {out}"
    );
}

#[test]
fn build_prompt_default_injections_unchanged() {
    // Regression: when no learnings are injected the prompt must not gain
    // any leading whitespace or knowledge-block headers.
    let text = build_prompt(
        TaskId(1),
        "title",
        "desc",
        None,
        None,
        &PromptContext::default(),
    );
    assert!(text.starts_with("Your task is:"));
    assert!(!text.contains("Validated knowledge for this task"));
}

#[test]
fn build_prompt_with_injections_includes_knowledge_block() {
    let procedural_l = {
        let mut l = seed(10, LearningScope::User, 0);
        l.kind = LearningKind::Procedural;
        l.detail = Some("Always run tests before committing.".into());
        l
    };
    let convention_l = seed(11, LearningScope::Repo, 2);
    let injections = LearningInjections {
        ranked: vec![&procedural_l, &convention_l],
    };
    let ctx = PromptContext {
        learnings: injections,
        ..PromptContext::default()
    };
    let text = build_prompt(TaskId(1), "title", "desc", None, None, &ctx);
    // Procedural learnings no longer appear as a verbatim prefix — prompt
    // always starts with the task block.
    assert!(text.starts_with("Your task is:"));
    assert!(text.contains("## Validated knowledge for this task"));
    // Both learnings appear in the validated-knowledge block.
    assert!(text.contains("[#10 user, \u{2191}0]"));
    assert!(text.contains("[#11 repo, \u{2191}2]"));
}

#[test]
fn build_quick_dispatch_prompt_default_injections_unchanged() {
    let text =
        build_quick_dispatch_prompt(TaskId(1), "title", "desc", None, &PromptContext::default());
    assert!(text.starts_with("You are working interactively with the user."));
    assert!(!text.contains("Validated knowledge for this task"));
}

#[test]
fn build_prompt_with_dependabot_tag_includes_review_section() {
    let ctx = PromptContext {
        tag: Some(TaskTag::Dependabot),
        ..PromptContext::default()
    };
    let text = build_prompt(
        TaskId(42),
        "Bump serde from 1.0.0 to 1.0.1",
        "https://github.com/example/repo/pull/7",
        None,
        None,
        &ctx,
    );

    assert!(text.contains("Dependabot PR review"), "missing role line");
    // The shared steps, which every route reaches.
    assert!(text.contains("gh pr view"));
    assert!(text.contains("gh pr diff"));
    assert!(text.contains("gh pr checks"));
    // No pr_url on this context, so the find-it-yourself step survives.
    assert!(text.contains("update_task(task_id=42, url="));
    assert!(text.contains("url_type=\"pr\""));
    assert!(text.contains("needs_input"));
    // 1.0.0 -> 1.0.1 is a patch, so this prompt takes the merge route and
    // states the verdict rather than asking for it.
    assert!(
        text.contains("Bump: patch — serde 1.0.0 → 1.0.1"),
        "the harness must state the bump, got: {text}"
    );
    assert!(text.contains("gh pr review"));
    assert!(text.contains("--approve"));
    assert!(text.contains("gh pr merge"));
    assert!(text.contains("--squash --auto"));
    // The standard trailing wrap-up instruction must not be present.
    assert!(
        !text.contains("use the /wrap-up skill"),
        "dependabot prompt must omit the standard wrap-up instruction"
    );
    // No TDD / allium — this agent doesn't edit code.
    assert!(
        !text.contains("Always use TDD"),
        "dependabot prompt must omit the TDD instruction"
    );
    // The standard spec-first addendum must be replaced.
    assert!(
        !text.contains("allium:elicit"),
        "dependabot prompt must omit the spec-first design addendum"
    );
}

#[test]
fn build_prompt_without_dependabot_tag_omits_review_section() {
    let text = build_prompt(
        TaskId(1),
        "title",
        "desc",
        None,
        None,
        &PromptContext::default(),
    );
    assert!(!text.contains("Dependabot PR review"));
    assert!(!text.contains("gh pr merge"));
}

#[test]
fn build_prompt_with_pr_review_tag_includes_review_commands() {
    let ctx = PromptContext {
        tag: Some(TaskTag::PrReview),
        ..PromptContext::default()
    };
    let text = build_prompt(
        TaskId(42),
        "Review PR: Add new login flow",
        "https://github.com/example/repo/pull/99",
        None,
        None,
        &ctx,
    );

    assert!(
        text.contains("/code-review"),
        "pr-review prompt must name the review command, got: {text}"
    );
    // The diff-size branch is gone: the command it routes to takes a PR
    // target and an effort level of its own, so measuring the diff was a
    // proxy for a choice that command already makes. See
    // AReviewRunbookCarriesOnlyTheBranchThatApplies in dispatch-prompt.allium.
    assert!(
        !text.contains("wc -l"),
        "pr-review prompt must not measure the diff, got: {text}"
    );
    assert!(
        !text.contains("/review-pr"),
        "pr-review prompt must name one review command, not two, got: {text}"
    );
}

/// The other half of `build_prompt_with_pr_review_tag_includes_review_commands`,
/// and the counterpart to `build_prompt_without_dependabot_tag_omits_review_section`:
/// `RenderPrReviewRunbook` is gated on the tag, so a task without it must
/// reach none of the runbook. Without this, a change that rendered the PR
/// review addendum unconditionally would pass every existing pr-review test
/// — each of them sets the tag.
#[test]
fn build_prompt_without_pr_review_tag_omits_the_review_runbook() {
    let text = build_prompt(
        TaskId(1),
        "title",
        "desc",
        None,
        None,
        &PromptContext::default(),
    );
    // `/code-review` is step 2 of prompts/pr-review.md, so its absence
    // already implies the whole addendum's — no second assertion needed.
    assert!(
        !text.contains("/code-review"),
        "an untagged prompt must not carry the pr-review runbook, got: {text}"
    );
}

#[test]
fn build_prompt_with_pr_review_tag_omits_the_spec_first_design_addendum() {
    let ctx = PromptContext {
        tag: Some(TaskTag::PrReview),
        ..PromptContext::default()
    };
    let text = build_prompt(
        TaskId(42),
        "Review PR: Add new login flow",
        "https://github.com/example/repo/pull/99",
        None,
        None,
        &ctx,
    );

    assert!(
        !text.contains("allium:elicit"),
        "pr-review prompt must NOT contain the spec-first design addendum"
    );
    assert!(
        !text.contains("implementation plan"),
        "pr-review prompt must NOT mention implementation plan"
    );
    assert!(
        !text.contains("docs/plans/"),
        "pr-review prompt must NOT reference docs/plans/"
    );
}

#[test]
fn build_prompt_with_pr_review_tag_omits_tdd_and_allium_instructions() {
    let ctx = PromptContext {
        tag: Some(TaskTag::PrReview),
        ..PromptContext::default()
    };
    let text = build_prompt(
        TaskId(42),
        "Review PR: Add new login flow",
        "https://github.com/example/repo/pull/99",
        None,
        None,
        &ctx,
    );

    assert!(
        !text.contains("Always use TDD"),
        "pr-review prompt must NOT contain TDD instruction"
    );
    assert!(
        !text.contains("Allium specs"),
        "pr-review prompt must NOT contain allium instruction"
    );
}

#[test]
fn build_prompt_with_pr_review_tag_omits_wrap_up() {
    let ctx = PromptContext {
        tag: Some(TaskTag::PrReview),
        ..PromptContext::default()
    };
    let text = build_prompt(
        TaskId(42),
        "Review PR: Add new login flow",
        "https://github.com/example/repo/pull/99",
        None,
        None,
        &ctx,
    );

    assert!(
        text.contains("do not write a plan or change code"),
        "pr-review prompt must still state its role, got: {text}"
    );
    assert!(
        !text.contains("use the /wrap-up skill"),
        "pr-review prompt must omit the standard wrap-up instruction"
    );
}

/// The rule moved from the prompt to the tool. `wrap_up` refuses a
/// review-tagged task outright (`ReviewTasksAreNotWrappedUp` in
/// `docs/specs/mcp-task-tools.allium`), so asking for it in prose is a
/// rule stated where it cannot be enforced. Counted at zero rather than
/// merely "not the old sentence": a reworded prohibition is the same
/// prompt work coming back under another name.
///
/// What the runbooks keep is the positive half — each terminal names the
/// end state that makes wrap-up unnecessary, which is what an agent
/// standing at that terminal actually needs.
#[test]
fn review_runbooks_no_longer_forbid_wrap_up() {
    for (label, tag, terminal_states) in [
        (
            "dependabot",
            TaskTag::Dependabot,
            ["auto-cleaned on merge", "wait for the user's reply"],
        ),
        (
            "pr-review",
            TaskTag::PrReview,
            ["wait for the user's instructions", "here to review the PR"],
        ),
    ] {
        let ctx = PromptContext {
            tag: Some(tag),
            ..PromptContext::default()
        };
        let text = build_prompt(
            TaskId(42),
            "Bump serde from 1.0.0 to 1.0.1",
            "https://x/pull/9",
            None,
            None,
            &ctx,
        );
        assert_eq!(
            text.matches("/wrap-up").count(),
            0,
            "{label}: wrap_up refuses a review task itself, so the prompt \
must not ask, got: {text}"
        );
        for state in terminal_states {
            assert!(
                text.contains(state),
                "{label}: the terminal branches must still name the end \
state that makes wrap-up unnecessary, missing {state:?}, got: {text}"
            );
        }
    }
}

/// The point of classifying in the harness: an agent is handed its own
/// branch, not a table it has to walk. Each route is checked for what it
/// must carry AND for the branches it must not — a leftover branch is
/// exactly the decision table this replaced.
#[test]
fn a_dependabot_prompt_carries_only_the_branch_its_bump_takes() {
    struct Route {
        label: &'static str,
        title: &'static str,
        body: &'static str,
        present: &'static [&'static str],
        absent: &'static [&'static str],
    }
    let cases = [
        Route {
            label: "patch",
            title: "#25 Bump dbt-common from 1.37.2 to 1.37.3 in /venvs/dbt",
            body: "",
            present: &["Bump: patch — dbt-common 1.37.2 → 1.37.3", "gh pr merge"],
            absent: &["CHANGELOG", "BREAKING", "gh pr comment", "cannot be routed"],
        },
        Route {
            label: "minor",
            title: "#29 Bump requests from 2.32.4 to 2.33.0 in /venvs/basic",
            body: "",
            present: &["Bump: minor — requests", "changelog", "gh pr merge"],
            absent: &["gh pr comment", "cannot be routed"],
        },
        Route {
            label: "major",
            title: "#47 fix(deps): update dependency deepdiff to v9",
            body: "",
            present: &[
                "Bump: major — deepdiff → v9",
                "gh pr comment",
                "never merge",
            ],
            // The merge terminal is unreachable from here, so the branch
            // that says "never merge a major bump yourself" must not be
            // followed two lines later by the commands to do exactly that.
            absent: &["gh pr merge", "CHANGELOG", "cannot be routed"],
        },
        Route {
            label: "grouped non-major",
            title: "#79 fix(deps): update python (non-major)",
            body: "",
            present: &[
                "Bump: non-major group — python",
                "cannot be routed",
                "several packages",
            ],
            absent: &["gh pr merge", "gh pr comment", "CHANGELOG"],
        },
        Route {
            label: "digest",
            title: "#500 fix(deps): update postgres:18 docker digest to 4ef4dbc",
            body: "| postgres:18 | final | digest | `34f47c4` → `4ef4dbc` |",
            // It says WHY there is nothing to read rather than reusing the
            // unclassified wording — the tag did not move, so no changelog
            // exists that would clear it.
            present: &[
                "Bump: digest re-pin — postgres:18 34f47c4 → 4ef4dbc",
                "no changelog",
            ],
            absent: &[
                "gh pr merge",
                "gh pr comment",
                "CHANGELOG",
                "kind could not be read",
            ],
        },
        Route {
            label: "unclassifiable",
            title: "#3 chore: tidy the release workflow",
            body: "",
            present: &["kind could not be read", "cannot be routed"],
            // An unclassified bump is usually a SINGLE package. Telling it
            // the group reason states something about the PR that is not
            // true, which is exactly what the ask branch must not do.
            absent: &[
                "gh pr merge",
                "gh pr comment",
                "CHANGELOG",
                "several packages",
                "grouped",
            ],
        },
    ];

    for case in cases {
        let ctx = PromptContext {
            tag: Some(TaskTag::Dependabot),
            ..PromptContext::default()
        };
        let text = build_prompt(TaskId(42), case.title, case.body, None, None, &ctx);
        let label = case.label;
        for needle in case.present {
            assert!(
                text.contains(needle),
                "{label}: missing {needle:?}, got: {text}"
            );
        }
        for needle in case.absent {
            assert!(
                !text.contains(needle),
                "{label}: carries {needle:?}, which belongs to another \
branch, got: {text}"
            );
        }
    }
}

/// A rendered prompt never ships a `{{KEY}}` the renderer failed to fill.
/// Asserted across every builder, because the failure is silent: a
/// mistyped or newly-added placeholder reaches the agent as literal braces,
/// and only a human reading the prompt would notice.
#[test]
fn no_rendered_prompt_carries_an_unsubstituted_placeholder() {
    let with_pr = PromptContext {
        tag: Some(TaskTag::Dependabot),
        pr_url: Some("https://github.com/o/r/pull/7"),
        ..PromptContext::default()
    };
    let variants = [
        build_prompt(TaskId(42), "t", "d", None, None, &PromptContext::default()),
        build_prompt(
            TaskId(42),
            "t",
            "d",
            Some("/p/plan.md"),
            None,
            &PromptContext::default(),
        ),
        build_prompt(
            TaskId(42),
            "Bump foo from 1.0.0 to 2.0.0",
            "d",
            None,
            None,
            &PromptContext {
                tag: Some(TaskTag::Dependabot),
                ..PromptContext::default()
            },
        ),
        build_prompt(
            TaskId(42),
            "Bump foo from 1.0.0 to 1.0.1",
            "d",
            None,
            None,
            &with_pr,
        ),
        build_prompt(
            TaskId(42),
            "t",
            "d",
            None,
            None,
            &PromptContext {
                tag: Some(TaskTag::PrReview),
                ..PromptContext::default()
            },
        ),
        build_quick_dispatch_prompt(TaskId(42), "t", "d", None, &PromptContext::default()),
        build_research_prompt(TaskId(42), "t", "d", None, &PromptContext::default()),
    ];
    for text in variants {
        assert!(
            !text.contains("{{"),
            "an unsubstituted placeholder reached the agent: {text}"
        );
    }
}

/// The PR URL is already on the task, so the runbook states it instead of
/// asking the agent to dig it out of a description the feed truncated to
/// 500 characters. `gh` takes a URL wherever it takes a number, and the URL
/// identifies the repo, so `--repo <owner/repo>` goes with it.
#[test]
fn a_recorded_pr_url_replaces_the_extract_it_yourself_step() {
    let ctx = PromptContext {
        tag: Some(TaskTag::Dependabot),
        pr_url: Some("https://github.com/example/repo/pull/42"),
        ..PromptContext::default()
    };
    let text = build_prompt(
        TaskId(7),
        "Bump serde from 1.0.0 to 1.0.1",
        "some truncated body",
        None,
        None,
        &ctx,
    );

    assert!(
        text.contains("PR: https://github.com/example/repo/pull/42"),
        "the runbook must state the PR it already has, got: {text}"
    );
    assert!(
        !text.contains("url_type="),
        "nothing left to record, so the update_task(url=…) step must go, got: {text}"
    );
    assert!(
        !text.contains("--repo <owner/repo>"),
        "the URL identifies the repo, got: {text}"
    );
}

/// Only a pr-typed url reaches the runbook. A security-alert url handed to
/// `gh pr view` would fail five times over, so the absence of one has to
/// leave the find-it-yourself step in place.
#[test]
fn without_a_recorded_pr_url_the_runbook_still_asks_the_agent_to_find_it() {
    let ctx = PromptContext {
        tag: Some(TaskTag::Dependabot),
        ..PromptContext::default()
    };
    let text = build_prompt(
        TaskId(7),
        "Bump serde from 1.0.0 to 1.0.1",
        "d",
        None,
        None,
        &ctx,
    );
    assert!(
        text.contains("update_task(task_id=7, url=<URL>, url_type=\"pr\")"),
        "got: {text}"
    );
}

/// Every route reaches the two guard failures and the ask terminal, so
/// those are shared rather than branch-local. Asserted separately from
/// the exclusivity test above so a regression says which half broke.
#[test]
fn every_dependabot_route_keeps_the_shared_steps_and_the_ask_terminal() {
    for title in [
        "Bump foo from 1.0.0 to 1.0.1",
        "Bump foo from 1.0.0 to 1.1.0",
        "fix(deps): update dependency foo to v9",
        "fix(deps): update python (non-major)",
        "chore: something else",
    ] {
        let ctx = PromptContext {
            tag: Some(TaskTag::Dependabot),
            ..PromptContext::default()
        };
        let text = build_prompt(TaskId(42), title, "", None, None, &ctx);
        for needle in ["gh pr view", "gh pr checks", "ASK THE USER", "needs_input"] {
            assert!(
                text.contains(needle),
                "{title:?}: every route needs {needle:?}, got: {text}"
            );
        }
    }
}

/// `TheAppsVerdictGatesTheMerge`: the verdict check opens the merge
/// terminal, so it appears on exactly the routes that can merge. The ask
/// terminal no longer carries the app's verdict on every route — it is one
/// direct question, and names the app only where the app is the reason.
#[test]
fn the_apps_verdict_gates_the_merge() {
    const APP: &str = "kognic-github-app";
    for (title, merges) in [
        ("Bump foo from 1.0.0 to 1.0.1", true),
        ("Bump foo from 1.0.0 to 1.1.0", true),
        ("fix(deps): update dependency foo to v9", false),
        ("fix(deps): update python (non-major)", false),
        ("chore: something else", false),
    ] {
        let text = dependabot_prompt(title);
        let Some((merge, _ask)) = text.split_once("ASK THE USER:") else {
            panic!("{title:?}: no ask terminal, got: {text}");
        };
        let gated = merge
            .split_once("AUTO-APPROVE + MERGE:")
            .is_some_and(|(_, terminal)| {
                let first_step = terminal.split("gh pr review").next().unwrap_or("");
                first_step.contains(APP) && first_step.contains("APPROVED")
            });
        assert_eq!(
                gated, merges,
                "{title:?}: the verdict gate must open the merge terminal exactly when it renders, got: {text}"
            );
    }
}

/// Every bump kind the runbook can render, by title.
const DEPENDABOT_ROUTES: [&str; 5] = [
    "Bump foo from 1.0.0 to 1.0.1",
    "Bump foo from 1.0.0 to 1.1.0",
    "fix(deps): update dependency foo to v9",
    "fix(deps): update python (non-major)",
    "chore: something else",
];

fn dependabot_prompt(title: &str) -> String {
    let ctx = PromptContext {
        tag: Some(TaskTag::Dependabot),
        ..PromptContext::default()
    };
    build_prompt(TaskId(42), title, "", None, None, &ctx)
}

/// `TheRunbookStatesThePolicyNotItsMechanics`: the dep-only check is a plain
/// policy — dependency manifests, lockfiles and pinned GitHub Action versions
/// in workflow files — applied by the agent's judgement, not a path allowlist
/// it has to match. Naming the action-pin case keeps Action bumps from all
/// escalating to the human.
#[test]
fn the_dependabot_runbook_states_the_dep_only_policy_in_plain_words() {
    for title in DEPENDABOT_ROUTES {
        let text = dependabot_prompt(title);
        let lower = text.to_lowercase();
        for needle in ["manifest", "lockfile", "github action", "workflow"] {
            assert!(
                lower.contains(needle),
                "{title:?}: the dep-only policy must name {needle:?}, got: {text}"
            );
        }
    }
}

/// The same guarantee: the sixteen-glob allowlist is gone. The agent applies
/// the reason the list existed to the files in front of it, including
/// ecosystems the list never named.
#[test]
fn the_dependabot_runbook_carries_no_path_allowlist() {
    for title in DEPENDABOT_ROUTES {
        let text = dependabot_prompt(title);
        assert!(
            !text.contains("must match one of"),
            "{title:?}: the runbook must not carry a path allowlist, got: {text}"
        );
        for glob in [
            "requirements*.txt",
            "pnpm-lock.yaml",
            "composer.lock",
            "Gemfile.lock",
            "go.sum",
            "gradle/libs.versions.toml",
            ".github/workflows/*",
        ] {
            assert!(
                !text.contains(glob),
                "{title:?}: {glob:?} is an allowlist entry, and the allowlist is \
gone, got: {text}"
            );
        }
    }
}

/// The same guarantee: "breaking change" is a judgement about the changelog,
/// not a token scan — a deprecation, a removed option or a changed default
/// can suggest one without any listed word. The minor branch still reads the
/// changelog and asks that question.
#[test]
fn the_minor_branch_judges_the_changelog_rather_than_scanning_for_tokens() {
    let text = dependabot_prompt("Bump foo from 1.0.0 to 1.1.0");
    let lower = text.to_lowercase();
    assert!(
        lower.contains("changelog") && lower.contains("breaking"),
        "the minor branch must still read the changelog for a breaking change, got: {text}"
    );
    for token_list in ["tokens", "major rewrite", "deprecat,", "BREAKING"] {
        assert!(
            !text.contains(token_list),
            "the minor branch must not carry a breaking-change token list \
(found {token_list:?}), got: {text}"
        );
    }
}

/// The same guarantee: the approval message is the agent's. The runbook does
/// not dictate its body; what it must not do is claim a check it did not
/// perform. The approve and squash auto-merge commands stay.
#[test]
fn the_merge_terminal_dictates_no_approval_body() {
    for title in [
        "Bump foo from 1.0.0 to 1.0.1",
        "Bump foo from 1.0.0 to 1.1.0",
    ] {
        let text = dependabot_prompt(title);
        assert!(
            !text.contains("--approve --body \""),
            "{title:?}: the runbook must not dictate the approval body, got: {text}"
        );
        assert!(
            !text.contains("Auto-approved by dispatch dependabot agent"),
            "{title:?}: the exact approval text is gone, got: {text}"
        );
        assert!(
            text.contains("--approve") && text.contains("--squash --auto"),
            "{title:?}: approve + squash auto-merge stay, got: {text}"
        );
    }
}

/// The same guarantee: the ask terminal is ONE direct question that says why
/// the agent did not approve, and the agent picks what the human needs. The
/// eight-item template for that question is gone.
#[test]
fn the_ask_terminal_is_one_direct_question_without_a_template() {
    for title in DEPENDABOT_ROUTES {
        let text = dependabot_prompt(title);
        let Some((_, ask)) = text.split_once("ASK THE USER:") else {
            panic!("{title:?}: no ask terminal, got: {text}");
        };
        let lower = ask.to_lowercase();
        assert!(
            lower.contains("one direct question"),
            "{title:?}: the ask terminal must still ask one direct question, got: {ask}"
        );
        assert!(
            lower.contains("why"),
            "{title:?}: the question must say why the agent did not approve, got: {ask}"
        );
        for item in [
            "that includes:",
            "dep-only verdict",
            "ci status summary",
            "changelog summary or its absence",
        ] {
            assert!(
                !lower.contains(item),
                "{title:?}: the ask terminal must not carry the item template \
(found {item:?}), got: {ask}"
            );
        }
    }
}

#[test]
fn build_prompt_with_pr_review_tag_includes_the_learning_instruction() {
    let ctx = PromptContext {
        tag: Some(TaskTag::PrReview),
        ..PromptContext::default()
    };
    let text = build_prompt(
        TaskId(42),
        "Review PR: Add new login flow",
        "https://github.com/example/repo/pull/99",
        None,
        None,
        &ctx,
    );

    assert!(
        text.contains("/learnings"),
        "pr-review prompt must include the knowledge-base line"
    );
}

/// No prompt variant may render a "## Verification" section — see
/// `ThePromptCarriesNoVerifyCommand` in `docs/specs/dispatch-prompt.allium`.
///
/// The builders take no verify input, so this can only regress through
/// hardcoded prompt copy. That is exactly the case the snapshots don't
/// catch: `INSTA_UPDATE=always` silently accepts a reintroduced section,
/// whereas a named test cannot be blanket-accepted.
#[test]
fn no_prompt_variant_renders_a_verification_section() {
    let ctx = PromptContext::default();
    let variants = [
        (
            "dispatch",
            build_prompt(TaskId(1), "t", "d", None, None, &ctx),
        ),
        (
            "quick dispatch",
            build_quick_dispatch_prompt(TaskId(1), "t", "d", None, &ctx),
        ),
        (
            "research",
            build_research_prompt(TaskId(1), "t", "d", None, &ctx),
        ),
    ];
    for (variant, text) in variants {
        assert!(
            !text.contains("## Verification"),
            "{variant} prompt must not render a verification section"
        );
        assert!(
            !text.contains("Before declaring work complete"),
            "{variant} prompt must not carry the verification instruction"
        );
    }
}

// -- The author check is omitted only where a feed did the filtering --
// (task #4728; see AReviewRunbookCarriesOnlyTheBranchThatApplies)

const AUTHOR_CLAIM: &str = "Do not re-check the PR author";

#[test]
fn a_feed_created_dependabot_prompt_omits_the_author_check() {
    let ctx = PromptContext {
        tag: Some(TaskTag::Dependabot),
        pr_url: Some("https://github.com/o/r/pull/42"),
        from_feed: true,
        ..PromptContext::default()
    };
    let text = build_prompt(
        TaskId(42),
        "#42 Bump serde from 1.0.0 to 1.0.1",
        "",
        None,
        None,
        &ctx,
    );
    assert!(
        text.contains(AUTHOR_CLAIM),
        "a feed listed this PR by bot author, so the check can only agree: {text}"
    );
}

/// No feed created this task, so no author filter ever ran. The sentence
/// asserts a fact, and asserting it here would be asserting a falsehood —
/// the agent checks the author itself instead.
#[test]
fn a_hand_created_dependabot_prompt_makes_no_claim_about_a_filter() {
    let ctx = PromptContext {
        tag: Some(TaskTag::Dependabot),
        from_feed: false,
        ..PromptContext::default()
    };
    let text = build_prompt(
        TaskId(42),
        "#42 Bump serde from 1.0.0 to 1.0.1",
        "",
        None,
        None,
        &ctx,
    );
    assert!(
        !text.contains(AUTHOR_CLAIM),
        "a task no feed created must not be told a feed filtered it: {text}"
    );
    assert!(
        !text.contains("already passed that filter"),
        "no half of the claim may survive: {text}"
    );
    // The rest of step 1 must be intact — only the one bullet goes.
    assert!(
        text.contains("Verify the PR touches only dependency files"),
        "dropping the bullet must not drop its step: {text}"
    );
    assert!(
        text.contains("Check CI"),
        "dropping the bullet must not disturb the step after it: {text}"
    );
}

/// Both halves are required. A feed created this task, but nothing on it
/// names a PR — so there is no PR whose author a filter could have vetted,
/// and the claim is not rendered. Reachable two ways: a task from a feed
/// that is not a PR feed (the CVE feed sets external_id too) and was later
/// retagged `dependabot` over MCP, and a feed-created review task whose
/// url was cleared or retyped afterwards.
#[test]
fn a_feed_created_task_that_names_no_pr_makes_no_claim_about_a_filter() {
    let ctx = PromptContext {
        tag: Some(TaskTag::Dependabot),
        pr_url: None,
        from_feed: true,
        ..PromptContext::default()
    };
    let text = build_prompt(
        TaskId(42),
        "#42 Bump serde from 1.0.0 to 1.0.1",
        "",
        None,
        None,
        &ctx,
    );
    assert!(
        !text.contains(AUTHOR_CLAIM),
        "with no PR recorded there is no filtered author to stand on: {text}"
    );
}

/// Provenance is read from external_id, not from having a PR url — a
/// hand-created task can carry one, since update_task takes a url and the
/// dependabot tag together.
#[test]
fn a_pr_url_alone_does_not_make_a_task_feed_created() {
    let ctx = PromptContext {
        tag: Some(TaskTag::Dependabot),
        pr_url: Some("https://github.com/o/r/pull/42"),
        from_feed: false,
        ..PromptContext::default()
    };
    let text = build_prompt(
        TaskId(42),
        "#42 Bump serde from 1.0.0 to 1.0.1",
        "",
        None,
        None,
        &ctx,
    );
    assert!(
        text.contains("PR: https://github.com/o/r/pull/42"),
        "the url is still rendered — it is recorded, whoever recorded it: {text}"
    );
    assert!(
        !text.contains(AUTHOR_CLAIM),
        "but a recorded url is not evidence a feed filtered the author: {text}"
    );
}

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
        "Design the solution spec-first",
        "Always use TDD",
    ] {
        assert!(
            !text.contains(token),
            "a CVE task must not be sent to {token}, got: {text}"
        );
    }
}

/// The runbook is not the review runbooks: a CVE task authors its own PR
/// and finishes through `/wrap-up`, so the closing lines are owed to it.
#[test]
fn cve_task_prompt_keeps_wrap_up_and_the_knowledge_base() {
    let ctx = PromptContext::default();
    let text = build_prompt(TaskId(1), "t", "d", None, Some(&cve_epic()), &ctx);
    assert!(
        text.contains("/wrap-up"),
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
    for tag in [TaskTag::Dependabot, TaskTag::PrReview] {
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
    for tag in [TaskTag::Dependabot, TaskTag::PrReview, TaskTag::Research] {
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
