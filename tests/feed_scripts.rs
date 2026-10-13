//! Structural guards on the reference feed scripts under `scripts/` — the
//! wrap-up path each one declares, and the config each one shares.
//!
//! The wrap_up_mode guards start with #4283: every script emitting GitHub
//! Dependabot vulnerability alerts (CVEs) must default `wrap_up_mode` to
//! `"pr"`. `fetch-cve.sh` got this in `8d9942f4 feat(feed): let feed items
//! declare wrap_up_mode; CVE feed defaults to pr`, but the commit only
//! touched that one script — `fetch-security.sh`, which hits the same `gh
//! api .../dependabot/alerts` endpoint, was left emitting items with no
//! `wrap_up_mode` at all. These tests make that drift structural: editing one
//! script's `wrap_up_mode` handling without the other now fails the suite.
//! `fetch-log-warnings.sh` is pinned the same way, to `"rebase"`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

fn repo_file(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// A feed script with every comment line stripped, so a `contains` assertion
/// pins the field the script actually EMITS rather than prose that happens to
/// quote it. Without this, a header comment mentioning `wrap_up_mode: "pr"`
/// satisfies the assertion on its own and the guard silently stops guarding —
/// the failure mode is invisible, because the test still passes.
///
/// Every comment in these scripts, shell and jq alike, is a whole line whose
/// first non-space character is `#`; neither language's syntax puts a bare `#`
/// at the start of a line that does anything else.
fn feed_script_code(rel: &str) -> String {
    repo_file(rel)
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The Dependabot review prompt and the feed that creates its tasks must not
/// both check the PR author. `fetch-dependabot.sh` lists PRs with
/// `--author "$author"` over `bots.conf`'s `BOT_AUTHORS`, so a dependabot task
/// exists only for a PR that already passed that filter — an agent re-deriving
/// it spends a `gh` call on a check that can only ever agree.
///
/// Pinned from both sides: if the feed ever drops its author filter, this fails
/// and the prompt's "do not re-check" instruction stops being true. Pinned on
/// the LOOP rather than a login, because which bots a deployment runs is
/// configuration — the instruction is true for any non-empty author filter.
#[test]
fn dependabot_prompt_does_not_re_check_the_author_the_feed_already_filtered() {
    let feed = repo_file("scripts/fetch-dependabot.sh");
    assert!(
        feed.contains("--author \"$author\""),
        "the feed must filter by bot author for the prompt's skipped check to be safe"
    );

    let prompt = repo_file("src/dispatch/prompts/dependabot.md");
    assert!(
        !prompt.contains("commits[].authors[].login"),
        "the prompt must not re-derive the author the feed already filtered on"
    );

    // The bullet lives in its own fragment because it is rendered only for a
    // feed-created task — a task no feed made was never filtered, so telling it
    // otherwise would be stating a falsehood. See
    // AReviewRunbookCarriesOnlyTheBranchThatApplies in docs/specs/dispatch-prompt.allium.
    assert!(
        prompt.contains("{{AUTHOR}}"),
        "dependabot.md must leave a slot for the author bullet rather than stating it \
         unconditionally"
    );
    let author = repo_file("src/dispatch/prompts/dependabot/author.md");
    assert!(
        author.contains("Do not re-check the PR author"),
        "the fragment must say why the author check is absent, or the next author will \
         re-add it"
    );
}

/// `bots.conf` has two readers with two DIFFERENT empty-value behaviours:
/// `fetch-reviews.sh` skips its bot-author pass, while `fetch-dependabot.sh`
/// falls back to `app/kognic-renovate`. A user editing the file sees only the
/// file, so the file has to say both — and each reader's own comment has to say
/// the list is shared, or the next author will add a second bot list.
#[test]
fn bots_conf_and_both_its_readers_say_the_list_is_shared() {
    let conf = repo_file("scripts/bots.conf");
    for reader in ["fetch-reviews.sh", "fetch-dependabot.sh"] {
        assert!(
            conf.contains(reader),
            "bots.conf must name {reader} as a reader, or an editor cannot know what \
             changing BOT_AUTHORS affects"
        );
    }
    assert!(
        conf.contains("app/kognic-renovate"),
        "bots.conf must state the fallback fetch-dependabot.sh applies to an empty list, \
         since that is the shipped state of the file"
    );

    for reader in ["scripts/fetch-reviews.sh", "scripts/fetch-dependabot.sh"] {
        let body = repo_file(reader);
        assert!(
            body.contains("bots.conf"),
            "{reader} must name bots.conf as where its bot logins come from"
        );
        assert!(
            !body.contains("bot-author pass ONLY"),
            "{reader} must not claim sole ownership of a list two scripts read"
        );
    }
}

#[test]
fn dependabot_alert_feed_scripts_default_wrap_up_mode_to_pr() {
    for script in ["scripts/fetch-cve.sh", "scripts/fetch-security.sh"] {
        let body = feed_script_code(script);
        assert!(
            body.contains(r#"wrap_up_mode: "pr""#),
            "{script} emits Dependabot vulnerability alerts (CVEs) and must set \
             wrap_up_mode: \"pr\" on every item so wrap-up always goes straight to a PR"
        );
    }
}

/// The log-warnings feed creates triage cards for dispatch's own WARN/ERROR
/// records. Every one of the three resolutions — fix the bug, demote the log
/// line, delete the card — is a small change landing on the repo the log came
/// from, so the card's wrap-up path is always rebase-onto-base. Pinned here so
/// the script cannot quietly go back to leaving the choice to wrap-up time.
#[test]
fn log_warning_feed_script_defaults_wrap_up_mode_to_rebase() {
    let body = feed_script_code("scripts/fetch-log-warnings.sh");
    assert!(
        body.contains(r#"wrap_up_mode: "rebase""#),
        "scripts/fetch-log-warnings.sh must set wrap_up_mode: \"rebase\" on every item \
         so a triage card wraps up by landing on its base branch"
    );
}

/// feed-scripts.allium "Release bots get their own tag": bots.conf carries a
/// second list, RELEASE_BOT_AUTHORS, documenting `app/kognic-gha-release-bot`
/// as the example and shipping the list EMPTY. The example lives only in
/// prose: an uncommented entry would arm the release runbook on every fresh
/// install.
#[test]
fn bots_conf_documents_release_bot_authors_and_ships_it_empty() {
    let conf = repo_file("scripts/bots.conf");
    assert!(
        conf.contains("RELEASE_BOT_AUTHORS"),
        "bots.conf must document the RELEASE_BOT_AUTHORS list"
    );
    assert!(
        conf.contains("app/kognic-gha-release-bot"),
        "bots.conf must give app/kognic-gha-release-bot as the release-bot example"
    );

    let code = feed_script_code("scripts/bots.conf");
    assert!(
        code.lines().any(|l| l.trim() == "RELEASE_BOT_AUTHORS=()"),
        "bots.conf must ship RELEASE_BOT_AUTHORS empty, got code:\n{code}"
    );
    assert!(
        !code.contains("kognic-gha-release-bot"),
        "the release-bot example must stay in a comment, never a shipped entry, got code:\n{code}"
    );
    assert!(
        code.lines().any(|l| l.trim() == "BOT_AUTHORS=()"),
        "BOT_AUTHORS still ships empty, got code:\n{code}"
    );
}

/// An existing bots.conf is never rewritten (InstallShippedFeedConfigs), so an
/// old conf has no RELEASE_BOT_AUTHORS. Both readers pre-declare it empty
/// BEFORE sourcing the conf, as they do BOT_AUTHORS, so an old conf reads as
/// an empty list rather than tripping `set -u`.
#[test]
fn both_readers_pre_declare_release_bot_authors_before_sourcing_bots_conf() {
    for reader in ["scripts/fetch-reviews.sh", "scripts/fetch-dependabot.sh"] {
        let code = feed_script_code(reader);
        let declare = code
            .find("RELEASE_BOT_AUTHORS=()")
            .unwrap_or_else(|| panic!("{reader} must pre-declare RELEASE_BOT_AUTHORS=()"));
        let source = code
            .find("source \"$SCRIPT_DIR/bots.conf\"")
            .unwrap_or_else(|| panic!("{reader} must source bots.conf"));
        assert!(
            declare < source,
            "{reader} must declare RELEASE_BOT_AUTHORS=() before sourcing bots.conf"
        );
    }
}

/// feed-scripts.allium: "THE TAG COMES FROM THE AUTHOR CONFIG, NEVER FROM THE
/// "[bot]" SUFFIX". fetch-reviews.sh emits the release-please tag, and the
/// emitted tag must be the wire string the board parses.
#[test]
fn fetch_reviews_emits_the_release_please_wire_tag() {
    let code = feed_script_code("scripts/fetch-reviews.sh");
    assert!(
        code.contains("\"release-please\""),
        "fetch-reviews.sh must emit tag \"release-please\" for a RELEASE_BOT_AUTHORS PR"
    );
    assert!(
        code.contains("RELEASE_BOT_AUTHORS"),
        "fetch-reviews.sh must read RELEASE_BOT_AUTHORS from bots.conf"
    );
}

/// fetch-dependabot.sh reads RELEASE_BOT_AUTHORS for the overlap check alone
/// ("A bot on both lists is fatal"): it never lists PRs by it and never emits
/// the release-please tag.
#[test]
fn fetch_dependabot_reads_release_bot_authors_only_for_the_overlap_check() {
    let code = feed_script_code("scripts/fetch-dependabot.sh");
    assert!(
        code.contains("RELEASE_BOT_AUTHORS"),
        "fetch-dependabot.sh must read RELEASE_BOT_AUTHORS for the overlap check"
    );
    assert!(
        !code.contains("release-please"),
        "fetch-dependabot.sh must never emit the release-please tag"
    );
}
