use super::*;

/// The learnings skill body with every run of whitespace folded to one space,
/// so a phrase the Markdown wraps across lines still matches.
fn learnings_text() -> String {
    skill_body("learnings")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// /learnings is followed by an agent, so the words in `SKILL.md` are the
/// behaviour a test can pin. Each guarantee of `LearningsSkillSession` in
/// `docs/specs/learnings-skill.allium` that has no dedicated test in
/// `skill_workflow.rs` is pinned here by the words that hold it.
#[test]
fn learnings_skill_text_holds_each_session_guarantee() {
    let content = learnings_text();
    for (guarantee, needle) in [
        (
            "AnnouncesItself",
            "I'm using the learnings skill to interact with the knowledge base.",
        ),
        ("QueryDirectlyWhenUnclear", "call `query_learnings` directly"),
        (
            "QueryDirectlyWhenUnclear",
            "Do that when anything is unclear — before guessing or asking.",
        ),
        (
            "TagFilterOnlyForAKnownTag",
            "Reach for `tag_filter` only when you already know the tag you want.",
        ),
        (
            "TagFilterOnlyForAKnownTag",
            "It is a soft score boost rather than a hard filter",
        ),
        ("RateAtTheMomentOfActing", "rate it at that moment"),
        (
            "VerdictsMeanWhatTheySay",
            "Use `helped` when the entry saved you from a pitfall or guided a decision",
        ),
        (
            "VerdictsMeanWhatTheySay",
            "`wrong` when it misled you or is out of date",
        ),
        (
            "NotApplicableForAnEntryThatDidNotFit",
            "use `not_applicable`",
        ),
        (
            "NotApplicableForAnEntryThatDidNotFit",
            "keep the entry out of that repo's dispatch prompts",
        ),
        (
            "SkimmedResultsAreNotRated",
            "Don't rate `query_learnings` results you skimmed and didn't act on.",
        ),
        (
            "ClearlyWrongIsDeleted",
            "An entry that is clearly wrong should be deleted instead.",
        ),
        (
            "ClearlyWrongIsDeleted",
            "Use `query_learnings` first to find the entry's ID",
        ),
        (
            "ReflectBeforeFinishing",
            "Before finishing a task, ask: *Did I discover anything non-obvious that a future agent would benefit from knowing?*",
        ),
        ("RecordingCasesAreListed", "### Record if:"),
        ("RecordingCasesAreListed", "**preference**"),
        ("RecordingCasesAreListed", "**landscape understanding**"),
        ("RecordingCasesAreListed", "**convention**"),
        ("RecordingCasesAreListed", "**workflow pattern**"),
        ("RecordingCasesAreListed", "**procedural step**"),
        (
            "KindScopeAndSummaryFollowTheRecordingFile",
            "how to write the one-sentence summary",
        ),
        (
            "RecordingDetailIsReadFirst",
            "Read `references/recording.md`",
        ),
    ] {
        assert!(
            content.contains(needle),
            "{guarantee}: the learnings skill must contain {needle:?}"
        );
    }
}

/// RecordingDetailIsReadFirst: the "Before calling `record_learning`" step
/// must come before the call is described as available, and the file it names
/// must ship with the skill, or the instruction points at nothing.
#[test]
fn learnings_skill_reads_a_recording_file_that_ships() {
    let (name, _) =
        reference_containing("learnings", "### Do NOT record:", "the Do NOT record list");
    assert!(
        skill_body("learnings").contains(&format!("references/{name}")),
        "SKILL.md must cite references/{name}, where the Do NOT record list lives"
    );
}
