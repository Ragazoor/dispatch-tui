//! A flattened column lays its epic groups out as the compact epic tree:
//! a header only for an epic that owns a task in the section or where two
//! drawn branches meet, leaves before deeper siblings, a depth on each
//! header, and folds that hide a whole subtree.
//!
//! See "Flattening" (EpicGroupTree) and "Epic Folding" in
//! `docs/specs/board-layout.allium` (tasks #98901, #99033).
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

/// A child epic's group follows its parent's own tasks. A top-level epic
/// with no drawn child comes before one with a child, whatever the ids.
#[test]
fn a_leaf_group_comes_before_a_group_with_children() {
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
            ('H', 20, 0),
            ('T', 2, 0),
            ('H', 10, 0),
            ('T', 1, 0),
            ('H', 30, 1),
            ('T', 3, 0),
        ]
    );
}

/// A lone group skips the ancestors only it passes through: its own header
/// draws flush, with nothing above it.
#[test]
fn a_lone_group_draws_flush_without_its_ancestors() {
    let app = flat_app(
        vec![owned(3, 30)],
        vec![epic_under(10, None), epic_under(30, Some(10))],
    );
    assert_eq!(running_layout(&app), vec![('H', 30, 0), ('T', 3, 0)]);
}

/// Groups that share no parent all draw flush, each under its own header.
#[test]
fn groups_with_no_shared_parent_draw_flush() {
    let app = flat_app(
        vec![owned(1, 11), owned(2, 21)],
        vec![
            epic_under(10, None),
            epic_under(11, Some(10)),
            epic_under(20, None),
            epic_under(21, Some(20)),
        ],
    );
    assert_eq!(
        running_layout(&app),
        vec![('H', 11, 0), ('T', 1, 0), ('H', 21, 0), ('T', 2, 0)]
    );
}

/// The elicitation example: A › B › X › C, A › B › D, A › E. A and B draw
/// because branches meet there; X only one branch passes through, so C sits
/// one step under B. E, a leaf, comes before B, which has children.
#[test]
fn headers_draw_only_where_branches_meet() {
    let (a, b, x, c, d, e) = (1, 2, 3, 4, 5, 6);
    let app = flat_app(
        vec![owned(11, c), owned(12, d), owned(13, e)],
        vec![
            epic_under(a, None),
            epic_under(b, Some(a)),
            epic_under(x, Some(b)),
            epic_under(c, Some(x)),
            epic_under(d, Some(b)),
            epic_under(e, Some(a)),
        ],
    );
    assert_eq!(
        running_layout(&app),
        vec![
            ('H', a, 0),
            ('H', e, 1),
            ('T', 13, 0),
            ('H', b, 1),
            ('H', c, 2),
            ('T', 11, 0),
            ('H', d, 2),
            ('T', 12, 0),
        ]
    );
}

/// The tree is built per section: two groups that share a parent but sit
/// in different sections each draw flush, with no shared header.
#[test]
fn shared_parents_are_found_per_section() {
    let mut waiting = owned(2, 30);
    waiting.sub_status = SubStatus::NeedsInput;
    let app = flat_app(
        vec![owned(1, 20), waiting],
        vec![
            epic_under(10, None),
            epic_under(20, Some(10)),
            epic_under(30, Some(10)),
        ],
    );
    let layout = running_layout(&app);
    assert!(!layout.contains(&('H', 10, 0)), "{layout:?}");
    assert!(layout.contains(&('H', 20, 0)), "{layout:?}");
    assert!(layout.contains(&('H', 30, 0)), "{layout:?}");
}

/// Under a shared parent, a sibling with no drawn child comes before one
/// with a child; a grandchild sits one level deeper than its parent.
#[test]
fn leaf_siblings_come_first_and_depth_grows_per_level() {
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
            ('H', 40, 1),
            ('T', 1, 0),
            ('H', 30, 1),
            ('T', 2, 0),
            ('H', 50, 2),
            ('T', 3, 0),
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
        vec![('H', 20, 0), ('T', 2, 0), ('F', 10, 0)]
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

/// A folded parent with no task of its own keeps its header while branches
/// still meet at it.
#[test]
fn a_folded_parent_with_no_own_task_keeps_its_header_while_branches_meet() {
    let mut app = flat_app(
        vec![owned(3, 30), owned(4, 40)],
        vec![
            epic_under(10, None),
            epic_under(30, Some(10)),
            epic_under(40, Some(10)),
        ],
    );
    app.epic_folds.toggle(TaskStatus::Running, EpicId(10));
    assert_eq!(running_layout(&app), vec![('F', 10, 0)]);
}

/// A fold on an epic the section skips hides nothing: the tasks below it
/// draw under their own header.
#[test]
fn a_fold_on_a_skipped_epic_hides_nothing() {
    let mut app = flat_app(
        vec![owned(3, 30)],
        vec![epic_under(10, None), epic_under(30, Some(10))],
    );
    app.epic_folds.toggle(TaskStatus::Running, EpicId(10));
    assert_eq!(running_layout(&app), vec![('H', 30, 0), ('T', 3, 0)]);
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

/// Unfolding a parent with no card of its own puts the cursor on the first
/// card of its subtree (board-layout.allium: "Epic Folding").
#[test]
fn unfolding_a_parent_with_no_own_card_lands_on_its_subtrees_first_card() {
    // 10 has two sub-epics with work, so it draws; as the one root with
    // children it comes after the leaf roots 5 and 20.
    let mut app = flat_app(
        vec![
            owned(1, 5),
            owned(6, 5),
            owned(3, 30),
            owned(4, 40),
            owned(2, 20),
        ],
        vec![
            epic_under(5, None),
            epic_under(10, None),
            epic_under(20, None),
            epic_under(30, Some(10)),
            epic_under(40, Some(10)),
        ],
    );
    app.epic_folds.toggle(TaskStatus::Running, EpicId(10));
    app.selection_mut().set_column(2); // Running
    for _ in 0..3 {
        app.handle_key(make_key(crossterm::event::KeyCode::Char('j')));
    }
    assert!(
        matches!(app.selected_column_item(), Some(ColumnItem::FoldedEpic(h)) if h.epic.id == EpicId(10)),
        "set-up: the cursor is on the folded parent, got {:?}",
        app.selected_column_item()
    );

    app.handle_key(make_key(crossterm::event::KeyCode::Enter));

    assert!(!app.epic_folds.is_folded(TaskStatus::Running, EpicId(10)));
    match app.selected_column_item() {
        Some(ColumnItem::Task(t)) => assert_eq!(t.id.0, 3),
        other => panic!("cursor should rest on the child's card, got {other:?}"),
    }
}

/// A folded child right under its parent's open header draws no blank line
/// between the two.
#[test]
fn a_folded_child_sits_directly_under_its_parents_header() {
    let mut ten = make_epic_with_title(10, "Ten");
    ten.parent_epic_id = None;
    let mut thirty = make_epic_with_title(30, "Thirty");
    thirty.parent_epic_id = Some(EpicId(10));
    let mut forty = make_epic_with_title(40, "Forty");
    forty.parent_epic_id = Some(EpicId(10));
    let mut app = flat_app(vec![owned(3, 30), owned(4, 40)], vec![ten, thirty, forty]);
    app.epic_folds.toggle(TaskStatus::Running, EpicId(30));

    let buf = render_to_buffer(&mut app, 160, 40);
    let (_, y10) = find_text(&buf, "#10 Ten").expect("parent header drawn");
    let (_, y30) = find_text(&buf, "#30 Thirty").expect("folded child drawn");
    assert_eq!(y30, y10 + 1);
}

/// A task naming an epic the board does not hold is an orphan: below the
/// separator, never drawn as part of the last epic group.
#[test]
fn a_task_with_an_unknown_epic_goes_below_the_separator() {
    let app = flat_app(
        vec![owned(3, 30), owned(4, 99)],
        vec![epic_under(10, None), epic_under(30, Some(10))],
    );
    let items = app.view().column_items_for_status(TaskStatus::Running);
    let separator = items
        .iter()
        .position(|i| matches!(i, ColumnItem::OrphanSeparator))
        .expect("separator drawn");
    let stray = items
        .iter()
        .position(|i| matches!(i, ColumnItem::Task(t) if t.id.0 == 4))
        .expect("task drawn");
    assert!(stray > separator, "{items:?}");
}

// ---- the fold moves down (tasks.allium: FoldMovesToDrawnDescendant) ----

/// The epics of the fold-move tests: A › B › C, A › B › D, A › E.
fn fold_move_epics() -> Vec<Epic> {
    vec![
        epic_under(1, None),
        epic_under(2, Some(1)),
        epic_under(4, Some(2)),
        epic_under(5, Some(2)),
        epic_under(6, Some(1)),
    ]
}

/// Whether `cmds` save the folded set.
fn saves_epic_folds(cmds: &[crate::tui::Command]) -> bool {
    cmds.iter().any(|c| {
        matches!(
            c,
            crate::tui::Command::Settings(
                crate::tui::commands::SettingsCommand::PersistStringSetting { key, .. }
            ) if key == crate::tui::COLLAPSED_EPICS_KEY
        )
    })
}

/// A folded epic that stops drawing hands its fold to the epic that now
/// heads what it hid, and the moved set is saved. The old fold is cleared.
#[test]
fn a_fold_on_an_epic_that_stops_drawing_moves_to_the_epic_below() {
    let (a, b) = (1, 2);
    let mut app = flat_app(
        vec![owned(11, 4), owned(12, 5), owned(13, 6), owned(14, a)],
        fold_move_epics(),
    );
    app.epic_folds.toggle(TaskStatus::Running, EpicId(a));

    // A's own task and E's task finish: A no longer owns a task, and only
    // B's branch passes through it.
    let mut left: Vec<Task> = app.board.tasks.clone();
    for t in left.iter_mut().filter(|t| t.id.0 == 13 || t.id.0 == 14) {
        t.status = TaskStatus::Done;
    }
    let cmds = app.handle_refresh_tasks(left);

    assert!(!app.epic_folds.is_folded(TaskStatus::Running, EpicId(a)));
    assert!(app.epic_folds.is_folded(TaskStatus::Running, EpicId(b)));
    assert!(saves_epic_folds(&cmds), "{cmds:?}");
    assert_eq!(running_layout(&app), vec![('F', b, 0)]);
}

/// The move judges the column unfiltered: a search that skips the folded
/// epic on screen does not move its fold.
#[test]
fn a_search_never_moves_a_fold() {
    let a = 1;
    let mut app = flat_app(
        vec![owned(11, 4), owned(12, 5), owned(13, 6)],
        fold_move_epics(),
    );
    app.epic_folds.toggle(TaskStatus::Running, EpicId(a));
    app.search.query = "task 11".into();

    let mut same = app.board.tasks.clone();
    same[0].title = "task 11 renamed".into();
    let cmds = app.handle_refresh_tasks(same);

    assert!(app.epic_folds.is_folded(TaskStatus::Running, EpicId(a)));
    assert!(!saves_epic_folds(&cmds), "{cmds:?}");
}

/// With no task left below the folded epic, there is nothing to move to:
/// the fold stays recorded and inert.
#[test]
fn a_fold_with_nothing_below_stays_put() {
    let a = 1;
    let mut app = flat_app(vec![owned(11, 4)], fold_move_epics());
    app.epic_folds.toggle(TaskStatus::Running, EpicId(a));

    let mut gone = app.board.tasks.clone();
    gone[0].status = TaskStatus::Done;
    let cmds = app.handle_refresh_tasks(gone);

    assert!(app.epic_folds.is_folded(TaskStatus::Running, EpicId(a)));
    assert!(!saves_epic_folds(&cmds), "{cmds:?}");
}

/// An epic refresh moves a fold whose epic stopped drawing, with no task
/// refresh needed (tasks.allium: FoldMovesToDrawnDescendant).
#[test]
fn an_epic_refresh_moves_a_fold_that_no_longer_draws() {
    let a = 1;
    let mut app = flat_app(
        vec![owned(11, 4), owned(12, 5), owned(14, a)],
        fold_move_epics(),
    );
    app.epic_folds.toggle(TaskStatus::Running, EpicId(a));
    // A owns task 14, so it draws. Remove it: only B's branch is left.
    let mut remaining = app.board.tasks.clone();
    remaining.retain(|t| t.id.0 != 14);
    let cmds = app.handle_refresh_epics(app.board.epics.clone());
    assert!(cmds.is_empty(), "a still draws: {cmds:?}");
    app.board.tasks = remaining;

    let cmds = app.handle_refresh_epics(app.board.epics.clone());

    assert!(!app.epic_folds.is_folded(TaskStatus::Running, EpicId(a)));
    assert!(app.epic_folds.is_folded(TaskStatus::Running, EpicId(2)));
    assert!(saves_epic_folds(&cmds), "{cmds:?}");
}
