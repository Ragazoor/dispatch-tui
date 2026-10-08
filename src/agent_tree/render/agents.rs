//! The agents section of the agent-tree pane: the band beneath the file tree
//! that lists every task with a live agent window and jumps to the selected
//! one (see `docs/specs/agent-tree.allium`'s `RefreshAgentTreeAgentList` and
//! `JumpToAgentWindow` rules, and the `AgentsSectionSitsBelowTheTree`,
//! `AgentRowShowsIdAndTitle` and `AgentKeysFollowFocus` guarantees).
//!
//! Pure view state and rendering only. Reading the board's task list and
//! selecting a tmux window both belong to the loop in [`crate::agent_tree::run`].

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::agent_tree::list_cursor::ListCursor;
use crate::models::{TaskId, TmuxWindow};

/// The most rows the section takes from the pane — the spec's
/// `config.agent_tree_agents_max_rows`.
pub(crate) const MAX_ROWS: usize = 8;

/// One listed agent, as its row draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRow {
    pub id: TaskId,
    pub title: String,
    pub window: TmuxWindow,
    /// Whether this is the pane's own task — marked, dimmed, and not jumpable.
    pub is_own: bool,
}

/// The section's rows and its own cursor. Kept apart from the tree's cursor so
/// Tab back and forth returns the user to where they were in each.
#[derive(Debug, Default)]
pub struct AgentsSection {
    rows: Vec<AgentRow>,
    cursor: usize,
    /// First row drawn, so the cursor stays in view past `MAX_ROWS`.
    offset: usize,
    /// Rows drawn at the last render — the half-page motions' page.
    viewport_rows: usize,
}

impl AgentsSection {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rows(&self) -> &[AgentRow] {
        &self.rows
    }

    /// Adopt a fresh read. The cursor follows the task it was on, not the
    /// index: an agent starting above it must not move the selection to a
    /// different agent. A task that left the list clamps the cursor instead.
    pub fn set_rows(&mut self, rows: Vec<AgentRow>) {
        let selected = self.selected().map(|row| row.id);
        self.rows = rows;
        self.cursor = selected
            .and_then(|id| self.rows.iter().position(|row| row.id == id))
            .unwrap_or(self.cursor)
            .min(self.last());
    }

    pub fn selected(&self) -> Option<&AgentRow> {
        self.rows.get(self.cursor)
    }

    /// The window Space or Enter jumps to: the selected row's, unless it is
    /// the pane's own task, where the jump is a no-op.
    pub fn jump_target(&self) -> Option<TmuxWindow> {
        self.selected()
            .filter(|row| !row.is_own)
            .map(|row| row.window.clone())
    }

    /// Rows the section wants, borders included: one per agent up to
    /// `MAX_ROWS`, and one empty row when there are none so the section — and
    /// Tab's destination — is still visible.
    pub fn height(&self) -> u16 {
        let rows = self.rows.len().clamp(1, MAX_ROWS);
        u16::try_from(rows + 2).unwrap_or(u16::MAX)
    }
}

impl ListCursor for AgentsSection {
    fn cursor_mut(&mut self) -> &mut usize {
        &mut self.cursor
    }

    fn last(&self) -> usize {
        self.rows.len().saturating_sub(1)
    }

    fn viewport_rows(&self) -> usize {
        self.viewport_rows
    }
}

/// A section's border: red while a notice is up, whatever has focus
/// (`AgentTreeNoticeRedensBorder`), else the focus colour on the focused
/// section, else the ordinary border. One rule for both of the pane's sections.
pub(crate) fn border_style(focused: bool, alert: bool) -> Style {
    use crate::palette::{CYAN, RED};
    if alert {
        Style::default().fg(RED)
    } else if focused {
        Style::default().fg(CYAN)
    } else {
        Style::default()
    }
}

/// A listed agent for tests: `#<id> task <id>`, window `task-<id>`.
#[cfg(test)]
pub(crate) fn test_row(id: i64, own: bool) -> AgentRow {
    AgentRow {
        id: TaskId(id),
        title: format!("task {id}"),
        window: crate::models::test_tmux_window(&format!("task-{id}")),
        is_own: own,
    }
}

/// Draw the section into `area`. `focused` decides the border colour and
/// whether the cursor highlight is drawn; `alert` reddens the border, and wins
/// over focus (`AgentTreeNoticeRedensBorder`).
pub fn render_agents(
    frame: &mut Frame,
    area: Rect,
    section: &mut AgentsSection,
    focused: bool,
    alert: bool,
) {
    use crate::palette::MUTED;

    let visible = usize::from(area.height.saturating_sub(2));
    section.viewport_rows = visible;
    if section.cursor < section.offset {
        section.offset = section.cursor;
    } else if visible > 0 && section.cursor >= section.offset + visible {
        section.offset = section.cursor + 1 - visible;
    }

    let border = border_style(focused, alert);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border)
        .title(" Agents (Tab) ");

    let lines: Vec<Line> = section
        .rows
        .iter()
        .enumerate()
        .skip(section.offset)
        .take(visible)
        .map(|(index, row)| {
            let mut style = Style::default();
            if row.is_own {
                style = style.fg(MUTED);
            }
            if focused && index == section.cursor {
                style = style.add_modifier(Modifier::REVERSED);
            }
            let mut text = format!("#{} {}", row.id, row.title);
            if row.is_own {
                text.push_str(" ●");
            }
            Line::from(Span::styled(text, style))
        })
        .collect();

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::test_tmux_window;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn row(id: i64, own: bool) -> AgentRow {
        test_row(id, own)
    }

    fn rendered(section: &mut AgentsSection, focused: bool, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(30, height)).expect("terminal");
        terminal
            .draw(|frame| render_agents(frame, frame.area(), section, focused, false))
            .expect("draw");
        crate::agent_tree::pane::buffer_to_string(terminal.backend().buffer())
    }

    // ---- cursor ----------------------------------------------------------

    #[test]
    fn the_cursor_follows_its_task_when_an_agent_starts_above_it() {
        let mut section = AgentsSection::new();
        section.set_rows(vec![row(5, false), row(9, false)]);
        section.down();
        assert_eq!(section.selected().unwrap().id, TaskId(9));

        section.set_rows(vec![row(2, false), row(5, false), row(9, false)]);
        assert_eq!(section.selected().unwrap().id, TaskId(9));
    }

    #[test]
    fn the_cursor_clamps_when_its_task_leaves_the_list() {
        let mut section = AgentsSection::new();
        section.set_rows(vec![row(1, false), row(2, false), row(3, false)]);
        section.bottom();
        section.set_rows(vec![row(1, false)]);
        assert_eq!(section.selected().unwrap().id, TaskId(1));
    }

    #[test]
    fn motions_clamp_at_both_ends() {
        let mut section = AgentsSection::new();
        section.set_rows(vec![row(1, false), row(2, false)]);
        section.up();
        assert_eq!(section.selected().unwrap().id, TaskId(1));
        section.down();
        section.down();
        assert_eq!(section.selected().unwrap().id, TaskId(2));
        section.top();
        assert_eq!(section.selected().unwrap().id, TaskId(1));
    }

    #[test]
    fn motions_on_an_empty_section_are_no_ops() {
        let mut section = AgentsSection::new();
        section.down();
        section.bottom();
        section.half_page_down();
        assert!(section.selected().is_none());
    }

    // ---- JumpToAgentWindow -----------------------------------------------

    #[test]
    fn the_jump_target_is_the_selected_agents_window() {
        let mut section = AgentsSection::new();
        section.set_rows(vec![row(1, false), row(2, true)]);
        assert_eq!(section.jump_target(), Some(test_tmux_window("task-1")));
    }

    #[test]
    fn the_own_task_has_no_jump_target() {
        let mut section = AgentsSection::new();
        section.set_rows(vec![row(1, false), row(2, true)]);
        section.down();
        assert_eq!(section.jump_target(), None);
    }

    // ---- AgentsSectionSitsBelowTheTree / AgentRowShowsIdAndTitle ----------

    #[test]
    fn the_section_grows_with_its_rows_up_to_the_cap() {
        let mut section = AgentsSection::new();
        assert_eq!(section.height(), 3, "an empty section still shows one row");
        section.set_rows((1..=3).map(|id| row(id, false)).collect());
        assert_eq!(section.height(), 5);
        section.set_rows((1..=20).map(|id| row(id, false)).collect());
        assert_eq!(section.height(), (MAX_ROWS + 2) as u16);
    }

    #[test]
    fn a_row_reads_id_then_title_and_the_own_task_is_marked() {
        let mut section = AgentsSection::new();
        section.set_rows(vec![row(4941, false), row(4942, true)]);
        let out = rendered(&mut section, true, 4);
        assert!(out.contains("Agents"), "{out}");
        assert!(out.contains("#4941 task 4941"), "{out}");
        assert!(out.contains("#4942 task 4942 ●"), "{out}");
    }

    #[test]
    fn the_title_names_tab_whichever_section_has_focus() {
        // AgentsSectionSitsBelowTheTree: the "(Tab)" hint is the only place
        // the pane names the key that reaches the section.
        let mut section = AgentsSection::new();
        for focused in [true, false] {
            let out = rendered(&mut section, focused, 3);
            assert!(out.contains("Agents (Tab)"), "focused={focused}: {out}");
        }
    }

    #[test]
    fn past_the_cap_the_section_scrolls_to_keep_the_cursor_in_view() {
        let mut section = AgentsSection::new();
        section.set_rows((1..=20).map(|id| row(id, false)).collect());
        section.bottom();
        let height = section.height();
        let out = rendered(&mut section, true, height);
        assert!(out.contains("#20 task 20"), "{out}");
        assert!(!out.contains("#1 task 1\n"), "{out}");
    }

    #[test]
    fn half_a_page_is_half_the_sections_visible_height() {
        let mut section = AgentsSection::new();
        section.set_rows((1..=20).map(|id| row(id, false)).collect());
        rendered(&mut section, true, 10);
        section.half_page_down();
        assert_eq!(section.selected().unwrap().id, TaskId(5));
        section.half_page_up();
        assert_eq!(section.selected().unwrap().id, TaskId(1));
    }
}
