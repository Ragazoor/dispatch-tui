//! Drawing the tree: line counts, the open marker and snapshots.

use super::*;

// ---- line counts on the rendered rows ---------------------------------

#[test]
fn a_file_row_shows_its_added_and_removed_line_counts() {
    let out = render_to_string(
        &[counted("a.rs", FileChange::Modified, 12, 3)],
        "task",
        60,
        8,
    );
    insta::assert_snapshot!("file_row_line_counts", out);
}

/// The foreground colour `offset` CELLS right of where `needle` starts, on
/// the first row that holds it. Used to check that a merged row's leading
/// segments are dimmed and its last segment is not.
///
/// Cell-wise rather than byte-wise on purpose: the row begins with the
/// pane border and an expansion arrow, both multi-byte, so a byte offset
/// into the joined line lands several columns past the name.
fn row_fg_at(buffer: &ratatui::buffer::Buffer, needle: &str, offset: u16) -> Option<Color> {
    let area = *buffer.area();
    for y in area.top()..area.bottom() {
        let cells: Vec<&str> = (area.left()..area.right())
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        for start in 0..cells.len() {
            // The needle is consumed cell by cell. Joining the row tail
            // instead would reallocate it once per start column.
            let mut rest = needle;
            for cell in &cells[start..] {
                let Some(tail) = rest.strip_prefix(*cell) else {
                    break;
                };
                rest = tail;
                if rest.is_empty() {
                    let x = area.left() + u16::try_from(start).ok()? + offset;
                    return buffer[(x, y)].style().fg;
                }
            }
        }
    }
    None
}

/// docs/specs/agent-tree.allium's CountsShowOnTheFolderNearestTheFiles: a
/// directory that holds another directory leaves the counts to the row
/// further in. Before this the same `+40 -10` appeared on every folder on
/// the way down, which is what made a deep tree unreadable in a narrow
/// pane.
#[test]
fn a_directory_holding_a_subdirectory_shows_no_counts_while_expanded() {
    let out = render_to_string(
        &[
            counted("src/main.rs", FileChange::Modified, 12, 0),
            counted("src/cli/a.rs", FileChange::Modified, 40, 10),
        ],
        "task",
        60,
        10,
    );

    let src_row = out
        .lines()
        .find(|l| l.contains("▼ src") && !l.contains("cli"))
        .unwrap_or_else(|| panic!("no src row in:\n{out}"));
    assert!(
        !src_row.contains('+') && !src_row.contains('-'),
        "src holds a subdirectory and must show no counts; row: {src_row:?}\n{out}"
    );
    assert!(
        out.lines().any(|l| l.contains("cli") && l.contains("+40")),
        "cli is nearest the files and must carry its counts:\n{out}"
    );
}

/// The exception, and the reason the sum is still derived for every
/// directory: a collapsed row summarises rows that are not on screen.
#[test]
fn a_collapsed_directory_shows_its_full_sum_however_deep() {
    let mut rig = KeyRig::new(&[
        counted("src/main.rs", FileChange::Modified, 12, 0),
        counted("src/cli/a.rs", FileChange::Modified, 40, 10),
    ]);
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["src".to_string()]);
    rig.press(KeyCode::Char('h'));

    let out = rig.rendered();
    insta::assert_snapshot!("collapsed_directory_full_sum", out);
}

/// The merged row is the folder nearest the files, so it carries them.
#[test]
fn a_merged_row_shows_the_counts_of_the_files_it_holds() {
    let out = render_to_string(
        &[counted("a/b/c/d.rs", FileChange::Modified, 7, 2)],
        "task",
        60,
        10,
    );
    let row = out
        .lines()
        .find(|l| l.contains("a/b/c"))
        .unwrap_or_else(|| panic!("no merged row in:\n{out}"));
    assert!(row.contains("+7"), "expected +7 on the merged row: {row:?}");
    assert!(row.contains("-2"), "expected -2 on the merged row: {row:?}");
}

/// The dim half of MergedDirectoryChainRows: the route recedes, the folder
/// the files are actually in does not. Asserted on the styled buffer,
/// because the whole point is a colour difference the plain text cannot
/// carry.
#[test]
fn a_merged_row_dims_every_segment_but_the_last() {
    let tree = build_tree(&root(), &[modified("a/b/c/d.rs")]);
    let mut state = RenderState::new();
    state.sync_expansion(&tree);
    let mut terminal = Terminal::new(TestBackend::new(60, 10)).expect("terminal");
    terminal
        .draw(|frame| render(frame, frame.area(), &tree, &mut state, "task"))
        .expect("draw");
    let buffer = terminal.backend().buffer();

    // "a/b/c" — the leading "a/b/" is the route, "c" is the folder.
    assert_eq!(
        row_fg_at(buffer, "a/b/c", 0),
        Some(MUTED),
        "the route must be dimmed"
    );
    assert_eq!(
        row_fg_at(buffer, "a/b/c", 4),
        Some(FG),
        "the last segment must keep the ordinary directory colour"
    );
}

/// The spec's `MergedRoutesClipAtThePaneEdge`. A route wider than the pane
/// is cut at the right edge — not wrapped onto a second row, which would
/// put one directory on two rows and break the one-row-per-node reading
/// the cursor and the expansion keys depend on.
///
/// Asserted on the rendered rows because that is the only place clipping
/// happens: the label is built at full width either way.
#[test]
fn a_route_wider_than_the_pane_is_clipped_not_wrapped() {
    let out = render_to_string(
        &[counted(
            "aaaaaaaa/bbbbbbbb/cccccccc/dddddddd/leaf.rs",
            FileChange::Modified,
            7,
            2,
        )],
        "task",
        24,
        8,
    );

    let route_rows = out.lines().filter(|l| l.contains("aaaaaaaa")).count();
    assert_eq!(
        route_rows, 1,
        "the route must occupy exactly one row:\n{out}"
    );
    assert!(
        out.lines().all(|l| l.chars().count() <= 24),
        "no row may exceed the pane width:\n{out}"
    );
    // The tail of the route, and the counts behind it, are what is lost.
    assert!(
        !out.contains("dddddddd"),
        "expected the route cut at the edge:\n{out}"
    );
}

/// An unmerged directory has no route to dim, so it is drawn exactly as
/// before — one span, ordinary colour.
#[test]
fn an_unmerged_directory_row_is_not_dimmed() {
    let tree = build_tree(&root(), &[modified("src/a.rs")]);
    let mut state = RenderState::new();
    state.sync_expansion(&tree);
    let mut terminal = Terminal::new(TestBackend::new(60, 10)).expect("terminal");
    terminal
        .draw(|frame| render(frame, frame.area(), &tree, &mut state, "task"))
        .expect("draw");

    assert_eq!(
        row_fg_at(terminal.backend().buffer(), "src", 0),
        Some(FG),
        "an unmerged directory name is not a route"
    );
}

/// A collapsed directory has to say how much is inside it, or the counts
/// are only useful once the user has already opened everything.
#[test]
fn a_directory_row_shows_the_sum_of_its_descendants() {
    let out = render_to_string(
        &[
            counted("src/a.rs", FileChange::Modified, 12, 3),
            counted("src/b.rs", FileChange::Added, 5, 1),
        ],
        "task",
        60,
        10,
    );
    insta::assert_snapshot!("summed_line_counts", out);
}

/// An untracked file has no counts and must show none — not "+0 -0", which
/// would read as "nothing changed in there" about a file the agent just
/// wrote. See the spec's UntrackedFilesHaveNoLineCounts.
#[test]
fn an_untracked_file_row_shows_no_counts_at_all() {
    let out = render_to_string(&[added("brand_new.rs")], "task", 60, 8);
    assert!(out.contains("brand_new.rs"), "expected the row in:\n{out}");
    assert!(!out.contains("+0"), "expected no +0 in:\n{out}");
    assert!(!out.contains("-0"), "expected no -0 in:\n{out}");
}

/// Zero is a real answer for a TRACKED file — a permission-only change
/// moves no lines — and is shown, unlike the absent counts above.
#[test]
fn a_tracked_file_with_no_moved_lines_still_shows_zeroes() {
    let out = render_to_string(
        &[counted("mode.sh", FileChange::Modified, 0, 0)],
        "task",
        60,
        8,
    );
    assert!(out.contains("+0"), "expected +0 in:\n{out}");
}

/// The badge still renders beside the counts. The two answer different
/// questions — what happened, and how much of it — and a row needs both.
#[test]
fn counts_render_alongside_the_badge_not_instead_of_it() {
    let out = render_to_string(
        &[counted("a.rs", FileChange::Modified, 2, 1)],
        "task",
        60,
        8,
    );
    insta::assert_snapshot!("counts_alongside_badge", out);
}

#[test]
fn q_exits_the_renderer() {
    let mut rig = KeyRig::new(&[]);
    assert_eq!(rig.press(KeyCode::Char('q')), KeyAction::Exit);
}

#[test]
fn ctrl_c_exits_the_renderer() {
    let mut state = RenderState::new();
    let tree = build_tree(&root(), &[]);
    let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(handle_key(&mut state, &tree, key), KeyAction::Exit);
}

#[test]
fn a_bare_c_does_not_exit_the_renderer() {
    let mut rig = KeyRig::new(&[]);
    assert_eq!(rig.press(KeyCode::Char('c')), KeyAction::Continue);
}

#[test]
fn down_and_j_both_move_the_cursor_down() {
    for code in [KeyCode::Down, KeyCode::Char('j')] {
        let mut rig = KeyRig::new(&three_node_changes());
        assert_eq!(rig.press(code), KeyAction::Continue);
        assert_eq!(rig.selected(), vec!["a.rs".to_string()], "{code:?}");
        rig.press(code);
        assert_eq!(rig.selected(), vec!["z.rs".to_string()], "{code:?}");
    }
}

#[test]
fn up_and_k_both_move_the_cursor_up() {
    for code in [KeyCode::Up, KeyCode::Char('k')] {
        let mut rig = KeyRig::new(&three_node_changes());
        rig.press(KeyCode::Down);
        rig.press(KeyCode::Down);
        assert_eq!(rig.selected(), vec!["z.rs".to_string()], "{code:?}");
        assert_eq!(rig.press(code), KeyAction::Continue);
        assert_eq!(rig.selected(), vec!["a.rs".to_string()], "{code:?}");
    }
}

#[test]
fn right_and_l_both_expand_the_selected_directory() {
    for code in [KeyCode::Right, KeyCode::Char('l')] {
        let mut rig = KeyRig::new(&three_node_changes());
        // "src" auto-expanded on first sync; collapse it so expanding is
        // an observable change. Three rows down: a.rs, z.rs, src.
        rig.press(KeyCode::Down);
        rig.press(KeyCode::Down);
        rig.press(KeyCode::Down);
        assert_eq!(rig.selected(), vec!["src".to_string()], "{code:?}");
        assert!(rig.state.tree_state.close(&["src".to_string()]));
        rig.draw();
        assert!(!rig.is_open(&["src"]), "{code:?}");

        assert_eq!(rig.press(code), KeyAction::Continue);
        assert!(rig.is_open(&["src"]), "{code:?}");
    }
}

#[test]
fn left_and_h_both_collapse_the_selected_directory() {
    for code in [KeyCode::Left, KeyCode::Char('h')] {
        let mut rig = KeyRig::new(&three_node_changes());
        rig.press(KeyCode::Down);
        rig.press(KeyCode::Down);
        rig.press(KeyCode::Down);
        assert_eq!(rig.selected(), vec!["src".to_string()], "{code:?}");
        assert!(rig.is_open(&["src"]), "{code:?}");

        assert_eq!(rig.press(code), KeyAction::Continue);
        assert!(!rig.is_open(&["src"]), "{code:?}");
    }
}

#[test]
fn h_on_a_child_moves_the_cursor_to_its_parent() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    // A node is identified by its own name, so a child's selection path is
    // [parent, child] — see `build_tree_items`.
    assert_eq!(
        rig.selected(),
        vec!["src".to_string(), "lib.rs".to_string()]
    );

    rig.press(KeyCode::Char('h'));
    assert_eq!(rig.selected(), vec!["src".to_string()]);
}

#[test]
fn space_and_enter_both_toggle_the_selected_directory() {
    for code in [KeyCode::Char(' '), KeyCode::Enter] {
        let mut rig = KeyRig::new(&three_node_changes());
        rig.press(KeyCode::Char('j'));
        rig.press(KeyCode::Char('j'));
        rig.press(KeyCode::Char('j'));
        assert_eq!(rig.selected(), vec!["src".to_string()], "{code:?}");

        assert_eq!(rig.press(code), KeyAction::Continue);
        assert!(!rig.is_open(&["src"]), "{code:?}");
        rig.press(code);
        assert!(rig.is_open(&["src"]), "{code:?}");
    }
}

/// No key expands a file — `tui_tree_widget`'s `open()` has no leaf guard, so
/// an unguarded press on a file inserts that file's path into `opened()`
/// (#3834). Nothing renders differently, which is exactly why this has to be
/// asserted on the open set.
///
/// Space/Enter now *open* a file rather than doing nothing, but that is an
/// action for the loop to perform; the pane's own expansion state must still
/// come out untouched, which is what this covers for all four keys. The
/// returned action differs per key and is asserted by the tests above.
///
/// Asserted over the *whole* set, not just the file's own path, so no
/// sibling directory's expansion is disturbed either. One press per rig:
/// `l` followed by Space on the same file would cancel out (open, then
/// toggle closed) and pass even unguarded.
#[test]
fn no_key_expands_a_file_in_the_open_set() {
    for code in [
        KeyCode::Char(' '),
        KeyCode::Enter,
        KeyCode::Char('l'),
        KeyCode::Right,
    ] {
        let mut rig = KeyRig::new(&three_node_changes());
        rig.press(KeyCode::Char('j'));
        assert_eq!(rig.selected(), vec!["a.rs".to_string()], "{code:?}");
        let before = rig.state.tree_state.opened().clone();

        rig.press(code);

        assert_eq!(rig.state.tree_state.opened(), &before, "{code:?}");
    }
}

/// The user-visible regression: a phantom open on a file is consumed by the
/// next `h`, so step-out silently needed two presses (#3834).
#[test]
fn h_after_space_on_a_file_steps_out_to_the_parent_in_one_press() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    assert_eq!(
        rig.selected(),
        vec!["src".to_string(), "lib.rs".to_string()]
    );
    rig.press(KeyCode::Char(' '));
    rig.press(KeyCode::Char('h'));
    assert_eq!(rig.selected(), vec!["src".to_string()]);
}

/// Space and Enter on a *file* open its diff. The action tells the loop
/// the set moved; `handle_key` stays pure, so splitting a pane and asking
/// git are the loop's job.
#[test]
fn space_and_enter_on_a_file_open_its_diff() {
    for code in [KeyCode::Char(' '), KeyCode::Enter] {
        let mut rig = KeyRig::new(&three_node_changes());
        rig.press(KeyCode::Char('j'));
        assert_eq!(rig.selected(), vec!["a.rs".to_string()], "{code:?}");

        assert_eq!(rig.press(code), KeyAction::DiffSetChanged, "{code:?}");
        assert!(rig.state.is_diff_open(Path::new("a.rs")), "{code:?}");
    }
}

/// The same key both ways — which is what makes it a toggle rather than an
/// open with a separate close to remember.
#[test]
fn space_and_enter_on_an_open_file_close_its_diff() {
    for code in [KeyCode::Char(' '), KeyCode::Enter] {
        let mut rig = KeyRig::new(&three_node_changes());
        rig.press(KeyCode::Char('j'));
        rig.press(code);
        assert!(rig.state.is_diff_open(Path::new("a.rs")), "{code:?}");

        assert_eq!(rig.press(code), KeyAction::DiffSetChanged, "{code:?}");
        assert!(!rig.state.is_diff_open(Path::new("a.rs")), "{code:?}");
    }
}

#[test]
fn opening_a_nested_file_records_its_whole_relative_path() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    assert_eq!(
        rig.selected(),
        vec!["src".to_string(), "lib.rs".to_string()]
    );

    assert_eq!(rig.press(KeyCode::Enter), KeyAction::DiffSetChanged);
    assert!(rig.state.is_diff_open(Path::new("src/lib.rs")));
}

/// The reversal from the editor this replaced, and the resolution of the
/// spec's old ShowDeletedFileContent question. An editor given a deleted
/// path opens a misleading empty buffer; a DIFF of a deleted file is
/// exactly its former contents, so the deleted case is the one where
/// opening it is most useful.
#[test]
fn space_and_enter_on_a_deleted_file_open_its_diff_too() {
    for code in [KeyCode::Char(' '), KeyCode::Enter] {
        let mut rig = KeyRig::new(&[deleted("gone.rs")]);
        rig.press(KeyCode::Char('j'));
        assert_eq!(rig.selected(), vec!["gone.rs".to_string()], "{code:?}");

        assert_eq!(rig.press(code), KeyAction::DiffSetChanged, "{code:?}");
        assert!(rig.state.is_diff_open(Path::new("gone.rs")), "{code:?}");
        assert!(rig.state.notice.is_none(), "{code:?}: no refusal expected");
    }
}

/// Every badge opens. There is no per-badge guard left anywhere in this
/// path — see OpenAgentTreeFileDiff in docs/specs/agent-tree.allium.
#[test]
fn every_badge_opens_a_diff() {
    for change in [FileChange::Added, FileChange::Modified, FileChange::Deleted] {
        let mut rig = KeyRig::new(&[changed("a.rs", change)]);
        rig.press(KeyCode::Char('j'));
        assert_eq!(
            rig.press(KeyCode::Enter),
            KeyAction::DiffSetChanged,
            "{change:?}"
        );
        assert!(rig.state.is_diff_open(Path::new("a.rs")), "{change:?}");
    }
}

/// The directory behaviour is unchanged: Space/Enter still toggles
/// expansion, and must not put a directory in the open set. See
/// OnlyFilesOpenDiffs in docs/specs/agent-tree.allium.
#[test]
fn space_on_a_directory_toggles_expansion_and_opens_no_diff() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["src".to_string()]);

    assert_eq!(rig.press(KeyCode::Char(' ')), KeyAction::Continue);
    assert!(!rig.is_open(&["src"]));
    assert!(rig.state.open_diffs().is_empty());
}

#[test]
fn space_with_nothing_selected_does_nothing() {
    let mut rig = KeyRig::new(&three_node_changes());
    assert!(rig.selected().is_empty());
    assert_eq!(rig.press(KeyCode::Char(' ')), KeyAction::Continue);
    assert!(rig.state.open_diffs().is_empty());
}

// ---- the open marker on tree rows -------------------------------------

/// Render the tree with `state` as it stands, rather than fresh — the
/// marker is a function of the open set, which only a pressed key fills.
fn render_rig(rig: &mut KeyRig) -> String {
    rig.draw();
    buffer_to_string(rig.terminal.backend().buffer())
}

#[test]
fn an_open_files_row_carries_the_open_marker() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char(' '));

    let out = render_rig(&mut rig);
    assert!(
        out.contains("\u{25cf} a.rs"),
        "expected the marker on a.rs in:\n{out}"
    );
}

#[test]
fn a_closed_files_row_carries_no_marker() {
    let mut rig = KeyRig::new(&three_node_changes());
    let out = render_rig(&mut rig);
    assert!(
        !out.contains("\u{25cf}"),
        "expected no marker anywhere in:\n{out}"
    );
}

/// The marker follows the SET, so closing a file takes it away again —
/// which is what NodeDiffOpenMatchesOpenSet asks of the rendered row.
#[test]
fn closing_a_diff_removes_its_marker() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char(' '));
    rig.press(KeyCode::Char(' '));

    let out = render_rig(&mut rig);
    assert!(!out.contains("\u{25cf}"), "expected no marker in:\n{out}");
}

/// A directory can never be open, so it never carries the marker OR the
/// blank that keeps file names in one column.
#[test]
fn a_directory_row_carries_no_marker_and_no_blank() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('a'));

    let out = render_rig(&mut rig);
    assert!(
        !out.contains("\u{25cf} src"),
        "a directory must not be marked; got:\n{out}"
    );
    assert!(
        out.contains("\u{25cf} lib.rs"),
        "its file child must be; got:\n{out}"
    );
}

// ---- Snapshots ---------------------------------------------------------

#[test]
fn snapshot_notice_is_shown_in_the_bottom_border() {
    let tree = build_tree(&root(), &three_node_changes());
    let mut state = RenderState::new();
    state.sync_expansion(&tree);
    state.notice = Some(Notice::diff("could not split the diff pane"));
    let mut terminal = Terminal::new(TestBackend::new(50, 12)).expect("terminal");
    terminal
        .draw(|frame| render(frame, frame.area(), &tree, &mut state, "dispatch"))
        .expect("draw");
    let rendered = buffer_to_string(terminal.backend().buffer());

    assert!(
        rendered.contains("could not split"),
        "the notice must be visible; rendered:\n{rendered}"
    );
    insta::assert_snapshot!(rendered);
}

/// The border reddens with the notice — see AgentTreeNoticeRedensBorder.
/// Asserted on the styled buffer, not the plain text, because the whole
/// point is that the frame carries where a line of text does not.
#[test]
fn a_notice_reddens_the_whole_border() {
    let tree = build_tree(&root(), &three_node_changes());
    let mut state = RenderState::new();
    state.sync_expansion(&tree);
    let mut terminal = Terminal::new(TestBackend::new(50, 12)).expect("terminal");

    terminal
        .draw(|frame| render(frame, frame.area(), &tree, &mut state, "dispatch"))
        .expect("draw");
    let corner = terminal.backend().buffer()[(0, 0)].clone();
    assert_ne!(
        corner.style().fg,
        Some(RED),
        "no notice: the border must not be red"
    );

    state.notice = Some(Notice::git("git: fatal: unable to read index.lock"));
    terminal
        .draw(|frame| render(frame, frame.area(), &tree, &mut state, "dispatch"))
        .expect("draw");
    let buf = terminal.backend().buffer();
    for (x, y) in [(0u16, 0u16), (49, 0), (0, 11), (49, 11)] {
        assert_eq!(
            buf[(x, y)].style().fg,
            Some(RED),
            "corner ({x},{y}) must be red while a notice shows"
        );
    }
}

#[test]
fn snapshot_empty_tree_shows_bare_title() {
    let rendered = render_to_string(&[], "dispatch", 50, 10);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_added_modified_and_deleted_badges() {
    let changes = vec![
        added("src/new.rs"),
        modified("src/lib.rs"),
        deleted("README.md"),
    ];
    let rendered = render_to_string(&changes, "dispatch", 50, 12);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_nested_directories_auto_expanded() {
    let rendered = render_to_string(&[modified("a/b/c.rs")], "dispatch", 50, 12);
    insta::assert_snapshot!(rendered);
}

/// The shape task #4716 is about: merged routes on one row each, and the
/// counts on the folder nearest the files rather than on every folder
/// above it.
#[test]
fn snapshot_merged_routes_and_nearest_folder_counts() {
    let changes = vec![
        counted("docs/specs/agent-tree.allium", FileChange::Modified, 40, 3),
        counted("src/main.rs", FileChange::Modified, 12, 0),
        counted("src/cli/agent_tree.rs", FileChange::Modified, 80, 20),
        counted("src/tui/ui/kanban/columns.rs", FileChange::Modified, 40, 10),
    ];
    let rendered = render_to_string(&changes, "dispatch", 50, 12);
    insta::assert_snapshot!(rendered);
}

/// The only form that exercises the widget's own open-set lookup, and
/// so the only one that can catch a key-representation mismatch: an
/// assertion over `opened()` can encode a key that matches no node and
/// still pass, because `TreeState::open` reports success on it (#3811).
/// Every directory on the way to a changed file is expanded, so the
/// leaf is on screen with no keypresses.
#[test]
fn deeply_nested_changed_file_is_visible_without_manual_expansion() {
    let rendered = render_to_string(&[modified("a/b/c/d/leaf.rs")], "dispatch", 50, 12);
    assert!(
        rendered.contains("leaf.rs"),
        "the leaf must be visible unaided; rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains('▶'),
        "no directory may render collapsed; rendered:\n{rendered}"
    );
}

/// A change to a sibling can merge or unmerge a chain, which renames the
/// row and so changes the identifier the widget stores its expansion
/// under. The collapse must survive that: RefreshAgentTree says a manual
/// collapse is not overwritten by the next refresh, unconditionally.
///
/// Collapsed with the real key rather than by reaching into the widget,
/// because what the renderer remembers about a collapse is recorded from
/// the key press — a direct `close` records nothing and would test a path
/// no user can take.
#[test]
fn manual_collapse_survives_a_refresh_that_unmerges_the_chain() {
    let mut rig = KeyRig::new(&[modified("a/b/c.rs"), modified("a/b/d.rs")]);
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["a/b".to_string()]);
    rig.press(KeyCode::Char('h'));
    assert!(!rig.is_open(&["a/b"]), "precondition: the row is collapsed");

    // `a/z.rs` gives `a` a file of its own, so `a/b` unmerges into
    // `a` -> `b` and the collapsed row is re-keyed.
    rig.refresh(&[
        modified("a/b/c.rs"),
        modified("a/b/d.rs"),
        modified("a/z.rs"),
    ]);

    assert!(
        !rig.is_open(&["a", "b"]),
        "re-keying the row must not re-open it; opened: {:?}",
        rig.state.tree_state.opened()
    );
}

/// The mirror: the agent reverts the sibling, the chain merges again, and
/// the merged row must not spring open either.
#[test]
fn manual_collapse_survives_a_refresh_that_merges_the_chain() {
    let mut rig = KeyRig::new(&[modified("a/b/c.rs"), modified("a/z.rs")]);
    // Rows under `a` are its own file first, then the subfolder.
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["a".to_string(), "b".to_string()]);
    rig.press(KeyCode::Char('h'));
    assert!(!rig.is_open(&["a", "b"]), "precondition: b is collapsed");

    rig.refresh(&[modified("a/b/c.rs")]);

    assert!(
        !rig.is_open(&["a/b"]),
        "re-keying the row must not re-open it; opened: {:?}",
        rig.state.tree_state.opened()
    );
}

/// The OUTER link of a chain. The user collapses `a` while it has a file
/// of its own; the agent reverts that file; the chain merges into `a/b`.
/// The merged row stands for `a` as much as for `a/b`, so it must come up
/// collapsed — the row absorbed the one the user closed, it did not
/// replace it.
#[test]
fn manual_collapse_survives_the_row_being_absorbed_into_a_merged_one() {
    let mut rig = KeyRig::new(&[modified("a/x.rs"), modified("a/b/c.rs")]);
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["a".to_string()]);
    rig.press(KeyCode::Char('h'));
    assert!(!rig.is_open(&["a"]), "precondition: a is collapsed");

    rig.refresh(&[modified("a/b/c.rs")]);

    assert!(
        !rig.is_open(&["a/b"]),
        "the merged row absorbed the collapsed one; opened: {:?}",
        rig.state.tree_state.opened()
    );
}

/// The mirror of the test above. Having inherited a collapse, the row is
/// opened by hand — and that open must clear the collapse recorded on
/// every link the row stands for, or a later unmerge hands the collapse
/// back to the row it came from and discards what the user asked for. An
/// explicit open cannot be weaker than an explicit close.
#[test]
fn opening_a_merged_row_clears_the_collapse_it_inherited() {
    let mut rig = KeyRig::new(&[modified("a/x.rs"), modified("a/b/c.rs")]);
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["a".to_string()]);
    rig.press(KeyCode::Char('h'));

    // The chain merges; the row inherits the collapse (the test above).
    rig.refresh(&[modified("a/b/c.rs")]);
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["a/b".to_string()]);
    assert!(!rig.is_open(&["a/b"]), "precondition: inherited collapse");

    rig.press(KeyCode::Char('l'));
    assert!(rig.is_open(&["a/b"]), "precondition: the user opened it");

    // The sibling file comes back and the chain unmerges again.
    rig.refresh(&[modified("a/x.rs"), modified("a/b/c.rs")]);

    assert!(
        rig.is_open(&["a"]),
        "the open must not be discarded; opened: {:?}",
        rig.state.tree_state.opened()
    );
}

/// The other half of RefreshAgentTree's rule 3, and the mirror of the two
/// tests above: when NOBODY collapsed anything, a refresh that unmerges a
/// chain must leave the rows it creates expanded. A row the user never
/// touched, rendered collapsed, hides a changed file's badge on a refresh
/// the user did not ask for — which is the failure the whole pane exists
/// to prevent.
#[test]
fn a_refresh_that_unmerges_a_chain_leaves_the_new_rows_expanded() {
    let mut rig = KeyRig::new(&[modified("a/b/c.rs")]);
    rig.refresh(&[modified("a/b/c.rs"), modified("a/x.rs")]);

    assert!(
        rig.is_open(&["a"]),
        "opened: {:?}",
        rig.state.tree_state.opened()
    );
    assert!(
        rig.is_open(&["a", "b"]),
        "opened: {:?}",
        rig.state.tree_state.opened()
    );
    let rendered = rig.rendered();
    assert!(
        rendered.contains("c.rs"),
        "the badge below must be on screen unaided:\n{rendered}"
    );
}

#[test]
fn manually_collapsed_nested_directory_stays_collapsed_on_refresh() {
    // `a/x.rs` keeps `a` from merging into `b`, so there is a genuinely
    // nested directory to collapse. Rows: a, x.rs, b, c.rs.
    let changes = [modified("a/x.rs"), modified("a/b/c.rs")];
    let mut rig = KeyRig::new(&changes);
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["a".to_string(), "b".to_string()]);
    rig.press(KeyCode::Char('h'));
    assert!(!rig.is_open(&["a", "b"]));

    rig.refresh(&changes);
    assert!(!rig.is_open(&["a", "b"]));
}
