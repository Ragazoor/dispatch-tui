use super::*;
use proptest::prelude::*;

// The five title shapes below are verbatim from dispatch's own board
// (epic 275, the airflow-images bot queue), which is where the mismatch
// this module fixes was found: thirteen of fifteen open bot PRs were
// Renovate-titled and matched none of the runbook's parsing rules.
#[test]
fn dependabot_patch_title_classifies_as_patch() {
    let bump = classify(
        "#25 Bump dbt-common from 1.37.2 to 1.37.3 in /kognic-airflow/venvs/dbt",
        "",
    );
    assert_eq!(bump.kind, BumpKind::Patch);
    assert_eq!(bump.package.as_deref(), Some("dbt-common"));
    assert_eq!(bump.from.as_deref(), Some("1.37.2"));
    assert_eq!(bump.to.as_deref(), Some("1.37.3"));
}

#[test]
fn dependabot_minor_title_classifies_as_minor() {
    let bump = classify(
        "#29 Bump requests from 2.32.4 to 2.33.0 in /kognic-airflow/venvs/basic",
        "",
    );
    assert_eq!(bump.kind, BumpKind::Minor);
    assert_eq!(bump.package.as_deref(), Some("requests"));
}

#[test]
fn renovate_bare_major_target_classifies_as_major() {
    let bump = classify("#47 fix(deps): update dependency deepdiff to v9", "");
    assert_eq!(bump.kind, BumpKind::Major);
    assert_eq!(bump.package.as_deref(), Some("deepdiff"));
    assert_eq!(bump.to.as_deref(), Some("v9"));
    assert_eq!(
        bump.from, None,
        "a Renovate title carries no source version"
    );
}

#[test]
fn renovate_action_title_classifies_as_major() {
    let bump = classify("#64 chore(deps): update actions/checkout action to v7", "");
    assert_eq!(bump.kind, BumpKind::Major);
    assert_eq!(bump.package.as_deref(), Some("actions/checkout"));
}

#[test]
fn renovate_grouped_non_major_is_its_own_kind() {
    let bump = classify("#79 fix(deps): update python (non-major)", "");
    assert_eq!(bump.kind, BumpKind::NonMajor);
    assert_eq!(bump.package.as_deref(), Some("python"));
}

/// A group declaring `(minor)` or `(patch)` must NOT become Minor/Patch.
/// Those route to the changelog branch, which reads one package's release
/// notes and merges when they come back clean — a check that cannot be
/// performed for a multi-package PR and so must never be cleared by one.
#[test]
fn a_grouped_update_is_never_minor_or_patch_however_it_declares_itself() {
    for title in [
        "fix(deps): update all minor dependencies (minor)",
        "fix(deps): update dev dependencies (patch)",
        "fix(deps): update python (non-major)",
    ] {
        assert_eq!(
            classify(title, "").kind,
            BumpKind::NonMajor,
            "a group must not reach the single-package changelog branch: {title}"
        );
    }
}

#[test]
fn a_grouped_major_update_is_still_major() {
    let bump = classify("fix(deps): update all major dependencies (major)", "");
    assert_eq!(bump.kind, BumpKind::Major);
}

// Renovate's own body table, as the feed stores it: 500 characters of the
// PR body, table header included.
#[test]
fn renovate_dotted_target_reads_the_kind_from_the_body_table() {
    let body = "This PR contains the following updates:\n\n\
             | Package | Type | Update | Change |\n|---|---|---|---|\n\
             | [foo](https://example.com) | dependency | minor | `v1.1.0` → `v1.2.3` |";
    let bump = classify("#12 fix(deps): update dependency foo to v1.2.3", body);
    assert_eq!(bump.kind, BumpKind::Minor);
    assert_eq!(bump.package.as_deref(), Some("foo"));
}

#[test]
fn renovate_dotted_target_without_a_table_is_unknown() {
    let bump = classify(
        "#12 fix(deps): update dependency foo to v1.2.3",
        "This PR contains the following updates:",
    );
    assert_eq!(bump.kind, BumpKind::Unknown);
}

#[test]
fn a_table_whose_rows_disagree_yields_no_kind() {
    let body = "| a | dependency | minor | x |\n| b | dependency | major | y |";
    assert_eq!(table_kind(body), None);
}

#[test]
fn prose_saying_major_is_not_a_table_cell() {
    let bump = classify(
        "#3 chore: something else entirely",
        "this is a major rewrite",
    );
    assert_eq!(bump.kind, BumpKind::Unknown);
}

#[test]
fn an_unrecognised_title_is_unknown_not_a_failure() {
    let bump = classify("#3 chore: tidy the workflow", "");
    assert_eq!(bump.kind, BumpKind::Unknown);
    assert_eq!(bump.package, None);
}

// The 0.x rule the runbook always stated, now the only place it lives.
#[test]
fn a_zero_x_minor_move_is_major() {
    assert_eq!(compare_versions("0.9.1", "0.10.0"), BumpKind::Major);
}

#[test]
fn a_zero_x_patch_move_is_patch() {
    assert_eq!(compare_versions("0.9.1", "0.9.2"), BumpKind::Patch);
}

// Dependabot's body wording, which the title regex also matches — and
// which ends in a full stop the version capture must not keep.
#[test]
fn a_version_from_the_body_sentence_loses_its_full_stop() {
    let bump = classify(
        "#25 something the title regex misses",
        "Bumps [dbt-common](https://github.com/dbt-labs/dbt-common) from 1.37.2 to 1.37.3.",
    );
    assert_eq!(bump.kind, BumpKind::Patch);
    assert_eq!(bump.to.as_deref(), Some("1.37.3"));
}

#[test]
fn a_stable_minor_move_is_minor() {
    assert_eq!(compare_versions("1.9.1", "1.10.0"), BumpKind::Minor);
}

#[test]
fn a_prerelease_suffix_does_not_break_comparison() {
    assert_eq!(compare_versions("1.2.3", "2.0.0-rc1"), BumpKind::Major);
}

#[test]
fn the_unknown_prompt_line_says_the_harness_looked() {
    assert_eq!(
        Bump::unknown().prompt_line(),
        "Bump: kind could not be read from the PR title or body."
    );
}

#[test]
fn the_prompt_line_carries_both_versions_when_the_title_did() {
    let bump = classify("Bump serde from 1.0.195 to 1.0.197", "");
    assert_eq!(bump.prompt_line(), "Bump: patch — serde 1.0.195 → 1.0.197");
}

#[test]
fn the_prompt_line_omits_a_source_version_the_title_never_carried() {
    let bump = classify("fix(deps): update dependency deepdiff to v9", "");
    assert_eq!(bump.prompt_line(), "Bump: major — deepdiff → v9");
}

#[test]
fn a_grouped_prompt_line_names_the_group_and_no_versions() {
    let bump = classify("fix(deps): update python (non-major)", "");
    assert_eq!(bump.prompt_line(), "Bump: non-major group — python");
}

// -- Renovate's Change-cell version pair (task #4728) --
//
// The bodies below are the shape the live board actually carries: the
// `| Package | Change | Age | Confidence |` table Renovate emits for a
// package update, which has NO Update column and so no `| major |` cell
// for TABLE_CELL_RE to find. What it does carry is the version pair.

/// The case the classifier used to defer to a kind cell that this table
/// shape never has: a dotted single-package target.
#[test]
fn a_change_cell_pair_settles_a_dotted_target() {
    let body = "This PR contains the following updates:\n\n\
             | Package | Change | Age | Confidence |\n|---|---|---|---|\n\
             | [foo](https://redirect.github.com/foo/foo) | `==1.1.0` → `==1.2.3` | x | y |";
    let bump = classify("#12 fix(deps): update dependency foo to v1.2.3", body);
    assert_eq!(bump.kind, BumpKind::Minor);
    assert_eq!(bump.package.as_deref(), Some("foo"));
    assert_eq!(bump.from.as_deref(), Some("1.1.0"));
    assert_eq!(bump.to.as_deref(), Some("1.2.3"));
}

/// The pair outranks the bare major target rather than merely filling in
/// for it. Both agree on the kind, but only the pair names the source
/// version, so the rendered Bump line stops saying just "→ v9".
#[test]
fn a_change_cell_pair_outranks_a_bare_major_title() {
    let body = "This PR contains the following updates:\n\n\
             | Package | Change | Age | Confidence |\n|---|---|---|---|\n\
             | [deepdiff](https://redirect.github.com/qlustered/deepdiff) | \
             `==8.6.2` → `==9.1.0` | x | y |";
    let bump = classify("#47 fix(deps): update dependency deepdiff to v9", body);
    assert_eq!(bump.kind, BumpKind::Major);
    assert_eq!(bump.package.as_deref(), Some("deepdiff"));
    assert_eq!(bump.from.as_deref(), Some("8.6.2"));
    assert_eq!(
        bump.prompt_line(),
        "Bump: major — deepdiff 8.6.2 → 9.1.0",
        "the pair exists to put the source version on the Bump line"
    );
}

/// A version pair belongs to exactly one row, so several rows mean no pair
/// is "the" bump. The title's own bare target still settles the kind — the
/// pair rule declines, it does not poison the result.
#[test]
fn two_change_cell_pairs_decline_and_the_title_still_settles_it() {
    let body = "| Package | Change | Age | Confidence |\n|---|---|---|---|\n\
             | [foo](x) | `==1.1.0` → `==1.2.3` | x | y |\n\
             | [bar](y) | `==4.0.0` → `==4.0.1` | x | y |";
    let bump = classify("#12 fix(deps): update dependency foo to v9", body);
    assert_eq!(
        bump.kind,
        BumpKind::Major,
        "from the bare target, not a row"
    );
    assert_eq!(
        bump.from, None,
        "no single row's source version may be reported for a multi-row body"
    );
}

/// The 500-character slice can cut a group's table after its first row, so
/// a truncated group body presents exactly one pair and reads as
/// single-package. The title is the part of a group that is never
/// truncated, and it must keep the group out of the changelog branch. This
/// body is PR #79's, sliced the way the feed slices it.
#[test]
fn a_grouped_title_wins_over_a_truncated_group_body_showing_one_pair() {
    let body = "This PR contains the following updates:\n\n\
             | Package | Change | Age | Confidence |\n|---|---|---|---|\n\
             | [google-cloud-storage](x) | `==1.83.0` → `==1.83.1` | x | y |";
    let bump = classify("#79 fix(deps): update python (non-major)", body);
    assert_eq!(
        bump.kind,
        BumpKind::NonMajor,
        "a truncated group must not read as a single patch bump"
    );
    assert_eq!(bump.package.as_deref(), Some("python"));
}

/// Renovate renders the arrow as an ASCII `->` in some configurations and
/// as U+2192 in others. Both are the same cell.
#[test]
fn an_ascii_arrow_reads_like_the_unicode_one() {
    let body = "| [foo](x) | `==1.1.0` -> `==1.2.3` | x | y |";
    let bump = classify("#12 fix(deps): update dependency foo to v1.2.3", body);
    assert_eq!(bump.kind, BumpKind::Minor);
    assert_eq!(bump.from.as_deref(), Some("1.1.0"));
}

/// The pair is read for a Renovate single-package title, not on its own. A
/// body whose title matched nothing keeps going to the user: the pair says
/// which versions moved, not that this task is a dependency bump at all.
#[test]
fn a_change_cell_pair_alone_does_not_classify_an_unrecognised_title() {
    let body = "| [foo](x) | `==1.1.0` → `==1.2.3` | x | y |";
    assert_eq!(
        classify("#3 chore: tidy the workflow", body).kind,
        BumpKind::Unknown
    );
}

/// A pair outside the table is not the update table. Requiring the row
/// keeps release-notes prose — which is most of what the body holds — from
/// being read as a version move.
#[test]
fn a_version_pair_in_prose_is_not_the_update_table() {
    let body = "### Release Notes\n\nUpgrading `==1.1.0` → `==1.2.3` needs a config change.";
    let bump = classify("#12 fix(deps): update dependency foo to v1.2.3", body);
    assert_eq!(bump.kind, BumpKind::Unknown);
}

/// The one live body shape that DOES carry a kind cell is the
/// action-datasource table. It still classifies, and now reports both
/// versions from the same row.
#[test]
fn the_action_table_still_classifies_and_now_carries_both_versions() {
    let body = "This PR contains the following updates:\n\n\
             | Package | Type | Update | Change |\n|---|---|---|---|\n\
             | [actions/checkout](https://redirect.github.com/actions/checkout) | action \
             | major | `v6.1.0` → `v7.0.1` |";
    let bump = classify(
        "#64 chore(deps): update actions/checkout action to v7",
        body,
    );
    assert_eq!(bump.kind, BumpKind::Major);
    assert_eq!(bump.package.as_deref(), Some("actions/checkout"));
    assert_eq!(bump.from.as_deref(), Some("6.1.0"));
    assert_eq!(bump.to.as_deref(), Some("7.0.1"));
}

// -- Renovate's digest updates (task #4708) --
//
// The bodies below are verbatim from the live board. A digest update moves
// the image a tag resolves to without moving the tag, so there is no
// version pair and no semver kind — every one of them read as
// unclassified before this kind existed.

#[test]
fn a_digest_row_classifies_as_a_digest_update() {
    let body = "This PR contains the following updates:\n\n\
             | Package | Type | Update | Change |\n|---|---|---|---|\n\
             | gcr.io/distroless/java25-debian13 | final | digest | \
             `34f47c4` → `19c68cb` |";
    let bump = classify(
        "#181 fix(deps): update gcr.io/distroless/java25-debian13 docker digest to 19c68cb",
        body,
    );
    assert_eq!(bump.kind, BumpKind::Digest);
    assert_eq!(
        bump.package.as_deref(),
        Some("gcr.io/distroless/java25-debian13")
    );
    assert_eq!(bump.from.as_deref(), Some("34f47c4"));
    assert_eq!(bump.to.as_deref(), Some("19c68cb"));
}

/// A digest is hex, so it can begin with a letter. The version-pair regex
/// requires a leading digit and would miss this row entirely.
#[test]
fn a_digest_beginning_with_a_letter_is_still_read() {
    let body = "| [amacneil/dbmate](https://redirect.github.com/amacneil/dbmate) \
             | final | digest | `e550994` → `32d88af` |";
    let bump = classify(
        "#46 fix(deps): update amacneil/dbmate:2.35 docker digest to 32d88af",
        body,
    );
    assert_eq!(bump.kind, BumpKind::Digest);
    assert_eq!(bump.package.as_deref(), Some("amacneil/dbmate"));
    assert_eq!(bump.from.as_deref(), Some("e550994"));
}

/// Renovate's FIRST pin of an unpinned tag: update cell `pinDigest`, and a
/// Change cell with an after digest and no before one. The title names no
/// package at all, so the row is the only place it can come from.
#[test]
fn a_pin_digest_row_is_the_same_kind_with_no_source() {
    let body = "This PR contains the following updates:\n\n\
             | Package | Update | Change |\n|---|---|---|\n\
             | [apache/airflow](https://airflow.apache.org) \
             ([source](https://redirect.github.com/apache/airflow)) | pinDigest |  → `3cbaa47` |";
    let bump = classify("#478 chore(deps): pin dependencies", body);
    assert_eq!(bump.kind, BumpKind::Digest);
    assert_eq!(bump.package.as_deref(), Some("apache/airflow"));
    assert_eq!(bump.from, None, "a first pin has no before digest");
    assert_eq!(bump.to.as_deref(), Some("3cbaa47"));
}

#[test]
fn the_digest_prompt_line_says_the_tag_did_not_move() {
    let body = "| postgres:18 | final | digest | `34f47c4` → `4ef4dbc` |";
    let bump = classify(
        "#500 fix(deps): update postgres:18 docker digest to 4ef4dbc",
        body,
    );
    assert_eq!(
        bump.prompt_line(),
        "Bump: digest re-pin — postgres:18 34f47c4 → 4ef4dbc"
    );
}

/// Several digest rows are still a digest update — every row agrees on the
/// kind, exactly as for `table_kind`. What does NOT survive is the package
/// and the pair: those belong to one row, so naming them here would
/// attribute one image's move to the whole PR. This body is PR #478's,
/// which is a two-row pin; Renovate's pin PRs are routinely multi-row.
#[test]
fn several_digest_rows_keep_the_kind_and_name_nothing() {
    let body = "| Package | Update | Change |\n|---|---|---|\n\
             | [apache/airflow](https://airflow.apache.org) \
             ([source](https://redirect.github.com/apache/airflow)) | pinDigest |  → `3cbaa47` |\n\
             | postgres | pinDigest |  → `c2ca909` |";
    let bump = classify("#478 chore(deps): pin dependencies", body);
    assert_eq!(bump.kind, BumpKind::Digest);
    assert_eq!(bump.package, None, "no one row's image is the PR's");
    assert_eq!(bump.from, None);
    assert_eq!(bump.to, None);
    assert_eq!(bump.prompt_line(), "Bump: digest re-pin");
}

/// A table carrying both a digest row and a semver kind has rows that
/// DISAGREE, so it classifies as neither and goes to the user. Reading it
/// as the semver kind would hand a two-package PR to the single-package
/// changelog branch, which asks for "the" changelog and merges when it
/// comes back clean — the trap the grouped rule exists to avoid.
#[test]
fn a_table_mixing_a_digest_row_with_a_semver_kind_routes_to_neither() {
    let body = "| [foo](x) | final | digest | `aaa1111` → `bbb2222` |\n\
             | [bar](y) | dependency | minor | `1.1.0` → `1.2.0` |";
    assert_eq!(
        classify("#3 chore(deps): something", body).kind,
        BumpKind::Unknown
    );
}

/// PR #215's body, verbatim from the live board, and the case that makes
/// the unanimity rule worth having. Three rows — two `minor` and one
/// `pinDigest` — across three artifacts, moving a Python runtime from
/// 3.11 to 3.14. It used to classify as Minor, which renders a Bump line
/// naming no package and routes to the changelog branch, where a clean
/// changelog earns an auto-merge. Rows that disagree must reach the user.
#[test]
fn the_live_python_runtime_pr_is_no_longer_auto_mergeable_as_minor() {
    let body = "This PR contains the following updates:\n\n\
             | Package | Type | Update | Change |\n|---|---|---|---|\n\
             | eu.gcr.io/annotell-com/python-base/builder | stage | minor | \
             `3.11-bookworm-slim` → `3.14-bookworm-slim` |\n\
             | eu.gcr.io/annotell-com/python-base/runner | final | pinDigest |  → `e11de72` |\n\
             | [python](https://python.org) ([source](https://redirect.github.com/python/cpython)) \
             | requires-python | minor | `>=3.11,<3.12` → `>=3.14,<3.15` |";
    assert_eq!(
        classify("#215 chore(deps): update python runtime", body).kind,
        BumpKind::Unknown,
        "a table whose rows disagree must not reach the auto-merge branch"
    );
}

/// Renovate's github-actions digest form, which — unlike the docker form —
/// DOES match the Renovate single-package title rule: `11bd719` is
/// dot-free, so the bare-major branch would have called it a major bump of
/// actions/checkout. Two guards stop that, and this pins both: the body's
/// digest row is read before the title branch is reached, and the bare
/// major now requires an all-digit target.
#[test]
fn an_actions_digest_title_is_a_digest_not_a_major() {
    let body = "| Package | Type | Update | Change |\n|---|---|---|---|\n\
             | [actions/checkout](https://redirect.github.com/actions/checkout) \
             | action | digest | `08c6903` → `11bd719` |";
    let bump = classify(
        "#64 chore(deps): update actions/checkout digest to 11bd719",
        body,
    );
    assert_eq!(bump.kind, BumpKind::Digest);
    assert_eq!(bump.package.as_deref(), Some("actions/checkout"));
    assert_eq!(bump.from.as_deref(), Some("08c6903"));
}

/// The same title with a body the slice destroyed. The all-digit rule is
/// the guard that still holds, and "cannot be read" is the honest answer —
/// "major" would not be.
#[test]
fn an_actions_digest_title_alone_is_never_reported_as_major() {
    let bump = classify(
        "#64 chore(deps): update actions/checkout digest to 11bd719",
        "",
    );
    assert_eq!(bump.kind, BumpKind::Unknown);
}

/// The bare-major rule still fires for every version-shaped target the
/// live board actually carries.
#[test]
fn a_version_shaped_bare_target_is_still_major() {
    for (title, pkg) in [
        ("fix(deps): update dependency deepdiff to v9", "deepdiff"),
        ("fix(deps): update dependency pytz to v2026", "pytz"),
        (
            "fix(deps): update dependency com.kognic.otel:kognic-otel-bom to v339",
            "com.kognic.otel:kognic-otel-bom",
        ),
    ] {
        let bump = classify(title, "");
        assert_eq!(bump.kind, BumpKind::Major, "{title}");
        assert_eq!(bump.package.as_deref(), Some(pkg));
    }
}

/// The grouped title still short-circuits first, which is what covers a
/// group whose table the 500-character slice cut down to one digest row.
#[test]
fn a_grouped_title_wins_over_a_lone_digest_row() {
    let body = "| [foo](x) | final | digest | `aaa1111` → `bbb2222` |";
    let bump = classify("#79 fix(deps): update python (non-major)", body);
    assert_eq!(bump.kind, BumpKind::NonMajor);
}

/// The KIND comes from the cell, not the pair, so a digest row whose
/// Change cell did not survive the 500-character slice still classifies —
/// it just names no digests. Declining here would make one unreadable row
/// behave differently from two, which nothing justifies.
#[test]
fn a_digest_row_with_no_readable_pair_still_classifies() {
    let body = "| Package | Type | Update | Change |\n|---|---|---|---|\n\
             | [amacneil/dbmate](https://redirect.github.com/amacneil/dbmate) | final | digest |";
    let bump = classify(
        "#46 fix(deps): update amacneil/dbmate:2.35 docker digest",
        body,
    );
    assert_eq!(bump.kind, BumpKind::Digest);
    assert_eq!(bump.package.as_deref(), Some("amacneil/dbmate"));
    assert_eq!(bump.to, None);
    assert_eq!(bump.prompt_line(), "Bump: digest re-pin — amacneil/dbmate");
}

/// The word has to fill a whole cell, exactly as for the kind cell —
/// including inside a table row, which is the case plain prose does not
/// exercise. Release notes sit under every Renovate table and say
/// "digest" freely.
#[test]
fn the_word_digest_must_fill_a_whole_cell() {
    for body in [
        "We now pin by digest rather than by tag.",
        "| foo | pinned by digest today | `aaa1111` → `bbb2222` |",
    ] {
        assert_eq!(
            classify("#3 chore: tidy the workflow", body).kind,
            BumpKind::Unknown,
            "not an update table: {body}"
        );
    }
}

/// Regression guard: an ordinary version bump must be untouched by the
/// digest reading, which runs before the version pair.
#[test]
fn a_version_pair_update_is_unaffected_by_the_digest_rule() {
    let body = "| Package | Change | Age | Confidence |\n|---|---|---|---|\n\
             | [foo](x) | `==1.1.0` → `==1.2.3` | x | y |";
    let bump = classify("#12 fix(deps): update dependency foo to v1.2.3", body);
    assert_eq!(bump.kind, BumpKind::Minor);
    assert_eq!(bump.from.as_deref(), Some("1.1.0"));
}

/// Dependabot's title still wins outright: its own from/to is on the title,
/// which no slice can truncate, and its body carries no Renovate table.
#[test]
fn a_dependabot_title_is_unaffected_by_the_pair_rule() {
    let bump = classify(
        "#25 Bump dbt-common from 1.37.2 to 1.37.3 in /kognic-airflow/venvs/dbt",
        "| [dbt-common](x) | `==9.0.0` → `==1.0.0` | x | y |",
    );
    assert_eq!(bump.kind, BumpKind::Patch);
    assert_eq!(bump.from.as_deref(), Some("1.37.2"));
}

// == The three properties every classification has, whatever it read ==
//
// The cases above each pin one title or body shape. These pin what holds
// across all of them, which is what `Bump`'s two invariants say in
// docs/specs/dispatch-prompt.allium. A new reading step that satisfies its
// own case but breaks one of these renders a Bump line no branch has a
// shape for.

/// Titles and bodies shaped like the ones the two bots actually write, so
/// the properties below reach the classified kinds and not only `Unknown`.
/// Free-form text alone classifies as unknown almost every time, which
/// would leave both properties vacuously true.
///
/// `every_generator_arm_classifies_as_the_shape_it_was_written_for` below
/// is what keeps that from happening quietly: one literal input per arm
/// here, with the kind it must produce.
fn a_bot_input() -> impl Strategy<Value = (String, String)> {
    let ver = "(0|[1-9][0-9]{0,2})\\.(0|[1-9][0-9]{0,2})\\.(0|[1-9][0-9]{0,2})";
    let pkg = "[a-z][a-z0-9-]{0,12}";
    prop_oneof![
        // Dependabot: both versions on the title.
        (pkg, ver, ver).prop_map(|(p, f, t)| (format!("Bump {p} from {f} to {t}"), String::new())),
        // Dependabot: both versions in the body sentence.
        (pkg, ver, ver).prop_map(|(p, f, t)| (
            "chore(deps)".to_string(),
            format!("Bumps [{p}] from {f} to {t}.")
        )),
        // Renovate: a bare target on the title, no source anywhere.
        (pkg, "[1-9][0-9]{0,2}").prop_map(|(p, n)| (
            format!("fix(deps): update dependency {p} to v{n}"),
            String::new()
        )),
        // Renovate: a version pair in the update table.
        (pkg, ver, ver).prop_map(|(p, f, t)| (
            format!("fix(deps): update dependency {p} to v{t}"),
            format!("| [{p}](x) | `=={f}` \u{2192} `=={t}` | x | y |"),
        )),
        // Renovate: a grouped update, which names a package and no version.
        (pkg, "major|minor|patch").prop_map(|(p, k)| (
            format!("chore(deps): update {p} monorepo ({k})"),
            String::new()
        )),
        // Renovate: a digest row, which names an image and two digests.
        (pkg, "[0-9a-f]{7,12}", "[0-9a-f]{7,12}").prop_map(|(p, a, b)| (
            format!("chore(deps): update {p} docker digest to {b}"),
            format!("| [{p}](x) | digest | `{a}` \u{2192} `{b}` |"),
        )),
        // And arbitrary text, which is the unknown arm.
        ("\\PC{0,120}", "\\PC{0,300}"),
    ]
}

proptest! {
    /// `AnUnreadableBumpNamesNothing`. The unknown kind is the one with
    /// nothing to append: its line says the whole sentence itself, so a
    /// package or a version arriving beside it would be rendered nowhere
    /// and silently lost.
    #[test]
    fn an_unreadable_bump_names_nothing((title, body) in a_bot_input()) {
        let bump = classify(&title, &body);
        if bump.kind == BumpKind::Unknown {
            prop_assert_eq!(bump.package, None);
            prop_assert_eq!(bump.from, None);
            prop_assert_eq!(bump.to, None);
        }
    }

    /// `ASourceVersionNeedsATarget`. `prompt_line` gates both versions on
    /// `to`, so a source without a target is a version the line drops.
    /// No reading step constructs that shape; this is what keeps it so.
    #[test]
    fn a_source_version_never_arrives_without_a_target((title, body) in a_bot_input()) {
        let bump = classify(&title, &body);
        prop_assert!(
            bump.from.is_none() || bump.to.is_some(),
            "from without to: {bump:?}"
        );
    }

}

/// One literal input per arm of `a_bot_input`, with the whole `Bump` each
/// must produce. Two jobs in one table.
///
/// It keeps the properties above honest: they say nothing useful if the
/// generator stops reaching the classified kinds, and free-form text
/// classifies as `Unknown` almost every time. Driving the strategy itself
/// and asserting every kind appeared was tried and is worse — it passes as
/// long as SOME arm still reaches each kind, so five of the seven could rot
/// in silence, and `Minor` turned out to be a 0.2% draw, so it only ever
/// passed on proptest's fixed seed.
///
/// It is also the one place a whole `Bump` is compared rather than field by
/// field, which is what pins the four fields as a unit.
#[test]
fn every_generator_arm_classifies_as_the_shape_it_was_written_for() {
    let bump = |kind, package: Option<&str>, from: Option<&str>, to: Option<&str>| Bump {
        kind,
        package: package.map(str::to_string),
        from: from.map(str::to_string),
        to: to.map(str::to_string),
    };
    let rows = [
        (
            "dependabot title",
            "Bump requests from 1.2.3 to 1.3.0",
            "",
            bump(
                BumpKind::Minor,
                Some("requests"),
                Some("1.2.3"),
                Some("1.3.0"),
            ),
        ),
        (
            "dependabot body sentence",
            "chore(deps)",
            "Bumps [requests] from 1.2.3 to 1.2.4.",
            bump(
                BumpKind::Patch,
                Some("requests"),
                Some("1.2.3"),
                Some("1.2.4"),
            ),
        ),
        (
            "renovate bare target",
            "fix(deps): update dependency deepdiff to v9",
            "",
            bump(BumpKind::Major, Some("deepdiff"), None, Some("v9")),
        ),
        (
            "renovate table pair",
            "fix(deps): update dependency deepdiff to v9.1.0",
            "| [deepdiff](x) | `==8.6.2` \u{2192} `==9.1.0` | x | y |",
            bump(
                BumpKind::Major,
                Some("deepdiff"),
                Some("8.6.2"),
                Some("9.1.0"),
            ),
        ),
        (
            "renovate group",
            "chore(deps): update airflow monorepo (minor)",
            "",
            bump(BumpKind::NonMajor, Some("airflow monorepo"), None, None),
        ),
        (
            "renovate digest row",
            "chore(deps): update postgres docker digest to 4ef4dbc",
            "| [postgres](x) | digest | `1a2b3c4` \u{2192} `4ef4dbc` |",
            bump(
                BumpKind::Digest,
                Some("postgres"),
                Some("1a2b3c4"),
                Some("4ef4dbc"),
            ),
        ),
        (
            "free-form text",
            "chore: tidy the workflow",
            "",
            bump(BumpKind::Unknown, None, None, None),
        ),
    ];

    for (arm, title, body, want) in &rows {
        assert_eq!(&classify(title, body), want, "generator arm: {arm}");
    }

    for kind in BumpKind::ALL {
        assert!(
            rows.iter().any(|(_, _, _, want)| want.kind == *kind),
            "no generator arm reaches {kind:?}"
        );
    }
}
