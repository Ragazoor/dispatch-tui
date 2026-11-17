use super::*;

/// The voice skill body with every run of whitespace folded to one space, so
/// a phrase the Markdown wraps across lines still matches.
fn voice_text() -> String {
    skill_body("voice")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// /voice drafts a message in the operator's voice and learns from the edits.
/// Each guarantee of `VoiceSession` in `docs/specs/voice-skill.allium` is
/// pinned here by the words in `SKILL.md` that hold it; the skill is followed
/// by an agent, so the text is the behaviour a test can pin.
#[test]
fn voice_skill_text_holds_each_session_guarantee() {
    let content = voice_text();
    for (guarantee, needle) in [
        (
            "VoiceInvocation",
            "I'm using the voice skill to draft this.",
        ),
        (
            "ContextFolderComesFromTheEnvironment",
            "`$DISPATCH_VOICE_CONTEXT`",
        ),
        (
            "ContextFolderComesFromTheEnvironment",
            "`~/.local/share/dispatch/voice-context`",
        ),
        ("ContextFolderComesFromTheEnvironment", "`realpath`"),
        ("RulesBeforeSamples", "Read `rules.md` first"),
        ("RulesBeforeSamples", "then every file in `samples/`"),
        (
            "RulesBeforeSamples",
            "Tell the user which files you loaded, by the resolved path",
        ),
        (
            "MissingFolderStillDrafts",
            "draft without a voice, say so in one line",
        ),
        (
            "MissingFolderStillDrafts",
            "do not fall back to the default",
        ),
        ("DraftStaysInChat", "Show the draft in the chat only"),
        (
            "DraftStaysInChat",
            "Never post to Slack, and never send or schedule a message",
        ),
        ("SampleIsVerbatim", "the final text verbatim"),
        ("RuleEditsNeedARepeat", "in two or more messages"),
        (
            "RuleEditsNeedARepeat",
            "A single edit goes into `observations.md` only",
        ),
        (
            "ProposalsShowPathAndLines",
            "the absolute path and the exact lines",
        ),
        (
            "ProposalsShowPathAndLines",
            "Write only after the user approves",
        ),
        (
            "NeverTidiesTheOperatorsWords",
            "Do not tidy the user's grammar or capitalisation",
        ),
        ("NeverCommitsInTheFolder", "Do not commit in that folder"),
    ] {
        assert!(
            content.contains(needle),
            "voice skill must hold {guarantee}: missing {needle:?}"
        );
    }
    let at = |needle: &str| {
        content
            .find(needle)
            .unwrap_or_else(|| panic!("voice skill must mention {needle}"))
    };
    // Load, then draft, then learn (RulesBeforeSamples, DraftStaysInChat,
    // SampleIsVerbatim).
    assert!(
        at("Read `rules.md` first") < at("Show the draft in the chat only")
            && at("Show the draft in the chat only") < at("the final text verbatim"),
        "voice skill must load, then draft, then learn"
    );
}
