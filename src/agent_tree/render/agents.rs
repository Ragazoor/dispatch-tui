//! The agents section of the agent-tree pane: the band beneath the file tree
//! that lists every task with an attached or detached agent, jumps to the
//! selected attached one and resumes a detached one (see
//! `docs/specs/agent-tree.allium`'s
//! `RefreshAgentTreeAgentList`, `JumpToAgentWindow` and
//! `ResumeDetachedAgentFromThePane` rules, and the `TwoViewsShareThePane`,
//! `AgentRowShowsAgentName` and `AgentKeysFollowFocus` guarantees).
//!
//! Pure view state and rendering only. Reading the board's task list and
//! selecting a tmux window both belong to the loop in [`crate::agent_tree::run`].

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::agent_tree::keys::KeyAction;
use crate::agent_tree::list_cursor::ListCursor;
use crate::hooks::wire::PaneEpic;
use crate::models::{TaskId, TmuxWindow};

/// One listed agent, as its row draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRow {
    pub id: TaskId,
    pub title: String,
    /// The word the row leads with, the task's current agent name; empty
    /// falls back to `#<id>`.
    pub agent_name: String,
    /// The agent's window; `None` for a detached agent, which has none and
    /// which Space resumes instead (`ResumeDetachedAgentFromThePane`).
    pub window: Option<TmuxWindow>,
    /// Whether this is the pane's own task — dimmed, and not jumpable.
    pub is_own: bool,
    /// The task's own epic, which the row is grouped under; `None` for an
    /// epicless task (`AgentListGroupsByEpic`).
    pub epic: Option<PaneEpic>,
}

impl AgentRow {
    /// What the row leads with: the agent name, or `#<id>` when the task has no
    /// current agent
    /// (`agent-tree.allium: AgentRowShowsAgentName`).
    pub fn label(&self) -> String {
        crate::models::task_lead(
            self.id,
            Some(self.agent_name.as_str()).filter(|name| !name.is_empty()),
        )
    }
}

/// One successful read of the board's agent list: the rows to show and how
/// many agents the board's filter left out of them
/// (`agent-tree.allium: ListsFollowTheBoardFilter`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentList {
    pub rows: Vec<AgentRow>,
    pub hidden: usize,
}

/// The section's rows and its own cursor. Kept apart from the tree's cursor so
/// Tab back and forth returns the user to where they were in each.
#[derive(Debug, Default)]
pub struct AgentsSection {
    rows: Vec<AgentRow>,
    cursor: usize,
    /// First drawn line shown, headers counted, so the cursor stays in view
    /// in a short pane.
    offset: usize,
    /// Rows drawn at the last render — the half-page motions' page.
    viewport_rows: usize,
    /// Agents the board's filter left out, named on the section's last line
    /// (`AgentListNamesHiddenAgents`).
    hidden: usize,
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

    /// How many agents the board's filter left out of the last good read.
    pub fn hidden(&self) -> usize {
        self.hidden
    }

    pub fn set_hidden(&mut self, hidden: usize) {
        self.hidden = hidden;
    }

    pub fn selected(&self) -> Option<&AgentRow> {
        self.rows.get(self.cursor)
    }

    /// What Space or Enter does on the selected row: jump to an attached
    /// agent's window, resume a detached one, or nothing on the pane's own
    /// task or an empty list.
    pub fn space_action(&self) -> Option<KeyAction> {
        let row = self.selected().filter(|row| !row.is_own)?;
        Some(match &row.window {
            Some(window) => KeyAction::JumpTo(window.clone()),
            None => KeyAction::Resume(row.id),
        })
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

/// A group's epic ids from the root down to its own: its full path, which
/// [`compact_paths`] reduces to the headers it draws (`AgentListGroupsByEpic`).
pub(crate) fn pane_epic_path(epic: &PaneEpic) -> Vec<crate::models::EpicId> {
    epic.ancestors
        .iter()
        .map(|a| a.id)
        .chain(std::iter::once(epic.id))
        .collect()
}

/// Each row's compact path (`AgentListGroupsByEpic`): the drawn epics from
/// the top down to its own, `None` for an epicless row. A header draws only
/// for an epic an agent sits in, or where two branches meet
/// (`crate::epic_tree::CompactTree`).
pub(crate) fn compact_paths(rows: &[AgentRow]) -> Vec<Option<Vec<crate::models::EpicId>>> {
    let (groups, full) = epic_groups(rows);
    let tree = crate::epic_tree::CompactTree::build(&full, |e| e.0);
    rows.iter()
        .map(|row| {
            let epic = row.epic.as_ref()?;
            let at = groups.iter().position(|&id| id == epic.id)?;
            tree.paths.get(at).cloned()
        })
        .collect()
}

/// The distinct epics `rows` sit in, first seen first, and each one's full
/// path from the root.
fn epic_groups(rows: &[AgentRow]) -> (Vec<crate::models::EpicId>, Vec<Vec<crate::models::EpicId>>) {
    let mut ids = Vec::new();
    let mut paths = Vec::new();
    for epic in rows.iter().filter_map(|row| row.epic.as_ref()) {
        if !ids.contains(&epic.id) {
            ids.push(epic.id);
            paths.push(pane_epic_path(epic));
        }
    }
    (ids, paths)
}

/// Order `rows` as the section draws them: the epic groups in compact tree
/// order, siblings by epic id, each group's agents by task id, then the
/// epicless agents by task id (RefreshAgentTreeAgentList's grouped_by_epic).
pub(crate) fn order_rows(rows: &mut [AgentRow]) {
    let (ids, full) = epic_groups(rows);
    let tree = crate::epic_tree::CompactTree::build(&full, |e| e.0);
    let mut rank = std::collections::HashMap::with_capacity(ids.len());
    for (place, &group) in tree.order.iter().enumerate() {
        if let Some(&id) = ids.get(group) {
            rank.insert(id, place);
        }
    }
    rows.sort_by_key(|row| {
        let place = row.epic.as_ref().and_then(|e| rank.get(&e.id).copied());
        (place.is_none(), place, row.id)
    });
}

/// One line the section draws (`AgentListGroupsByEpic`). Only an agent line
/// holds the cursor; the other two are decoration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DrawnLine {
    /// A header over the group that starts at `row`: the epic at `level` on
    /// that row's compact path ([`compact_paths`]), the last its own epic.
    /// `level` is its depth.
    EpicHeader {
        row: usize,
        level: usize,
    },
    /// The rule between the last epic group and the epicless agents.
    EndOfEpics,
    Agent(usize),
    /// The muted count of agents the board's filter hides, after every other
    /// line (`AgentListNamesHiddenAgents`).
    HiddenByFilter,
}

/// The text of the hidden-agents line.
pub(crate) fn hidden_agents_text(hidden: usize) -> String {
    let noun = if hidden == 1 { "agent" } else { "agents" };
    format!("{hidden} {noun} hidden by board filter")
}

/// The lines `rows` draw as: a header wherever a new epic's group starts, and
/// the end-of-epics rule where the epicless agents follow an epic group.
/// `rows` arrive already grouped (`read_agent_list`).
#[cfg(test)]
pub(crate) fn drawn_lines(rows: &[AgentRow]) -> Vec<DrawnLine> {
    drawn_lines_with(rows, compact_paths(rows))
}

/// `drawn_lines` over paths the caller already has from [`compact_paths`].
fn drawn_lines_with(
    rows: &[AgentRow],
    paths: Vec<Option<Vec<crate::models::EpicId>>>,
) -> Vec<DrawnLine> {
    let mut lines = Vec::with_capacity(rows.len());
    // The headers the previous row sat under, root first: a new group draws
    // only the part of its path it does not share with them.
    let mut open: Vec<crate::models::EpicId> = Vec::new();
    for (index, path) in paths.into_iter().enumerate() {
        match path {
            Some(path) => {
                if path != open {
                    let shared = open.iter().zip(&path).take_while(|(a, b)| a == b).count();
                    // The group's own header always draws, even when the
                    // previous group was its ancestor.
                    let from = shared.min(path.len() - 1);
                    for level in from..path.len() {
                        lines.push(DrawnLine::EpicHeader { row: index, level });
                    }
                    open = path;
                }
            }
            None => {
                if !open.is_empty() {
                    lines.push(DrawnLine::EndOfEpics);
                    open.clear();
                }
            }
        }
        lines.push(DrawnLine::Agent(index));
    }
    lines
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
        agent_name: String::new(),
        window: Some(crate::models::test_tmux_window(&format!("task-{id}"))),
        is_own: own,
        epic: None,
    }
}

/// A detached agent for tests: `#<id> task <id>`, no window.
#[cfg(test)]
pub(crate) fn test_detached_row(id: i64) -> AgentRow {
    AgentRow {
        window: None,
        ..test_row(id, false)
    }
}

/// A [`PaneEpic`] for tests, from its id and breadcrumb.
#[cfg(test)]
pub(crate) fn test_pane_epic(id: i64, breadcrumb: &[&str]) -> PaneEpic {
    PaneEpic {
        id: crate::models::EpicId(id),
        breadcrumb: breadcrumb.iter().map(|s| s.to_string()).collect(),
        ancestors: Vec::new(),
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
    notice: Option<&str>,
) {
    use crate::palette::{MUTED, RED};

    let visible = usize::from(area.height.saturating_sub(2));
    section.viewport_rows = visible;
    let paths = compact_paths(&section.rows);
    let mut drawn = drawn_lines_with(&section.rows, paths.clone());
    if section.hidden > 0 {
        drawn.push(DrawnLine::HiddenByFilter);
    }
    if let Some(cursor_line) = drawn
        .iter()
        .position(|line| *line == DrawnLine::Agent(section.cursor))
    {
        // On a group's first agent, every header line above it (a nested
        // group's ancestors too) or the rule comes into view with it
        // (AgentListGroupsByEpic).
        let mut top_line = cursor_line;
        while top_line > 0
            && matches!(
                drawn[top_line - 1],
                DrawnLine::EpicHeader { .. } | DrawnLine::EndOfEpics
            )
        {
            top_line -= 1;
        }
        if top_line < section.offset {
            section.offset = top_line;
        }
        // Then the cursor's own row, which wins in a pane too short for both
        // — on the last agent, with the hidden-agents line below it where the
        // pane has room for both (AgentListNamesHiddenAgents).
        let bottom_line = match drawn.get(cursor_line + 1) {
            Some(DrawnLine::HiddenByFilter) if visible > 1 => cursor_line + 1,
            _ => cursor_line,
        };
        if visible > 0 && bottom_line >= section.offset + visible {
            section.offset = bottom_line + 1 - visible;
        }
    }

    let border = border_style(focused, alert);
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(border)
        .title(" Agents (Tab: browse) ");
    // The agents view is the whole pane, so a notice has no tree border to sit
    // on: it goes on this one (TwoViewsShareThePane).
    if let Some(text) = notice {
        block = block.title_bottom(Line::from(Span::styled(
            format!(" {text} "),
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        )));
    }

    let width = area.width.saturating_sub(2);
    let lines: Vec<Line> = drawn
        .iter()
        .skip(section.offset)
        .take(visible)
        .filter_map(|line| {
            let index = match *line {
                DrawnLine::Agent(index) => index,
                DrawnLine::EpicHeader { row, level } => {
                    let epic = section.rows.get(row)?.epic.as_ref()?;
                    let id = *paths.get(row)?.as_ref()?.get(level)?;
                    let title = if id == epic.id {
                        epic.breadcrumb.last().map_or("", String::as_str)
                    } else {
                        epic.ancestors
                            .iter()
                            .find(|a| a.id == id)
                            .map_or("", |a| a.title.as_str())
                    };
                    return Some(crate::epic_header::epic_header_line(
                        id, title, width, level,
                    ));
                }
                DrawnLine::EndOfEpics => return Some(crate::epic_header::end_of_epics_line(width)),
                DrawnLine::HiddenByFilter => {
                    return Some(Line::from(Span::styled(
                        hidden_agents_text(section.hidden),
                        Style::default().fg(MUTED),
                    )))
                }
            };
            let row = section.rows.get(index)?;
            let mut style = Style::default();
            if row.is_own {
                style = style.fg(MUTED);
            }
            if focused && index == section.cursor {
                style = style.add_modifier(Modifier::REVERSED);
            }
            // The dot leads every row so it lines up and the edge never
            // clips it: attached or detached (AgentRowShowsAgentName).
            let dot = if row.window.is_some() { '●' } else { '○' };
            // An agent steps in with its group's header; the epicless do not.
            let indent = paths.get(index).and_then(Option::as_ref).map_or(0, |p| {
                crate::epic_header::epic_indent(p.len().saturating_sub(1))
            });
            let text = format!(
                "{:indent$}{dot} {} {}",
                "",
                row.label(),
                row.title,
                indent = indent
            );
            Some(Line::from(Span::styled(text, style)))
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
            .draw(|frame| render_agents(frame, frame.area(), section, focused, false, None))
            .expect("draw");
        crate::agent_tree::pane::buffer_to_string(terminal.backend().buffer())
    }

    fn in_epic(id: i64, epic: i64, breadcrumb: &[&str]) -> AgentRow {
        AgentRow {
            epic: Some(test_pane_epic(epic, breadcrumb)),
            ..row(id, false)
        }
    }

    /// The drawn line each text fragment first appears on.
    fn line_of(out: &str, needle: &str) -> usize {
        out.lines()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} not drawn:\n{out}"))
    }

    // ---- AgentListNamesHiddenAgents --------------------------------------

    #[test]
    fn hidden_agents_are_named_on_a_muted_last_line() {
        let mut section = AgentsSection::new();
        section.set_rows(vec![in_epic(7, 3, &["Three"]), row(4, false)]);
        section.set_hidden(3);
        let out = rendered(&mut section, true, 8);
        let hint = line_of(&out, "3 agents hidden by board");
        assert!(hint > line_of(&out, "task 4"), "{out}");
        assert!(hint > line_of(&out, "╌"), "{out}");
    }

    #[test]
    fn one_hidden_agent_reads_singular() {
        assert_eq!(hidden_agents_text(1), "1 agent hidden by board filter");
    }

    #[test]
    fn nothing_hidden_draws_no_hint_line() {
        let mut section = AgentsSection::new();
        section.set_rows(vec![row(4, false)]);
        let out = rendered(&mut section, true, 6);
        assert!(!out.contains("hidden by board"), "{out}");
    }

    #[test]
    fn a_list_whose_every_agent_is_hidden_shows_the_hint_alone() {
        let mut section = AgentsSection::new();
        section.set_hidden(2);
        let out = rendered(&mut section, true, 5);
        assert!(out.contains("2 agents hidden by board"), "{out}");
        assert_eq!(section.space_action(), None);
    }

    #[test]
    fn the_cursor_on_the_last_agent_brings_the_hint_into_view() {
        let mut section = AgentsSection::new();
        section.set_rows((1..=6).map(|id| row(id, false)).collect());
        section.set_hidden(1);
        rendered(&mut section, true, 5);
        section.bottom();
        let out = rendered(&mut section, true, 5);
        assert!(out.contains("task 6"), "{out}");
        assert!(out.contains("1 agent hidden"), "{out}");
    }

    // ---- AgentListGroupsByEpic -------------------------------------------

    #[test]
    fn the_drawn_lines_put_a_header_over_each_epic_and_a_rule_before_the_epicless() {
        let rows = vec![
            in_epic(7, 3, &["Three"]),
            in_epic(2, 5, &["Five"]),
            in_epic(9, 5, &["Five"]),
            row(4, false),
        ];
        assert_eq!(
            drawn_lines(&rows),
            vec![
                DrawnLine::EpicHeader { row: 0, level: 0 },
                DrawnLine::Agent(0),
                DrawnLine::EpicHeader { row: 1, level: 0 },
                DrawnLine::Agent(1),
                DrawnLine::Agent(2),
                DrawnLine::EndOfEpics,
                DrawnLine::Agent(3),
            ]
        );
    }

    #[test]
    fn a_list_with_no_epic_agents_draws_no_header_and_no_rule() {
        let rows = vec![row(1, false), row(2, false)];
        assert_eq!(
            drawn_lines(&rows),
            vec![DrawnLine::Agent(0), DrawnLine::Agent(1)]
        );
    }

    #[test]
    fn a_list_of_only_epic_agents_draws_no_rule() {
        let rows = vec![in_epic(1, 3, &["Three"]), in_epic(2, 3, &["Three"])];
        assert_eq!(
            drawn_lines(&rows),
            vec![
                DrawnLine::EpicHeader { row: 0, level: 0 },
                DrawnLine::Agent(0),
                DrawnLine::Agent(1)
            ]
        );
    }

    #[test]
    fn an_epic_header_reads_id_then_own_title_above_its_agents() {
        let mut section = AgentsSection::new();
        section.set_rows(vec![
            in_epic(41, 314, &["Dispatch"]),
            in_epic(42, 320, &["Dispatch", "Pane"]),
            row(43, false),
        ]);
        let out = rendered(&mut section, true, 10);
        let first = line_of(&out, "#314 Dispatch ──");
        let nested = line_of(&out, "#320 Pane ──");
        assert!(!out.contains("Dispatch › Pane"), "ancestor drawn:\n{out}");
        let rule = line_of(&out, "╌╌ · ╌╌");
        assert_eq!(line_of(&out, "#41 task 41"), first + 1, "{out}");
        assert_eq!(line_of(&out, "#42 task 42"), nested + 1, "{out}");
        assert_eq!(line_of(&out, "#43 task 43"), rule + 1, "{out}");
    }

    /// An agent in `epic` (own title `title`) whose ancestors, root first,
    /// are `ancestors`.
    fn nested(id: i64, epic: i64, title: &str, ancestors: &[(i64, &str)]) -> AgentRow {
        let mut pane_epic = test_pane_epic(epic, &[title]);
        pane_epic.ancestors = ancestors
            .iter()
            .map(|&(id, title)| crate::hooks::wire::PaneEpicRef {
                id: crate::models::EpicId(id),
                title: title.to_string(),
            })
            .collect();
        AgentRow {
            epic: Some(pane_epic),
            ..row(id, false)
        }
    }

    /// The compact tree: a header for an epic an agent sits in, and for an
    /// epic where two branches meet; an ancestor only one branch passes
    /// through draws none and adds no level (AgentListGroupsByEpic). Here
    /// 10 owns an agent and 30 sits under it; 40 has a single branch, so 50
    /// draws flush.
    #[test]
    fn a_nested_group_draws_headers_only_where_branches_meet() {
        let rows = vec![
            nested(1, 10, "Ten", &[]),
            nested(2, 30, "Thirty", &[(10, "Ten")]),
            nested(3, 50, "Fifty", &[(40, "Forty")]),
        ];
        assert_eq!(
            drawn_lines(&rows),
            vec![
                DrawnLine::EpicHeader { row: 0, level: 0 },
                DrawnLine::Agent(0),
                DrawnLine::EpicHeader { row: 1, level: 1 },
                DrawnLine::Agent(1),
                DrawnLine::EpicHeader { row: 2, level: 0 },
                DrawnLine::Agent(2),
            ]
        );
    }

    /// The elicitation example, rows as the read orders them: E (a leaf)
    /// under A, then C and D meeting at B. X sits between B and C with one
    /// branch, so C is one level under B.
    #[test]
    fn headers_meet_at_the_nearest_shared_parent() {
        let rows = vec![
            nested(3, 6, "E", &[(1, "A")]),
            nested(1, 4, "C", &[(1, "A"), (2, "B"), (3, "X")]),
            nested(2, 5, "D", &[(1, "A"), (2, "B")]),
        ];
        assert_eq!(
            drawn_lines(&rows),
            vec![
                DrawnLine::EpicHeader { row: 0, level: 0 },
                DrawnLine::EpicHeader { row: 0, level: 1 },
                DrawnLine::Agent(0),
                DrawnLine::EpicHeader { row: 1, level: 1 },
                DrawnLine::EpicHeader { row: 1, level: 2 },
                DrawnLine::Agent(1),
                DrawnLine::EpicHeader { row: 2, level: 2 },
                DrawnLine::Agent(2),
            ]
        );
        let mut section = AgentsSection::new();
        section.set_rows(rows);
        let out = rendered(&mut section, true, 14);
        let lines: Vec<&str> = out.lines().collect();
        let at = |needle: &str| lines[line_of(&out, needle)];
        assert!(at("#1 A ").starts_with("│#1 A "), "{out}");
        assert!(at("#2 B ").starts_with("│  #2 B "), "{out}");
        assert!(at("#4 C ").starts_with("│    #4 C "), "{out}");
        assert!(!out.contains("#3 X"), "{out}");
        assert!(at("task 1").starts_with("│    ● #1 task 1"), "{out}");
    }

    /// Headers and agent rows of a nested group step in together, two columns
    /// per level; the epicless agents below the rule do not.
    #[test]
    fn a_nested_groups_header_and_agents_are_indented() {
        let mut section = AgentsSection::new();
        section.set_rows(vec![
            nested(2, 30, "Thirty", &[(10, "Ten")]),
            nested(3, 40, "Forty", &[(10, "Ten")]),
            row(4, false),
        ]);
        let out = rendered(&mut section, true, 12);
        let lines: Vec<&str> = out.lines().collect();
        let at = |needle: &str| lines[line_of(&out, needle)];
        assert!(at("#10 Ten").starts_with("│#10 Ten"), "{out}");
        assert!(at("#30 Thirty").starts_with("│  #30 Thirty"), "{out}");
        assert!(at("task 2").starts_with("│  ● #2 task 2"), "{out}");
        assert!(at("task 4").starts_with("│● #4 task 4"), "{out}");
    }

    #[test]
    fn the_cursor_steps_over_headers_from_agent_to_agent() {
        let mut section = AgentsSection::new();
        section.set_rows(vec![in_epic(1, 3, &["Three"]), in_epic(2, 5, &["Five"])]);
        section.down();
        assert_eq!(section.selected().unwrap().id, TaskId(2));
    }

    #[test]
    fn with_headers_a_short_pane_still_scrolls_the_last_agent_into_view() {
        let mut section = AgentsSection::new();
        section.set_rows(
            (1..=20)
                .map(|id| in_epic(id, id, &[&format!("E{id}")]))
                .collect(),
        );
        section.bottom();
        let out = rendered(&mut section, true, 10);
        assert!(out.contains("#20 task 20"), "{out}");
    }

    #[test]
    fn scrolling_up_onto_a_groups_first_agent_brings_its_header_into_view() {
        let mut section = AgentsSection::new();
        let mut rows: Vec<AgentRow> = (1..=10).map(|id| in_epic(id, 3, &["Three"])).collect();
        rows.extend((11..=20).map(|id| in_epic(id, 5, &["Five"])));
        section.set_rows(rows);
        section.bottom();
        rendered(&mut section, true, 10);
        for _ in 0..9 {
            section.up();
        }
        assert_eq!(section.selected().unwrap().id, TaskId(11));
        let out = rendered(&mut section, true, 10);
        assert_eq!(
            line_of(&out, "#11 task 11"),
            line_of(&out, "#5 Five") + 1,
            "{out}"
        );
    }

    #[test]
    fn in_a_one_line_pane_the_cursors_row_wins_over_its_header() {
        let mut section = AgentsSection::new();
        section.set_rows(vec![in_epic(1, 3, &["Three"]), in_epic(2, 5, &["Five"])]);
        section.down();
        for _ in 0..2 {
            let out = rendered(&mut section, true, 3);
            assert!(out.contains("#2 task 2"), "{out}");
        }
    }

    #[test]
    fn at_the_top_the_first_groups_header_is_in_view() {
        let mut section = AgentsSection::new();
        let mut rows: Vec<AgentRow> = (1..=10).map(|id| in_epic(id, 3, &["Three"])).collect();
        rows.extend((11..=20).map(|id| in_epic(id, 5, &["Five"])));
        section.set_rows(rows);
        section.bottom();
        rendered(&mut section, true, 10);
        section.top();
        let out = rendered(&mut section, true, 10);
        assert!(out.contains("#3 Three"), "{out}");
    }

    // ---- label -----------------------------------------------------------

    #[test]
    fn a_row_with_an_agent_name_leads_with_it_and_not_the_id() {
        let mut section = AgentsSection::new();
        section.set_rows(vec![AgentRow {
            agent_name: "scar".into(),
            ..row(4941, false)
        }]);
        let out = rendered(&mut section, true, 6);
        assert!(out.contains("scar task 4941"), "{out}");
        assert!(!out.contains("#4941"), "{out}");
    }

    #[test]
    fn a_row_without_an_agent_name_leads_with_the_id() {
        let mut section = AgentsSection::new();
        section.set_rows(vec![row(4941, false)]);
        let out = rendered(&mut section, true, 6);
        assert!(out.contains("#4941 task 4941"), "{out}");
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

    // ---- JumpToAgentWindow / ResumeDetachedAgentFromThePane --------------

    #[test]
    fn space_on_an_attached_agent_jumps_to_its_window() {
        let mut section = AgentsSection::new();
        section.set_rows(vec![row(1, false), row(2, true)]);
        assert_eq!(
            section.space_action(),
            Some(KeyAction::JumpTo(test_tmux_window("task-1")))
        );
    }

    #[test]
    fn the_own_task_has_no_space_action() {
        let mut section = AgentsSection::new();
        section.set_rows(vec![row(1, false), row(2, true)]);
        section.down();
        assert_eq!(section.space_action(), None);
    }

    #[test]
    fn space_on_a_detached_agent_resumes_it() {
        let mut section = AgentsSection::new();
        section.set_rows(vec![test_detached_row(3)]);
        assert_eq!(section.space_action(), Some(KeyAction::Resume(TaskId(3))));
    }

    // ---- TwoViewsShareThePane / AgentRowShowsAgentName ----------

    #[test]
    fn every_row_leads_with_its_dot_and_the_own_task_has_no_trailing_mark() {
        let mut section = AgentsSection::new();
        section.set_rows(vec![
            row(4941, false),
            row(4942, true),
            test_detached_row(4943),
        ]);
        let out = rendered(&mut section, true, 5);
        assert!(out.contains("Agents"), "{out}");
        assert!(out.contains("● #4941 task 4941"), "{out}");
        assert!(out.contains("● #4942 task 4942"), "{out}");
        assert!(!out.contains("task 4942 ●"), "{out}");
        assert!(out.contains("○ #4943 task 4943"), "{out}");
    }

    #[test]
    fn a_detached_row_is_drawn_in_the_ordinary_colour_and_the_own_row_dimmed() {
        use crate::palette::MUTED;
        let mut section = AgentsSection::new();
        section.set_rows(vec![row(1, true), test_detached_row(2)]);
        let mut terminal = Terminal::new(TestBackend::new(30, 4)).expect("terminal");
        terminal
            .draw(|frame| render_agents(frame, frame.area(), &mut section, false, false, None))
            .expect("draw");
        let buffer = terminal.backend().buffer();
        // Row 1 (inside the border) is the own task, row 2 the detached one;
        // column 3 is past the dot, on the label.
        assert_eq!(buffer[(3, 1)].fg, MUTED);
        assert_ne!(buffer[(3, 2)].fg, MUTED);
    }

    #[test]
    fn the_title_names_tab_whichever_section_has_focus() {
        // TwoViewsShareThePane: the hint names the key that leaves the view.
        let mut section = AgentsSection::new();
        for focused in [true, false] {
            let out = rendered(&mut section, focused, 3);
            assert!(
                out.contains("Agents (Tab: browse)"),
                "focused={focused}: {out}"
            );
        }
    }

    #[test]
    fn in_a_short_pane_the_section_scrolls_to_keep_the_cursor_in_view() {
        let mut section = AgentsSection::new();
        section.set_rows((1..=20).map(|id| row(id, false)).collect());
        section.bottom();
        let out = rendered(&mut section, true, 10);
        assert!(out.contains("#20 task 20"), "{out}");
        assert!(!out.contains("#1 task 1 "), "{out}");
    }

    #[test]
    fn scrolling_up_to_a_nested_groups_first_agent_shows_every_header_above_it() {
        let mut nested = test_pane_epic(30, &["Thirty"]);
        nested.ancestors = vec![crate::hooks::wire::PaneEpicRef {
            id: crate::models::EpicId(10),
            title: "Ten".into(),
        }];
        // A second branch under 10, so 10 draws (two branches meet there).
        let mut sibling = test_pane_epic(40, &["Forty"]);
        sibling.ancestors = nested.ancestors.clone();
        let mut section = AgentsSection::new();
        section.set_rows(vec![
            in_epic(1, 5, &["Five"]),
            in_epic(2, 5, &["Five"]),
            in_epic(3, 5, &["Five"]),
            AgentRow {
                epic: Some(nested),
                ..row(4, false)
            },
            AgentRow {
                epic: Some(sibling),
                ..row(6, false)
            },
        ]);
        section.bottom();
        section.up();
        // Scrolled past the nested group's headers: lines 0..=6 are above.
        section.offset = 7;
        let out = rendered(&mut section, true, 6);
        assert!(out.contains("#10 Ten"), "root header hidden:\n{out}");
        assert!(out.contains("#30 Thirty"), "{out}");
        assert!(out.contains("#4 task 4"), "{out}");
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
