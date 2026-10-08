//! The commits section of the agent-tree pane: the band beneath the file tree
//! and above the agents section that lists the agent's own commits since the
//! fork point, under a fixed "unstaged work" row, and selects which change the
//! tree and the diff pane show (see `docs/specs/agent-tree.allium`'s
//! `RefreshAgentTreeCommitList` and `SelectAgentTreeSource` rules, and the
//! `CommitsSectionSitsBetweenTreeAndAgents`, `CommitRowShowsShortIdAndSubject`
//! and `AgentTreeSourceIsOneSelection` guarantees).
//!
//! Pure view state and rendering only. Reading the commits from git belongs to
//! `super::agent_tree::git_branch_commits`, and the selection itself lives on
//! the pane's `RenderState`.

use std::time::Duration;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::cli::agent_tree_agents::border_style;
use crate::tui::ui::palette::YELLOW;

/// The most rows the section takes from the pane, "unstaged work" included —
/// the spec's `config.agent_tree_commits_max_rows`.
pub(crate) const MAX_ROWS: usize = 6;

/// The most commits listed — the spec's `config.agent_tree_commits_max_listed`.
pub(crate) const MAX_LISTED: usize = 50;

/// The section's re-read cadence — `config.agent_tree_commits_refresh_interval`.
pub(crate) const COMMITS_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

/// One of the agent's own commits, as the section lists it — the spec's
/// `AgentCommit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCommit {
    /// The full commit id; what a selection is keyed by.
    pub id: String,
    /// The first line of the commit message, as git prints it.
    pub subject: String,
}

/// How many leading characters of a commit id a row shows.
const SHORT_ID_LEN: usize = 7;

/// Marks the row that is the pane's selected source, in every state — see
/// `CommitRowShowsShortIdAndSubject`. Unselected rows carry a blank of the
/// same width so the labels stay in one column.
const SELECTED_MARKER: &str = "\u{25cf} ";
const UNSELECTED_MARKER: &str = "  ";

/// A commit id's leading characters, as a row and the tree title show it.
pub(crate) fn short_id(id: &str) -> &str {
    id.get(..SHORT_ID_LEN).unwrap_or(id)
}

/// The section's commits and its own cursor. Row 0 is the fixed "unstaged
/// work" row; row `n` is `commits[n - 1]`.
#[derive(Debug, Default)]
pub struct CommitsSection {
    commits: Vec<AgentCommit>,
    cursor: usize,
    /// First row drawn, so the cursor stays in view past `MAX_ROWS`.
    offset: usize,
    /// Rows drawn at the last render — the half-page motions' page.
    viewport_rows: usize,
}

impl CommitsSection {
    pub fn new() -> Self {
        Self::default()
    }

    /// The listed commits, newest first — `pane.listed_commits`.
    pub fn commits(&self) -> &[AgentCommit] {
        &self.commits
    }

    /// Adopt a fresh read. The cursor follows the commit it was on by id, and
    /// clamps to the last row when that commit has gone.
    pub fn set_commits(&mut self, commits: Vec<AgentCommit>) {
        let on = self.cursor_commit().map(|c| c.id.clone());
        self.commits = commits;
        self.cursor = on
            .and_then(|id| self.commits.iter().position(|c| c.id == id))
            .map_or(self.cursor, |index| index + 1)
            .min(self.last());
    }

    /// The commit under the cursor, or `None` on the "unstaged work" row.
    pub fn cursor_commit(&self) -> Option<&AgentCommit> {
        self.cursor.checked_sub(1).and_then(|i| self.commits.get(i))
    }

    /// The last row's index: "unstaged work" is always row 0.
    fn last(&self) -> usize {
        self.commits.len()
    }

    pub fn up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn down(&mut self) {
        self.cursor = (self.cursor + 1).min(self.last());
    }

    pub fn top(&mut self) {
        self.cursor = 0;
    }

    pub fn bottom(&mut self) {
        self.cursor = self.last();
    }

    fn half_page(&self) -> usize {
        crate::cli::half_page(self.viewport_rows)
    }

    pub fn half_page_down(&mut self) {
        self.cursor = (self.cursor + self.half_page()).min(self.last());
    }

    pub fn half_page_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(self.half_page());
    }

    /// Rows the section wants, borders included: "unstaged work" plus one per
    /// commit, capped at [`MAX_ROWS`].
    pub fn height(&self) -> u16 {
        let rows = (self.commits.len() + 1).min(MAX_ROWS);
        u16::try_from(rows + 2).unwrap_or(u16::MAX)
    }
}

/// Draw the section into `area`. `selected` is the pane's selected source
/// (`None` for unstaged work), marked in every state; `focused` decides the
/// border colour and the cursor highlight; `alert` reddens the border.
pub fn render_commits(
    frame: &mut Frame,
    area: Rect,
    section: &mut CommitsSection,
    selected: Option<&str>,
    focused: bool,
    alert: bool,
) {
    let visible = usize::from(area.height.saturating_sub(2));
    section.viewport_rows = visible;
    if section.cursor < section.offset {
        section.offset = section.cursor;
    } else if visible > 0 && section.cursor >= section.offset + visible {
        section.offset = section.cursor + 1 - visible;
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style(focused, alert))
        .title(" Commits (Tab) ");

    let unstaged = std::iter::once((None, "unstaged work".to_string()));
    let commits = section.commits.iter().map(|commit| {
        (
            Some(commit.id.as_str()),
            format!("{} {}", short_id(&commit.id), commit.subject),
        )
    });
    let lines: Vec<Line> = unstaged
        .chain(commits)
        .enumerate()
        .skip(section.offset)
        .take(visible)
        .map(|(index, (id, label))| {
            let marker = if id == selected {
                SELECTED_MARKER
            } else {
                UNSELECTED_MARKER
            };
            let mut style = Style::default();
            if focused && index == section.cursor {
                style = style.add_modifier(Modifier::REVERSED);
            }
            Line::from(vec![
                Span::styled(marker, style.fg(YELLOW)),
                Span::styled(label, style),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines).block(block), area);
}
