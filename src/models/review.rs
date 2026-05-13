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
