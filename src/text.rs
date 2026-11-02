//! Plain-text fitting helpers shared by every drawing surface: the board
//! (`tui`) and the agent-tree panes (`agent_tree`, through `epic_header`).

/// Truncate a string to at most `max` characters, appending "…" if truncated.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", truncated)
    }
}

/// Separator joining ancestor-breadcrumb segments in the epic-view border
/// title.
pub const BREADCRUMB_SEPARATOR: &str = " › ";
