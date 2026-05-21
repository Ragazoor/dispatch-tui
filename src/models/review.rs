// ---------------------------------------------------------------------------
// ReviewDecision — review status for a PR
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    ReviewRequired,
    WaitingForResponse,
    ChangesRequested,
    Approved,
}

impl ReviewDecision {
    pub const ALL: [Self; 4] = [
        Self::ReviewRequired,
        Self::WaitingForResponse,
        Self::ChangesRequested,
        Self::Approved,
    ];

    pub const COLUMN_COUNT: usize = Self::ALL.len();

    pub fn column_index(self) -> usize {
        match self {
            Self::ReviewRequired => 0,
            Self::WaitingForResponse => 1,
            Self::ChangesRequested => 2,
            Self::Approved => 3,
        }
    }

    pub fn from_column_index(idx: usize) -> Option<Self> {
        match idx {
            0 => Some(Self::ReviewRequired),
            1 => Some(Self::WaitingForResponse),
            2 => Some(Self::ChangesRequested),
            3 => Some(Self::Approved),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReviewRequired => "Needs Review",
            Self::WaitingForResponse => "Waiting for Response",
            Self::ChangesRequested => "Changes Requested",
            Self::Approved => "Approved",
        }
    }

    /// Stable string for database storage. Not the same as `as_str()` (display)
    /// or `parse()` (GitHub wire format).
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::ReviewRequired => "ReviewRequired",
            Self::WaitingForResponse => "WaitingForResponse",
            Self::ChangesRequested => "ChangesRequested",
            Self::Approved => "Approved",
        }
    }

    /// Parse from database string. Inverse of `as_db_str`.
    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "ReviewRequired" => Some(Self::ReviewRequired),
            "WaitingForResponse" => Some(Self::WaitingForResponse),
            "ChangesRequested" => Some(Self::ChangesRequested),
            "Approved" => Some(Self::Approved),
            _ => None,
        }
    }

    /// Parse from GitHub GraphQL `reviewDecision` field value.
    /// Note: `WaitingForResponse` has no wire value — it is derived client-side.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "REVIEW_REQUIRED" => Some(Self::ReviewRequired),
            "CHANGES_REQUESTED" => Some(Self::ChangesRequested),
            "APPROVED" => Some(Self::Approved),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// PR URL helpers
// ---------------------------------------------------------------------------

pub fn pr_number_from_url(url: &str) -> Option<i64> {
    url.split(['?', '#'])
        .next()
        .and_then(|u| u.trim_end_matches('/').rsplit('/').next())
        .and_then(|s| s.parse::<i64>().ok())
}

/// Returns the URL type word: `"PR"` for pull requests, `"Issue"` for issues, `"Link"` otherwise.
pub fn url_type(url: &str) -> &'static str {
    let clean = url.split(['?', '#']).next().unwrap_or(url);
    if clean.contains("/pull/") {
        "PR"
    } else if clean.contains("/issues/") {
        "Issue"
    } else {
        "Link"
    }
}

/// Returns the display label for a URL stored in the `pr_url` field.
///
/// - URLs containing `/pull/<N>` → `"PR #N"` (or `"PR"` if no number follows)
/// - URLs containing `/issues/<N>` → `"Issue #N"` (or `"Issue"` if no number follows)
/// - Anything else → `"Link"`
pub fn url_label(url: &str) -> String {
    let clean = url.split(['?', '#']).next().unwrap_or(url);
    let type_label = url_type(url);

    if type_label != "Link" {
        let segment = if type_label == "PR" {
            "/pull/"
        } else {
            "/issues/"
        };
        if let Some((_, after)) = clean.split_once(segment) {
            if let Some(n) = after
                .trim_end_matches('/')
                .split('/')
                .next()
                .and_then(|s| s.parse::<i64>().ok())
            {
                return format!("{type_label} #{n}");
            }
            return type_label.to_string();
        }
    }

    "Link".to_string()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn review_decision_as_str_waiting_for_response() {
        assert_eq!(
            ReviewDecision::WaitingForResponse.as_str(),
            "Waiting for Response"
        );
    }

    #[test]
    fn review_decision_from_db_str_unknown_returns_none() {
        assert_eq!(ReviewDecision::from_db_str(""), None);
        assert_eq!(ReviewDecision::from_db_str("bogus"), None);
        // DB strings are PascalCase; lowercase must not match
        assert_eq!(ReviewDecision::from_db_str("review_required"), None);
        assert_eq!(ReviewDecision::from_db_str("approved"), None);
    }

    #[test]
    fn review_decision_parse_rejects_unknown_and_derived_values() {
        assert_eq!(ReviewDecision::parse(""), None);
        assert_eq!(ReviewDecision::parse("bogus"), None);
        // GitHub values are SCREAMING_SNAKE_CASE; mixed case must not match
        assert_eq!(ReviewDecision::parse("Review_Required"), None);
        // WaitingForResponse is derived client-side; the GitHub API never emits this
        assert_eq!(ReviewDecision::parse("WAITING_FOR_RESPONSE"), None);
    }

    #[test]
    fn url_type_identifies_pull_requests() {
        assert_eq!(url_type("https://github.com/org/repo/pull/42"), "PR");
    }

    #[test]
    fn url_type_identifies_issues() {
        assert_eq!(url_type("https://github.com/org/repo/issues/7"), "Issue");
    }

    #[test]
    fn url_type_returns_link_for_other_urls() {
        assert_eq!(url_type("https://jira.example.com/PROJ-123"), "Link");
        assert_eq!(url_type(""), "Link");
    }

    #[test]
    fn url_type_strips_query_and_fragment_before_matching() {
        assert_eq!(
            url_type("https://github.com/org/repo/pull/42?diff=split"),
            "PR"
        );
        assert_eq!(
            url_type("https://github.com/org/repo/issues/7#comment"),
            "Issue"
        );
    }
}
