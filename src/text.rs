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

/// Separator joining ancestor-breadcrumb segments, shared by the flat-view
/// epic header (`fair_truncate_segments`) and the epic-view border title.
pub const BREADCRUMB_SEPARATOR: &str = " › ";

/// Join `segments` with `sep`, truncating so the joined result fits within
/// `budget` display characters while keeping a visible prefix of every segment.
///
/// Water-filling: segments shorter than the fair share are kept whole and their
/// unused width is redistributed to longer segments. Falls back to a
/// whole-string truncate when the budget is too small to give every segment at
/// least a one-char prefix plus ellipsis.
pub fn fair_truncate_segments<S: AsRef<str>>(segments: &[S], budget: usize, sep: &str) -> String {
    if segments.is_empty() {
        return String::new();
    }
    let segments: Vec<&str> = segments.iter().map(AsRef::as_ref).collect();
    let n = segments.len();
    let sep_cost = (n - 1) * sep.chars().count();
    let lens: Vec<usize> = segments.iter().map(|s| s.chars().count()).collect();
    let name_budget = budget.saturating_sub(sep_cost);

    // Everything already fits (separators included).
    if sep_cost + lens.iter().sum::<usize>() <= budget {
        return segments.join(sep);
    }
    // Too narrow to give each segment ≥1 char + ellipsis: truncate the whole string.
    if name_budget < 2 * n {
        return truncate(&segments.join(sep), budget);
    }

    // Water-fill: lock any segment at or under the current fair share, return its
    // slack to the pool, and repeat until only oversized segments remain.
    let mut alloc = vec![0usize; n];
    let mut settled = vec![false; n];
    let mut pool = name_budget;
    let mut remaining = n;
    loop {
        let fair = pool / remaining;
        let mut locked_any = false;
        for i in 0..n {
            if !settled[i] && lens[i] <= fair {
                alloc[i] = lens[i];
                settled[i] = true;
                pool -= lens[i];
                remaining -= 1;
                locked_any = true;
            }
        }
        if !locked_any || remaining == 0 {
            break;
        }
    }
    // Split the remaining pool across still-oversized segments; leftmost get the
    // remainder chars first.
    if let Some(base) = pool.checked_div(remaining) {
        let mut extra = pool % remaining;
        for i in 0..n {
            if !settled[i] {
                alloc[i] = base + usize::from(extra > 0);
                extra = extra.saturating_sub(1);
            }
        }
    }

    segments
        .iter()
        .enumerate()
        .map(|(i, s)| truncate(s, alloc[i]))
        .collect::<Vec<_>>()
        .join(sep)
}
