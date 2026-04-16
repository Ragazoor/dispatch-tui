// ---------------------------------------------------------------------------
// Tips — embedded tip files loaded at compile time
// ---------------------------------------------------------------------------

/// A single tip card loaded from a numbered markdown file.
#[derive(Debug, Clone)]
pub struct Tip {
    pub id: u32,
    pub title: String,
    pub body: String,
}

/// Parse a `Tip` from the numeric id and raw markdown content.
/// The first `## ` heading becomes the title; everything after is the body.
pub fn parse_tip(id: u32, content: &str) -> Tip {
    let mut title = String::new();
    let mut body_start = 0;

    for (i, line) in content.lines().enumerate() {
        if let Some(stripped) = line.strip_prefix("## ") {
            title = stripped.trim().to_string();
            body_start = i + 1;
            break;
        }
    }

    let body: String = content
        .lines()
        .skip(body_start)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_start_matches('\n')
        .to_string();

    Tip { id, title, body }
}

/// Returns all embedded tips sorted by id.
pub fn embedded_tips() -> Vec<Tip> {
    let raw: &[(&str, &str)] = &[
        ("001", include_str!("tips/001-quick-dispatch.md")),
        ("002", include_str!("tips/002-review-board.md")),
        ("003", include_str!("tips/003-split-pane.md")),
        ("004", include_str!("tips/004-help-overlay.md")),
        ("005", include_str!("tips/005-epic-brainstorm.md")),
    ];

    let mut tips: Vec<Tip> = raw
        .iter()
        .filter_map(|(prefix, content)| {
            let id: u32 = prefix.parse().ok()?;
            Some(parse_tip(id, content))
        })
        .collect();

    tips.sort_by_key(|t| t.id);
    tips
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tip_extracts_title_and_body() {
        let content = "## Quick Dispatch\n\nPress **Shift+D** to dispatch immediately.\n";
        let tip = parse_tip(1, content);
        assert_eq!(tip.id, 1);
        assert_eq!(tip.title, "Quick Dispatch");
        assert_eq!(
            tip.body.trim(),
            "Press **Shift+D** to dispatch immediately."
        );
    }

    #[test]
    fn parse_tip_trims_title_whitespace() {
        let content = "##   Spaces Around Title   \n\nBody text.\n";
        let tip = parse_tip(2, content);
        assert_eq!(tip.title, "Spaces Around Title");
    }

    #[test]
    fn parse_tip_handles_missing_heading() {
        // No ## heading — title falls back to empty string, body is whole content
        let content = "Just some text\n";
        let tip = parse_tip(3, content);
        assert_eq!(tip.title, "");
        assert!(tip.body.contains("Just some text"));
    }

    #[test]
    fn embedded_tips_are_non_empty_and_sorted() {
        let tips = embedded_tips();
        assert!(!tips.is_empty());
        // ids are monotonically increasing
        let ids: Vec<u32> = tips.iter().map(|t| t.id).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
        // every tip has a non-empty title
        for tip in &tips {
            assert!(!tip.title.is_empty(), "tip {} has empty title", tip.id);
        }
    }
}
