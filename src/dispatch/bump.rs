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

/// Test-only: production never enumerates the kinds, it matches on them
/// exhaustively. This list exists so a test that must cover all of them cannot
/// go quietly narrow, and it lives beside the enum rather than in the test
/// module so someone adding a seventh variant sees it.
#[cfg(test)]
impl BumpKind {
    /// Every kind. `dependabot_decision` and `prompt_line` are exhaustive
    /// matches and fail to compile on a new variant; a hand-written list in a
    /// test is the one place that would not.
    pub(super) const ALL: &'static [BumpKind] = &[
        BumpKind::Patch,
        BumpKind::Minor,
        BumpKind::Major,
        BumpKind::NonMajor,
        BumpKind::Digest,
        BumpKind::Unknown,
    ];
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
/// the PR body rather than the title — Dependabot writes "Bumps \[foo\] from
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
mod tests;
