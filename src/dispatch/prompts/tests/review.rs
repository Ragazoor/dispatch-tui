use super::*;

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
    assert!(text.contains("update_tasks(task_ids=[42], url="));
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
        !text.contains("use the dispatch:wrap-up skill"),
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
        !text.contains("use the dispatch:wrap-up skill"),
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
        (
            "release-please",
            TaskTag::ReleasePlease,
            ["auto-cleaned", "needs_input"],
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
            text.matches("dispatch:wrap-up").count(),
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
        build_prompt(
            TaskId(42),
            "chore(master): release 8.21.10",
            "d",
            None,
            None,
            &PromptContext {
                tag: Some(TaskTag::ReleasePlease),
                ..PromptContext::default()
            },
        ),
        build_prompt(
            TaskId(42),
            "chore(master): release 8.21.10",
            "d",
            None,
            None,
            &PromptContext {
                tag: Some(TaskTag::ReleasePlease),
                pr_url: Some("https://github.com/o/r/pull/7"),
                from_feed: true,
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
        "nothing left to record, so the update_tasks(url=…) step must go, got: {text}"
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
        text.contains("update_tasks(task_ids=[7], url=<URL>, url_type=\"pr\")"),
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
/// hand-created task can carry one, since update_tasks takes a url and the
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
