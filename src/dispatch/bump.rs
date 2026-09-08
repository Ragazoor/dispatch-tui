//! Classifying a dependency-update PR from what the task already carries.
//!
//! The dependabot runbook used to open with a parsing step: read
//! `Bump <pkg> from <X.Y.Z> to <A.B.C>` out of the PR title, compare the two
//! versions as semver, and pick a branch. That is a function of its inputs, and
//! both inputs — the PR title and the truncated PR body — are already on the
//! task before the prompt is built. So it runs here, and the prompt is rendered
//! around the answer rather than asking for it. See
//! `AReviewRunbookCarriesOnlyTheBranchThatApplies` in `docs/specs/dispatch.allium`.
//!
//! Two bots feed this queue and they title their PRs differently. Dependabot
//! writes `Bump requests from 2.32.4 to 2.33.0`; Renovate writes
//! `fix(deps): update dependency deepdiff to v9` or, for a grouped update,
//! `fix(deps): update python (non-major)`. The old runbook recognised only the
//! first form, so on a real board where Renovate authors most of the queue,
//! nearly every bump fell through to "ask the user".
//!
//! Classification is best-effort on purpose. [`BumpKind::Unknown`] is a
//! routable answer rather than a failure — it renders the ask-the-user branch,
//! which is exactly where an unrecognised bump ended up before.
//!
//! The feed truncates a PR body to 500 characters, and that is deliberately
//! not treated as a defect to work around: measured against the 15 open bot
//! PRs on epic 275, the version pair survives the slice in 13 of 13 Renovate
//! bodies, and the two Dependabot PRs state theirs in the never-truncated
//! title. What the slice removes is the release notes below the table, which
//! nothing here reads. Task #4728 considered having the feed DECLARE the kind
//! instead and rejected it — see
//! `AReviewRunbookCarriesOnlyTheBranchThatApplies` in
//! `docs/specs/dispatch.allium` for why.

use std::sync::LazyLock;

use regex::Regex;

/// What kind of dependency update this PR is, as far as the title and body say.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum BumpKind {
    Patch,
    Minor,
    Major,
    /// A Renovate grouped update declared non-major, covering more than one
    /// package. Not routable as a minor bump: the changelog branch is written
    /// for a single package and there is no one changelog to read.
    NonMajor,
    /// Neither the title nor the body said, or they said something this does
    /// not recognise.
    Unknown,
}

/// A classified dependency bump: the kind, plus whatever the title happened to
/// name. `package`, `from` and `to` are each reported only when actually read —
/// a Renovate major title carries a package and a target but no source version,
/// and a grouped title carries neither version.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Bump {
    pub(super) kind: BumpKind,
    // Read only by `prompt_line` and this module's tests. Private on purpose:
    // nothing outside needs the parts once the line is rendered.
    package: Option<String>,
    from: Option<String>,
    to: Option<String>,
}

impl Bump {
    fn unknown() -> Self {
        Bump {
            kind: BumpKind::Unknown,
            package: None,
            from: None,
            to: None,
        }
    }

    /// The one line the rendered prompt carries above its steps.
    ///
    /// Written so the ask-the-user branch can quote it verbatim: an agent that
    /// cannot route the bump still has to tell the user what it saw, and the
    /// unknown wording says the harness looked rather than that nothing was
    /// there.
    pub(super) fn prompt_line(&self) -> String {
        let kind = match self.kind {
            BumpKind::Patch => "patch",
            BumpKind::Minor => "minor",
            BumpKind::Major => "major",
            BumpKind::NonMajor => "non-major group",
            // The only kind with no package or versions to append, so it says
            // the whole sentence itself rather than leaving "Bump: unknown".
            BumpKind::Unknown => {
                return "Bump: kind could not be read from the PR title or body.".to_string()
            }
        };
        let mut line = format!("Bump: {kind}");
        if let Some(pkg) = &self.package {
            line.push_str(" — ");
            line.push_str(pkg);
        }
        // A target with no source is Renovate's shape; a source with no target
        // is nothing any branch above constructs, so `to` gates both.
        if let Some(to) = &self.to {
            match &self.from {
                Some(from) => line.push_str(&format!(" {from} → {to}")),
                None => line.push_str(&format!(" → {to}")),
            }
        }
        line
    }
}

// Dependabot's title form. Anchored on ` from ` / ` to ` around two
// version-shaped tokens, so the trailing ` in /some/path` a monorepo bump
// carries is simply not matched rather than needing to be stripped.
static DEPENDABOT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bbumps?\s+\[?([^\]\s]+)\]?(?:\([^)]*\))?\s+from\s+v?([0-9][^\s]*)\s+to\s+v?([0-9][^\s]*)")
        .unwrap_or_else(|e| unreachable!("DEPENDABOT_RE is a hardcoded pattern: {e}"))
});

/// Strip the sentence punctuation a version picks up when the match came from
/// the PR body rather than the title — Dependabot writes "Bumps [foo] from
/// 1.2.3 to 1.2.4." with a full stop the version regex would otherwise keep.
/// Trailing, never leading: a version's own dots are load-bearing.
fn trim_version(v: &str) -> String {
    v.trim_end_matches(['.', ',', ')']).to_string()
}

// Renovate's single-package form: `update dependency <pkg> to v9`,
// `update <pkg> action to v7`, `update <pkg> to v1.2.3`. The optional noun
// before `to` is Renovate's datasource word (action, docker, image, …).
static RENOVATE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bupdate\s+(?:dependency\s+)?([^\s]+)(?:\s+[a-z]+)?\s+to\s+v?([0-9][^\s]*)")
        .unwrap_or_else(|e| unreachable!("RENOVATE_RE is a hardcoded pattern: {e}"))
});

// Renovate's grouped form: `update python (non-major)`, `update all minor
// dependencies (minor)`. The parenthesised word is the group's declared kind.
static RENOVATE_GROUP_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bupdate\s+(.+?)\s*\((non-major|major|minor|patch)\)")
        .unwrap_or_else(|e| unreachable!("RENOVATE_GROUP_RE is a hardcoded pattern: {e}"))
});

// A lone `major` / `minor` / `patch` cell in Renovate's update table. Requiring
// the word to fill a whole cell is what keeps prose out: a body that merely
// says "this is a major rewrite" has no `| major |` in it.
//
// This cell only exists in the `| Package | Type | Update | Change |` table
// Renovate emits for the ACTION datasource. A package update emits
// `| Package | Change | Age | Confidence |`, which has no Update column at
// all — so for most of the queue the cell is absent by construction, not by
// truncation, and `CHANGE_CELL_RE` below is what actually reads those bodies.
static TABLE_CELL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\s*(major|minor|patch)\s*\|")
        .unwrap_or_else(|e| unreachable!("TABLE_CELL_RE is a hardcoded pattern: {e}"))
});

// The version pair in a Renovate update table's Change cell:
// `` `==8.6.2` → `==9.1.0` ``. Each side is a whole backticked token, so the
// constraint operator Renovate prefixes (`==`, `^`, `v`) is skipped rather
// than parsed, and a prerelease suffix is kept for `component` to ignore. The
// arrow is written ASCII in some Renovate configurations and U+2192 in others.
static CHANGE_CELL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"`[^`\n]*?([0-9][^`\s]*)`\s*(?:->|→)\s*`[^`\n]*?([0-9][^`\s]*)`")
        .unwrap_or_else(|e| unreachable!("CHANGE_CELL_RE is a hardcoded pattern: {e}"))
});

/// Classify the bump this task is about, from its title and description.
///
/// Pure, and deliberately so: it runs on the dispatch path, where a network
/// call would block the board. Everything it reads was fetched by the feed.
pub(super) fn classify(title: &str, description: &str) -> Bump {
    if let Some(bump) = dependabot_form(title) {
        return bump;
    }

    // A grouped update is checked before the single-package form: the group
    // title also contains ` update `, and reading a group's name as a package
    // would report a package that does not exist.
    if let Some(caps) = RENOVATE_GROUP_RE.captures(title) {
        // Only `major` earns its own kind. Everything else a group declares
        // becomes NonMajor, INCLUDING `(minor)` and `(patch)` — mapping those
        // to Minor would route a multi-package PR into the changelog branch,
        // which asks for "the" changelog and merges when it comes back clean.
        // There is no one changelog for a group, so that branch would clear a
        // check it never performed. Non-major or not, a group goes to the user.
        let kind = if caps[2].eq_ignore_ascii_case("major") {
            BumpKind::Major
        } else {
            BumpKind::NonMajor
        };
        return Bump {
            kind,
            package: Some(caps[1].trim().to_string()),
            from: None,
            to: None,
        };
    }

    if let Some(caps) = RENOVATE_RE.captures(title) {
        // The body's Change cell is read BEFORE the title's own target. For a
        // title carrying a bare major the two agree, but only the cell names
        // the SOURCE version, so the Bump line reads "deepdiff 8.6.2 → 9.1.0"
        // rather than "deepdiff → v9". The target below is what settles a PR
        // whose body did not survive, not the primary reading.
        if let Some((from, to)) = change_cell_pair(description) {
            return Bump {
                kind: compare_versions(&from, &to),
                package: Some(caps[1].to_string()),
                from: Some(from),
                to: Some(to),
            };
        }
        let target = caps[2].to_string();
        // Renovate writes a bare major (`to v9`) only when the whole constraint
        // moves to a new major. A dotted target says nothing on its own, so it
        // defers to the body's table.
        // `table_kind` is read here and at the fallback below, and both are
        // reached only when the branches above declined — on the live board
        // that is 2 of 15 PRs. So it is computed at each use rather than once
        // up front, which would scan the body for every dispatch to throw the
        // answer away. The dotted-target-with-no-readable-table path does fall
        // through and scan twice; it is the rarest input and still less total
        // work.
        let kind = if target.contains('.') {
            table_kind(description)
        } else {
            Some(BumpKind::Major)
        };
        // A dotted target the table could not settle falls through to the
        // Dependabot sentence below rather than short-circuiting to Unknown —
        // that sentence is the one other place a version pair can be stated.
        if let Some(kind) = kind {
            return Bump {
                kind,
                package: Some(caps[1].to_string()),
                from: None,
                to: Some(format!("v{target}")),
            };
        }
    }

    // The body is read last, and in the same order: Dependabot states the pair
    // in a sentence ("Bumps [foo] from 1.2.3 to 1.2.4."), Renovate in a table.
    if let Some(bump) = dependabot_form(description) {
        return bump;
    }

    match table_kind(description) {
        Some(kind) => Bump {
            kind,
            package: None,
            from: None,
            to: None,
        },
        None => Bump::unknown(),
    }
}

/// Dependabot's `Bump <pkg> from <X> to <Y>` shape, wherever it appears. Run
/// over the title first and the body second, because both carry it and the
/// title is the one guaranteed not to be truncated.
fn dependabot_form(text: &str) -> Option<Bump> {
    let caps = DEPENDABOT_RE.captures(text)?;
    let from = trim_version(&caps[2]);
    let to = trim_version(&caps[3]);
    Some(Bump {
        kind: compare_versions(&from, &to),
        package: Some(caps[1].to_string()),
        from: Some(from),
        to: Some(to),
    })
}

/// The single version pair Renovate's update table states, when the body
/// carries exactly one.
///
/// Only table ROWS are scanned. The release notes below the table are most of
/// what a body holds and they quote versions freely, so a pair found in prose
/// is not a version move.
///
/// Exactly one is required, and several decline rather than picking the first.
/// A pair belongs to exactly one row, so reporting it for a multi-row body
/// would attribute one package's versions to the whole PR. That is stricter
/// than `table_kind`, which refuses only rows that DISAGREE: a kind every row
/// agrees on is still that PR's kind.
///
/// The count is not a sufficient group guard on its own — the 500-character
/// slice can cut a group's table after its first row, leaving exactly one pair
/// — which is why `classify` checks the grouped TITLE before reaching here.
fn change_cell_pair(description: &str) -> Option<(String, String)> {
    let mut pairs = description
        .lines()
        .filter(|line| line.trim_start().starts_with('|'))
        .flat_map(|line| CHANGE_CELL_RE.captures_iter(line))
        .map(|caps| (caps[1].to_string(), caps[2].to_string()));
    let first = pairs.next()?;
    // Short-circuits on the second match rather than counting them all.
    pairs.next().is_none().then_some(first)
}

/// The kind Renovate's update table declares, when the body carries exactly
/// one. Two different kinds in one table is a grouped PR whose rows disagree —
/// no single answer routes it, so it gets none.
fn table_kind(description: &str) -> Option<BumpKind> {
    let mut found: Option<BumpKind> = None;
    for caps in TABLE_CELL_RE.captures_iter(description) {
        let kind = match &caps[1].to_ascii_lowercase()[..] {
            "major" => BumpKind::Major,
            "minor" => BumpKind::Minor,
            _ => BumpKind::Patch,
        };
        match found {
            Some(seen) if seen != kind => return None,
            _ => found = Some(kind),
        }
    }
    found
}

/// The leading integer of a version component, ignoring any suffix — `3` from
/// `3-rc1`, `0` from `0+build2`. A component with no leading digit reads as 0,
/// which makes a malformed version compare equal rather than panic.
fn component(v: &str, index: usize) -> u64 {
    v.split('.')
        .nth(index)
        .map(|part| {
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().unwrap_or(0)
        })
        .unwrap_or(0)
}

/// Semver comparison, with the 0.x rule the runbook always stated: below 1.0.0
/// a minor bump is a breaking change, so `0.9.x → 0.10.x` is major and only a
/// patch move inside the same `0.X` counts as patch. There is no minor outcome
/// under 1.0.0 — that is the point of the rule, not a gap in it.
fn compare_versions(from: &str, to: &str) -> BumpKind {
    let (from_major, to_major) = (component(from, 0), component(to, 0));
    if from_major != to_major {
        return BumpKind::Major;
    }
    let (from_minor, to_minor) = (component(from, 1), component(to, 1));
    if from_minor != to_minor {
        return if from_major == 0 {
            BumpKind::Major
        } else {
            BumpKind::Minor
        };
    }
    BumpKind::Patch
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
