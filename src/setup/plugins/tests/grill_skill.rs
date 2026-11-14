use super::*;

/// The grill skill body with every run of whitespace folded to one space, so
/// a phrase the Markdown wraps across lines still matches.
fn grill_text() -> String {
    skill_body("grill")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// /grill interviews the user about a plan, then records the decisions.
/// Each guarantee of `GrillSessionSurface` in `docs/specs/grill-skill.allium`
/// that the skill text can hold is pinned here by the words in `SKILL.md`
/// that hold it; the skill is followed by an agent, so the text is the
/// behaviour a test can pin.
#[test]
fn grill_skill_text_holds_each_session_guarantee() {
    let content = grill_text();
    for (guarantee, needle) in [
        (
            "AnnouncesItself",
            "I'm using the grill skill to interview you about this.",
        ),
        (
            "GrillInvocation",
            "\"grill me on this\", \"poke holes in this\"",
        ),
        ("OneQuestionAtATime", "Ask one question at a time"),
        ("OneQuestionAtATime", "wait for the answer"),
        (
            "EveryQuestionCarriesARecommendation",
            "Give your recommended answer with each question",
        ),
        (
            "CodebaseAnswersBeforeTheUserIsAsked",
            "If the codebase can answer a question, read it instead of asking",
        ),
        ("DependenciesBeforeDependents", "Walk the design tree"),
        (
            "DependenciesBeforeDependents",
            "resolving each decision before the ones that depend on it",
        ),
        (
            "SpecsAreTheVocabulary",
            "Name a term that is fuzzy or used two ways",
        ),
        ("SpecsAreTheVocabulary", "`docs/specs/*.allium`"),
        ("SpecsAreTheVocabulary", "shared vocabulary"),
        (
            "SpecsAreTheVocabulary",
            "when a decision contradicts or extends a spec",
        ),
        (
            "EdgesAreProbedAfterEachDecision",
            "probe its edges: nulls, concurrent agents, the failure path",
        ),
        (
            "TheInterviewEndsAtSharedUnderstanding",
            "reach a shared understanding of the plan",
        ),
        (
            "TheInterviewEndsAtSharedUnderstanding",
            "written down where the next agent will find them",
        ),
        ("DomainDecisionsGoThroughTend", "`allium:tend`"),
        (
            "DomainDecisionsGoThroughTend",
            "Record only what was decided",
        ),
        ("AdrOnlyWhenAllThree", "hard to reverse"),
        ("AdrOnlyWhenAllThree", "surprising without context"),
        ("AdrOnlyWhenAllThree", "had real alternatives"),
        ("SkipWhatIsAlreadyRecorded", "Skip obvious choices"),
        (
            "SkipWhatIsAlreadyRecorded",
            "anything the spec already captures",
        ),
        ("SkipWhatIsAlreadyRecorded", "implementation details"),
        ("AdrLocation", "`docs/adr/NNNN-slug.md`"),
        ("AdrLocation", "next free number"),
        ("AdrLocation", "create the directory if needed"),
        (
            "AdrShape",
            "`Status`, `Context`, `Decision` and `Consequences`",
        ),
        ("AdrShape", "a few short paragraphs"),
    ] {
        assert!(
            content.contains(needle),
            "grill skill must hold {guarantee}: missing {needle:?}"
        );
    }
}

/// GrillSession.stage: the interview comes before the capture, and the
/// announcement before both.
#[test]
fn grill_skill_interviews_before_it_captures() {
    let content = skill_body("grill");
    let order = ["**Announce at start:**", "## Interview", "## Capture"];
    let positions: Vec<usize> = order
        .iter()
        .map(|n| {
            content
                .find(n)
                .unwrap_or_else(|| panic!("grill skill must have {n}"))
        })
        .collect();
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "grill must follow the session's stage order: {order:?}"
    );
}
