//! Classifying a dependency-update PR from what the task already carries.
//!
//! The dependabot runbook used to open with a parsing step: read
//! `Bump <pkg> from <X.Y.Z> to <A.B.C>` out of the PR title, compare the two
//! versions as semver, and pick a branch. That is a function of its inputs, and
//! both inputs — the PR title and the truncated PR body — are already on the
//! task before the prompt is built. So it runs here, and the prompt is rendered
//! around the answer rather than asking for it. See
//! `AReviewRunbookCarriesOnlyTheBranchThatApplies` in `docs/specs/dispatch-prompt.allium`.
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
//! `docs/specs/dispatch-prompt.allium` for why.

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
    /// The image a tag resolves to moved without the tag moving — Renovate's
    /// `digest` and `pinDigest` updates. No version pair, no semver kind, and
    /// no changelog anywhere that would describe what changed.
    Digest,
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
            BumpKind::Digest => "digest re-pin",
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

// Every word Renovate writes in an update table's Update column, and ONE
// reader for all of them, so the "what if the rows disagree" policy is stated
// once rather than per kind. `major`/`minor`/`patch` declare a version move;
// `digest`/`pinDigest` declare an image move, where the tag stays put and only
// the image behind it changes. Requiring the word to fill a whole cell is what
// keeps prose out: a body that merely says "this is a major rewrite" has no
// `| major |` in it.
//
// The Update column exists only in the `| Package | Type | Update | Change |`
// table Renovate emits for the ACTION datasource, and in the digest tables. A
// package update emits `| Package | Change | Age | Confidence |`, which has no
// Update column at all — so for most of the queue this cell is absent by
// construction, not by truncation, and `CHANGE_CELL_RE` below is what actually
// reads those bodies.
static UPDATE_CELL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\|\s*(major|minor|patch|pindigest|digest)\s*\|")
        .unwrap_or_else(|e| unreachable!("UPDATE_CELL_RE is a hardcoded pattern: {e}"))
});

// The arrow between the two sides of a Change cell, shared by the two readers
// below so a Renovate configuration emitting a third spelling is fixed in one
// place. It is written ASCII in some configurations and U+2192 in others.
const ARROW: &str = r"\s*(?:->|→)\s*";

// The version pair in a Renovate update table's Change cell:
// `` `==8.6.2` → `==9.1.0` ``. Each side is a whole backticked token, so the
// constraint operator Renovate prefixes (`==`, `^`, `v`) is skipped rather
// than parsed, and a prerelease suffix is kept for `component` to ignore.
static CHANGE_CELL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"`[^`\n]*?([0-9][^`\s]*)`{ARROW}`[^`\n]*?([0-9][^`\s]*)`"
    ))
    .unwrap_or_else(|e| unreachable!("CHANGE_CELL_RE is a hardcoded pattern: {e}"))
});

// The digest pair in a digest row's Change cell: `` `34f47c4` → `19c68cb` ``.
// The BEFORE side is optional because a first pin has none — Renovate writes
// `` → `3cbaa47` `` with the left of the arrow empty. Hex, not version-shaped:
// a digest routinely begins with a letter, which `CHANGE_CELL_RE` would skip.
static DIGEST_PAIR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)(?:`([0-9a-f]{{6,}})`)?{ARROW}`([0-9a-f]{{6,}})`"
    ))
    .unwrap_or_else(|e| unreachable!("DIGEST_PAIR_RE is a hardcoded pattern: {e}"))
});

// A markdown link's text, used to read the package out of a table row whose
// first cell links to the source. A bare cell has no link and is taken whole.
static LINK_TEXT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[([^\]]+)\]\(")
        .unwrap_or_else(|e| unreachable!("LINK_TEXT_RE is a hardcoded pattern: {e}"))
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

    // Read before the single-package branches, and title-agnostic like the
    // kind cell: a digest title ("update postgres:18 docker digest to
    // 4ef4dbc") matches no version-shaped rule, and Renovate's first pin
    // ("chore(deps): pin dependencies") names no package in its title at all.
    if let Some(bump) = digest_update(description) {
        return bump;
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
        // moves to a new major. The target has to be VERSION-SHAPED to be read
        // that way — all digits, no dot — and not merely dot-free: an image
        // digest is dot-free too, and `update actions/checkout digest to
        // 11bd719` would otherwise report a major bump of a package that did
        // not change version at all. Anything else defers to the body's table,
        // which is where a digest and a dotted target are both settled.
        let kind = if target.chars().all(|c| c.is_ascii_digit()) {
            Some(BumpKind::Major)
        } else {
            table_kind(description)
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

/// The lines of a body that are update-table ROWS.
///
/// The rule this encodes is load-bearing for every reader below it: the
/// release notes under the table are most of what a body holds and they quote
/// versions and digests freely, so a token found in prose is not a version
/// move. Deliberately loose about what follows the leading pipe — Renovate's
/// own separator row (`|---|---|`) is harmless to scan and cheaper to admit
/// than to exclude.
fn table_rows(description: &str) -> impl Iterator<Item = &str> {
    description
        .lines()
        .filter(|line| line.trim_start().starts_with('|'))
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
    let mut pairs = table_rows(description)
        .flat_map(|line| CHANGE_CELL_RE.captures_iter(line))
        .map(|caps| (caps[1].to_string(), caps[2].to_string()));
    let first = pairs.next()?;
    // Short-circuits on the second match rather than counting them all.
    pairs.next().is_none().then_some(first)
}

/// The digest update Renovate's table declares, when every cell in it agrees
/// that a digest is what moved.
///
/// The unanimity test is [`table_kind`]'s, not a second policy: a table
/// carrying a digest row AND a semver one describes more than an image move,
/// its rows disagree, and no single answer routes it. Asking `table_kind`
/// rather than re-deriving that here is why a mixed table now reaches the
/// user instead of being handed the single-package changelog branch.
///
/// What this adds over the kind alone is the NAMING, and only for a single
/// row: the image and its two digests belong to one row, so reporting them for
/// a multi-row table would attribute one image's move to the whole PR — the
/// same reasoning as [`change_cell_pair`]. An unreadable Change cell costs the
/// digests the same way. Renovate's pin PRs are routinely multi-row, so the
/// unnamed form is the common case for `pinDigest` rather than an edge of it.
///
/// A grouped TITLE short-circuits before this is reached, which is what covers
/// a group whose table the 500-character slice cut down to a digest row.
fn digest_update(description: &str) -> Option<Bump> {
    if table_kind(description) != Some(BumpKind::Digest) {
        return None;
    }
    // Unanimity already established, so every row here is a digest row.
    let mut rows = table_rows(description).filter(|line| UPDATE_CELL_RE.is_match(line));
    let first = rows.next()?;
    let row = rows.next().is_none().then_some(first);
    let caps = row.and_then(|r| DIGEST_PAIR_RE.captures(r));
    let digest = |group| Some(caps.as_ref()?.get(group)?.as_str().to_string());
    Some(Bump {
        kind: BumpKind::Digest,
        package: row.and_then(row_package),
        from: digest(1),
        to: digest(2),
    })
}

/// The package a table row names, read from its first non-empty cell: a
/// markdown link's text where it has one, the bare cell otherwise. Renovate
/// writes both — a linkable package gets `[foo](url)`, a bare image name does
/// not — and a first cell can carry a second `([source](url))` link, so only
/// the FIRST link's text is taken.
fn row_package(row: &str) -> Option<String> {
    let cell = row.split('|').map(str::trim).find(|c| !c.is_empty())?;
    Some(
        LINK_TEXT_RE
            .captures(cell)
            .map_or_else(|| cell.to_string(), |caps| caps[1].to_string()),
    )
}

/// The kind Renovate's update table declares, when every cell in it agrees.
///
/// Cells that DISAGREE yield nothing: that is a grouped PR, and no single
/// answer routes it. Unanimity is enough, though — a kind every cell agrees on
/// is that PR's kind however many rows say it. This is the one disagreement
/// policy in the module, and both a semver table and a digest table are held
/// to it, which is what stops a table mixing the two from being read as
/// either.
fn table_kind(description: &str) -> Option<BumpKind> {
    let mut found: Option<BumpKind> = None;
    for caps in UPDATE_CELL_RE.captures_iter(description) {
        let kind = match &caps[1].to_ascii_lowercase()[..] {
            "major" => BumpKind::Major,
            "minor" => BumpKind::Minor,
            "patch" => BumpKind::Patch,
            _ => BumpKind::Digest,
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
    // across all of them, which is what `Bump`'s two invariants and the
    // `PromptComposer` contract's `ClassificationNeverReachesTheNetwork` say
    // in docs/specs/dispatch-prompt.allium. A new reading step that satisfies
    // its own case but breaks one of these renders a Bump line no branch has
    // a shape for.

    /// Titles and bodies shaped like the ones the two bots actually write, so
    /// the properties below reach the classified kinds and not only `Unknown`.
    /// Free-form text alone classifies as unknown almost every time, which
    /// would leave the two version properties vacuously true.
    fn a_bot_input() -> impl Strategy<Value = (String, String)> {
        let ver = "(0|[1-9][0-9]{0,2})\\.(0|[1-9][0-9]{0,2})\\.(0|[1-9][0-9]{0,2})";
        let pkg = "[a-z][a-z0-9-]{0,12}";
        prop_oneof![
            // Dependabot: both versions on the title.
            (pkg, ver, ver)
                .prop_map(|(p, f, t)| (format!("Bump {p} from {f} to {t}"), String::new())),
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

        /// `ClassificationNeverReachesTheNetwork`, from the observable side:
        /// the title and the description are the whole input, so the same
        /// pair classifies the same way every time. Also the one place a
        /// whole `Bump` is compared rather than field by field.
        #[test]
        fn classification_is_a_pure_function_of_the_title_and_the_body(
            (title, body) in a_bot_input(),
        ) {
            prop_assert_eq!(classify(&title, &body), classify(&title, &body));
        }
    }

    /// The generator above is only useful if it reaches the kinds it is shaped
    /// for. This pins that: a run that stopped producing classified bumps would
    /// leave the two version properties vacuously true and say nothing.
    #[test]
    #[allow(clippy::expect_used)]
    fn the_bot_input_generator_reaches_every_kind() {
        use proptest::strategy::ValueTree;
        use proptest::test_runner::TestRunner;

        let mut runner = TestRunner::deterministic();
        let strategy = a_bot_input();
        let mut seen: Vec<BumpKind> = Vec::new();
        for _ in 0..2000 {
            let (title, body) = strategy
                .new_tree(&mut runner)
                .expect("generate a bot input")
                .current();
            let kind = classify(&title, &body).kind;
            if !seen.contains(&kind) {
                seen.push(kind);
            }
        }
        for kind in [
            BumpKind::Patch,
            BumpKind::Minor,
            BumpKind::Major,
            BumpKind::NonMajor,
            BumpKind::Digest,
            BumpKind::Unknown,
        ] {
            assert!(seen.contains(&kind), "generator never produced {kind:?}");
        }
    }
}
