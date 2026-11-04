//! A flattened column lays its epic groups out as the epic tree: tree order,
//! a header for every drawn ancestor, a depth on each header, and folds that
//! hide a whole subtree.
//!
//! See "Flattening" (EpicGroupTree) and "Epic Folding" in
//! `docs/specs/board-layout.allium` (task #98901).
use super::*;
use crate::models::{EpicId, SubStatus, TaskStatus};
use crate::tui::types::Lens;

/// A Running task owned by `epic_id`.
fn owned(id: i64, epic_id: i64) -> Task {
    let mut t = make_task(id, TaskStatus::Running);
    t.sub_status = SubStatus::Active;
    t.epic_id = Some(EpicId(epic_id));
    t
}

fn epic_under(id: i64, parent: Option<i64>) -> Epic {
    let mut e = make_epic(id);
    e.parent_epic_id = parent.map(EpicId);
    e
}

/// The Running column as `(kind, id, depth)`: `H` an open epic header, `F` a
/// folded one (its id and depth), `T` a task card (depth 0, it has none).
fn running_layout(app: &App) -> Vec<(char, i64, usize)> {
    app.view()
        .column_items_for_status(TaskStatus::Running)
        .iter()
        .filter_map(|i| match i {
            ColumnItem::EpicHeader(h) => Some(('H', h.epic.id.0, h.depth)),
            ColumnItem::FoldedEpic(h) => Some(('F', h.epic.id.0, h.depth)),
            ColumnItem::Task(t) => Some(('T', t.id.0, 0)),
            _ => None,
        })
        .collect()
}

fn flat_app(tasks: Vec<Task>, epics: Vec<Epic>) -> App {
    let mut app = App::new(tasks);
    app.board.epics = epics;
    app.filter.lens = Lens::Flat;
    app
}

/// A child epic's group follows its parent's own tasks, before the next
/// top-level epic, even though its id sorts after that epic's.
#[test]
fn a_child_group_follows_its_parent_in_tree_order() {
    let app = flat_app(
        vec![owned(1, 10), owned(2, 20), owned(3, 30)],
        vec![
            epic_under(10, None),
            epic_under(20, None),
            epic_under(30, Some(10)),
        ],
    );
    assert_eq!(
        running_layout(&app),
        vec![
            ('H', 10, 0),
            ('T', 1, 0),
            ('H', 30, 1),
            ('T', 3, 0),
            ('H', 20, 0),
            ('T', 2, 0),
        ]
    );
}

/// An ancestor with no task of its own in the section still draws its header,
/// so its child never reads as unrelated.
#[test]
fn an_ancestor_with_no_task_of_its_own_still_draws_its_header() {
    let app = flat_app(
        vec![owned(3, 30)],
        vec![epic_under(10, None), epic_under(30, Some(10))],
    );
    assert_eq!(
        running_layout(&app),
        vec![('H', 10, 0), ('H', 30, 1), ('T', 3, 0)]
    );
}

/// Sibling sub-epics come in ascending id under their parent, and a grandchild
/// sits one level deeper than its parent.
#[test]
fn siblings_sort_by_id_and_depth_grows_per_level() {
    let app = flat_app(
        vec![owned(1, 40), owned(2, 30), owned(3, 50)],
        vec![
            epic_under(10, None),
            epic_under(40, Some(10)),
            epic_under(30, Some(10)),
            epic_under(50, Some(30)),
        ],
    );
    assert_eq!(
        running_layout(&app),
        vec![
            ('H', 10, 0),
            ('H', 30, 1),
            ('T', 2, 0),
            ('H', 50, 2),
            ('T', 3, 0),
            ('H', 40, 1),
            ('T', 1, 0),
        ]
    );
}

/// A parent that is not on the board does not count: the child is top-level.
#[test]
fn a_child_whose_parent_is_not_on_the_board_is_top_level() {
    let app = flat_app(vec![owned(3, 30)], vec![epic_under(30, Some(99))]);
    assert_eq!(running_layout(&app), vec![('H', 30, 0), ('T', 3, 0)]);
}

/// In an epic view the viewed epic is never an ancestor: its own tasks and
/// its direct sub-epics are both depth 0.
#[test]
fn in_an_epic_view_the_viewed_epics_children_are_top_level() {
    let mut app = flat_app(
        vec![owned(1, 10), owned(3, 30)],
        vec![epic_under(10, None), epic_under(30, Some(10))],
    );
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    assert_eq!(
        running_layout(&app),
        vec![('H', 10, 0), ('T', 1, 0), ('H', 30, 0), ('T', 3, 0)]
    );
}

/// Folding a parent hides its whole subtree in the section, and its count is
/// the subtree's tasks.
#[test]
fn folding_a_parent_hides_its_whole_subtree() {
    let mut app = flat_app(
        vec![owned(1, 10), owned(2, 20), owned(3, 30), owned(4, 30)],
        vec![
            epic_under(10, None),
            epic_under(20, None),
            epic_under(30, Some(10)),
        ],
    );
    app.epic_folds.toggle(TaskStatus::Running, EpicId(10));

    assert_eq!(
        running_layout(&app),
        vec![('F', 10, 0), ('H', 20, 0), ('T', 2, 0)]
    );
    let hidden = app
        .view()
        .column_items_for_status(TaskStatus::Running)
        .iter()
        .find_map(|i| match i {
            ColumnItem::FoldedEpic(h) => Some(h.hidden),
            _ => None,
        });
    assert_eq!(hidden, Some(3), "its own task and its child's two");
}

/// A folded parent with no task of its own keeps its header while a
/// descendant has a task in the section.
#[test]
fn a_folded_parent_with_no_own_task_keeps_its_header() {
    let mut app = flat_app(
        vec![owned(3, 30)],
        vec![epic_under(10, None), epic_under(30, Some(10))],
    );
    app.epic_folds.toggle(TaskStatus::Running, EpicId(10));
    assert_eq!(running_layout(&app), vec![('F', 10, 0)]);
}

/// Folding the parent leaves a child's own recorded fold alone: when the
/// parent unfolds, the child still draws folded, at its depth.
#[test]
fn unfolding_a_parent_keeps_its_childs_own_fold() {
    let mut app = flat_app(
        vec![owned(1, 10), owned(3, 30)],
        vec![epic_under(10, None), epic_under(30, Some(10))],
    );
    app.epic_folds.toggle(TaskStatus::Running, EpicId(30));
    app.epic_folds.toggle(TaskStatus::Running, EpicId(10));
    app.epic_folds.toggle(TaskStatus::Running, EpicId(10));
    assert_eq!(
        running_layout(&app),
        vec![('H', 10, 0), ('T', 1, 0), ('F', 30, 1)]
    );
}

/// The orphan rule still follows the last epic group, however deep.
#[test]
fn epicless_tasks_follow_the_whole_tree() {
    let mut orphan = make_task(5, TaskStatus::Running);
    orphan.sub_status = SubStatus::Active;
    let app = flat_app(
        vec![owned(3, 30), orphan],
        vec![epic_under(10, None), epic_under(30, Some(10))],
    );
    let items = app.view().column_items_for_status(TaskStatus::Running);
    let tail: Vec<bool> = items
        .iter()
        .rev()
        .take(2)
        .map(|i| matches!(i, ColumnItem::OrphanSeparator))
        .collect();
    assert_eq!(tail, vec![false, true], "{items:?}");
}

/// Where `needle` starts in the buffer: `(x, y)` of its first cell.
fn find_text(buf: &ratatui::buffer::Buffer, needle: &str) -> Option<(u16, u16)> {
    let area = buf.area();
    (area.top()..area.bottom()).find_map(|y| {
        let row: String = (area.left()..area.right())
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect();
        let at = row.find(needle)?;
        Some((row[..at].chars().count() as u16, y))
    })
}

/// The x of the first `╭` (a card's top-left corner) in row `y` at or right
/// of `from`.
fn card_corner(buf: &ratatui::buffer::Buffer, y: u16, from: u16) -> Option<u16> {
    (from..buf.area().right()).find(|&x| buf[(x, y)].symbol() == "\u{256d}")
}

/// A nested group's header and its cards both step in by the indent step:
/// the cards are indented with their header, not left flush.
#[test]
fn a_nested_groups_header_and_cards_are_indented_together() {
    let mut ten = make_epic_with_title(10, "Ten");
    ten.parent_epic_id = None;
    let mut thirty = make_epic_with_title(30, "Thirty");
    thirty.parent_epic_id = Some(EpicId(10));
    let mut app = flat_app(vec![owned(1, 10), owned(3, 30)], vec![ten, thirty]);

    let buf = render_to_buffer(&mut app, 160, 40);
    let (x10, y10) = find_text(&buf, "#10 Ten").expect("parent header drawn");
    let (x30, y30) = find_text(&buf, "#30 Thirty").expect("child header drawn");
    assert_eq!(x30, x10 + 2, "the child header steps in one level");

    let start = x10.saturating_sub(2);
    let c10 = card_corner(&buf, y10 + 1, start).expect("parent's card under its header");
    let c30 = card_corner(&buf, y30 + 1, start).expect("child's card under its header");
    assert_eq!(c30, c10 + 2, "the child's card steps in with its header");
}
