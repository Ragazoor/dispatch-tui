use super::*;

/// The pull-request skill body with every run of whitespace folded to one
/// space, so a phrase the Markdown wraps across lines still matches.
fn pull_request_text() -> String {
    skill_body("pull-request")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// /pull-request guides an agent through one PR. Each guarantee of
/// `PullRequestSession` in `docs/specs/pull-request-skill.allium` is pinned
/// here by the words in `SKILL.md` that hold it; the skill is followed by an
/// agent, so the text is the behaviour a test can pin.
#[test]
fn pull_request_skill_text_holds_each_session_guarantee() {
    let content = pull_request_text();
    for (guarantee, needle) in [
        ("ForAHumanWithAMinute", "in about a minute"),
        ("ForAHumanWithAMinute", "would the reviewer miss it"),
        ("ForAHumanWithAMinute", "judgement"),
        ("RepoRulesComeFirst", "PR template"),
        ("RepoRulesComeFirst", "come first"),
        ("OneChangePerPr", "the whole diff"),
        ("OneChangePerPr", "A PR is one change"),
        ("TitleIsOneAction", "72 characters"),
        ("TitleIsOneAction", "git log --oneline"),
        ("OpeningCarriesThePr", "no template"),
        ("OpeningCarriesThePr", "first 500 characters"),
        ("ShowsTheChangeWhereItHasAShape", "before/after"),
        ("ShowsTheChangeWhereItHasAShape", "table"),
        ("LeavesOutWhatTheReviewerHas", "summary"),
        ("LeavesOutWhatTheReviewerHas", "table of contents"),
        ("LeavesOutWhatTheReviewerHas", "ADR"),
        ("OperatorPreferences", "bullet lists"),
        ("OperatorPreferences", "Test plan"),
        ("OperatorPreferences", "dangerous or breaking"),
        (
            "NeverReferencesTheDispatchTask",
            "Do not mention the dispatch task",
        ),
        ("AgentNotesInAComment", "HTML comment"),
        ("OpensAgainstTheBaseBranch", "`--base`"),
        ("NeverForcePushes", "never force-push"),
        ("ReusesAnExistingPr", "already exists"),
        ("KeepsItCurrent", "gh pr edit"),
        ("KeepsItCurrent", "not a log of review rounds"),
    ] {
        assert!(
            content.contains(needle),
            "pull-request skill must hold {guarantee}: missing {needle:?}"
        );
    }
}

/// The skill is a guide that leaves room for judgement, not a template: it
/// prescribes no fixed headings or category labels, and it does not shout.
/// See the design decisions at the head of `docs/specs/pull-request-skill.allium`.
#[test]
fn pull_request_skill_is_a_guide_not_a_template() {
    let body = skill_body("pull-request");
    for template in [
        "## Summary",
        "**Breaking Changes**",
        "**New Features**",
        "**Bug Fixes**",
    ] {
        assert!(
            !body.contains(template),
            "pull-request skill must not prescribe a template, found: {template}"
        );
    }
    for shout in ["MUST", "NEVER", "ALWAYS", "CRITICAL", "IMPORTANT"] {
        assert!(
            !body
                .split(|c: char| !c.is_alphanumeric())
                .any(|w| w == shout),
            "pull-request skill is a guide; found the all-caps rule word {shout}"
        );
    }
}
