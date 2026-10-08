//! Key handling: the all-files key, jump motions and the keybinding table.

use super::*;

// ---- the all-files key ------------------------------------------------

#[test]
fn a_opens_every_changed_files_diff() {
    let mut rig = KeyRig::new(&three_node_changes());

    assert_eq!(rig.press(KeyCode::Char('a')), KeyAction::DiffSetChanged);

    assert!(rig.state.is_diff_open(Path::new("a.rs")));
    assert!(rig.state.is_diff_open(Path::new("src/lib.rs")));
}

/// MergedDirectoryChainRows: merging shortens what is drawn, never what is
/// opened. A file under a merged row must reach the open set under the
/// path it would have had with every intermediate row present — the diff
/// pane resolves that path against the worktree, so a shortened one would
/// open nothing.
#[test]
fn opening_a_file_under_a_merged_row_records_its_whole_relative_path() {
    let mut rig = KeyRig::new(&[modified("a/b/c/d.rs")]);
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["a/b/c".to_string()]);
    rig.press(KeyCode::Char('j'));

    assert_eq!(rig.press(KeyCode::Char(' ')), KeyAction::DiffSetChanged);
    assert!(
        rig.state.is_diff_open(Path::new("a/b/c/d.rs")),
        "open set: {:?}",
        rig.state.open_diffs()
    );
}

/// The bulk form walks the tree rather than the selection, so it is the
/// other place a merged name could truncate a path.
#[test]
fn a_records_whole_paths_through_merged_rows() {
    let mut rig = KeyRig::new(&[modified("a/b/c/d.rs"), modified("src/x.rs")]);
    rig.press(KeyCode::Char('a'));

    let open: Vec<&Path> = rig.state.open_diffs().iter().map(|p| p.as_path()).collect();
    assert_eq!(
        open,
        vec![Path::new("a/b/c/d.rs"), Path::new("src/x.rs")],
        "merged rows must not shorten the paths beneath them"
    );
}

/// The open marker is looked up by the same relative path, so a merged row
/// above a file must not cost it its marker.
#[test]
fn a_file_under_a_merged_row_still_gets_its_open_marker() {
    let mut open = BTreeSet::new();
    open.insert(PathBuf::from("a/b/c/d.rs"));
    let tree = build_tree(&root(), &[modified("a/b/c/d.rs")]);
    let mut state = RenderState::new();
    state.sync_expansion(&tree);
    let mut terminal = Terminal::new(TestBackend::new(60, 10)).expect("terminal");
    state.open_diffs = open;
    terminal
        .draw(|frame| render(frame, frame.area(), &tree, &mut state, "task"))
        .expect("draw");
    let rendered = buffer_to_string(terminal.backend().buffer());

    let row = rendered
        .lines()
        .find(|l| l.contains("d.rs"))
        .unwrap_or_else(|| panic!("no file row in:\n{rendered}"));
    assert!(
        row.contains('●'),
        "expected the open marker on {row:?}\n{rendered}"
    );
}

/// What the tree publishes to the diff pane: the open paths in ROW order,
/// which is not their sorted order. `z.rs` is a top-level file so its row
/// is above `src/lib.rs` (`RowsPutAFoldersOwnFilesFirst`), and the diff
/// pane renders what it is handed — it cannot re-derive this.
#[test]
fn the_published_open_set_is_in_row_order_not_path_order() {
    let mut rig = KeyRig::new(&[modified("a.rs"), modified("z.rs"), modified("src/lib.rs")]);
    rig.press(KeyCode::Char('a'));

    assert_eq!(
        rig.state.open_diffs_in_tree_order(&rig.tree),
        vec![
            PathBuf::from("a.rs"),
            PathBuf::from("z.rs"),
            PathBuf::from("src/lib.rs"),
        ]
    );
}

/// A path the tree no longer knows about is appended, not dropped: the open
/// set outlives its files on purpose (`OpenDiffPathsMaySurviveTheirFiles`),
/// and dropping it here would silently un-open a file the user opened.
#[test]
fn a_published_path_the_tree_no_longer_knows_is_kept_at_the_end() {
    let mut rig = KeyRig::new(&[modified("a.rs"), modified("src/lib.rs")]);
    rig.press(KeyCode::Char('a'));
    rig.refresh(&[modified("src/lib.rs")]);

    assert_eq!(
        rig.state.open_diffs_in_tree_order(&rig.tree),
        vec![PathBuf::from("src/lib.rs"), PathBuf::from("a.rs")],
        "the reverted file keeps its place in the set, at the end"
    );
}

/// Directories are routes to files, not things with contents, so `a` must
/// not put one in the set — see OnlyFilesOpenDiffs in the spec.
#[test]
fn a_opens_no_directories() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('a'));
    assert!(!rig.state.is_diff_open(Path::new("src")));
}

/// Direction is decided by the SET, not by a remembered mode: one press
/// always empties a non-empty set, however it came to be non-empty.
#[test]
fn a_closes_everything_when_anything_is_open() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('a'));
    assert!(!rig.state.open_diffs().is_empty());

    assert_eq!(rig.press(KeyCode::Char('a')), KeyAction::DiffSetChanged);
    assert!(rig.state.open_diffs().is_empty());
}

/// Even one file opened with Space is enough to make `a` mean "close",
/// because the set is what decides and the set is what the user can see.
#[test]
fn a_closes_a_set_that_space_filled() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char(' '));
    assert_eq!(rig.state.open_diffs().len(), 1);

    rig.press(KeyCode::Char('a'));
    assert!(rig.state.open_diffs().is_empty());
}

/// `a` does not dispatch on the selection at all, so it works with the
/// cursor parked on a directory or on nothing.
#[test]
fn a_acts_on_the_whole_tree_whatever_the_cursor_is_on() {
    let mut rig = KeyRig::new(&three_node_changes());
    assert!(rig.selected().is_empty());
    rig.press(KeyCode::Char('a'));
    assert!(rig.state.is_diff_open(Path::new("src/lib.rs")));

    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["src".to_string()]);
    rig.press(KeyCode::Char('a'));
    assert!(rig.state.is_diff_open(Path::new("a.rs")));
}

/// On an empty tree there is nothing to open, and the press is harmless.
#[test]
fn a_on_an_empty_tree_opens_nothing() {
    let mut rig = KeyRig::new(&[]);
    assert_eq!(rig.press(KeyCode::Char('a')), KeyAction::DiffSetChanged);
    assert!(rig.state.open_diffs().is_empty());
}

/// `l`/`Right` are expansion keys only — they must NOT have picked up the
/// open behaviour along with Space/Enter (#3834's guard stays a guard).
#[test]
fn l_and_right_on_a_file_still_do_nothing() {
    for code in [KeyCode::Char('l'), KeyCode::Right] {
        let mut rig = KeyRig::new(&three_node_changes());
        rig.press(KeyCode::Char('j'));
        assert_eq!(rig.press(code), KeyAction::Continue, "{code:?}");
    }
}

#[test]
fn any_key_clears_a_pending_notice() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.state.notice = Some(Notice::diff("could not split the diff pane"));

    rig.press(KeyCode::Char('j'));

    assert!(rig.state.notice.is_none());
}

// ---- Jump motions: gg, G, Ctrl-D, Ctrl-U ------------------------------
//
// See the AgentTreeCompanionPane surface and the
// AgentTreeGgChordNeverExpires guarantee in docs/specs/agent-tree.allium.

/// `count` top-level files, `f01.rs`..`fNN.rs`. Flat and zero-padded, so
/// the flattened view is exactly the files in that order and a landing row
/// is nameable without counting directories.
fn flat_file_changes(count: usize) -> Vec<GitFileChange> {
    (1..=count)
        .map(|n| modified(&format!("f{n:02}.rs")))
        .collect()
}

#[test]
fn gg_jumps_to_the_first_visible_node() {
    let mut rig = KeyRig::new(&flat_file_changes(6));
    for _ in 0..4 {
        rig.press(KeyCode::Char('j'));
    }
    assert_ne!(
        rig.selected_name(),
        "f01.rs",
        "precondition: moved off row 0"
    );

    rig.press(KeyCode::Char('g'));
    assert_eq!(rig.press(KeyCode::Char('g')), KeyAction::Continue);
    assert_eq!(rig.selected_name(), "f01.rs");
}

#[test]
fn a_lone_g_moves_nothing() {
    let mut rig = KeyRig::new(&flat_file_changes(6));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    let before = rig.selected();

    assert_eq!(rig.press(KeyCode::Char('g')), KeyAction::Continue);
    assert_eq!(rig.selected(), before, "a lone g must be swallowed");
}

/// The chord has no clock, so the only thing that can end it is another
/// key — and that key must still do its own job.
#[test]
fn a_key_between_the_two_gs_disarms_the_chord_and_still_acts() {
    let mut rig = KeyRig::new(&flat_file_changes(6));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));

    rig.press(KeyCode::Char('g'));
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected_name(), "f03.rs", "the j must still move down");

    rig.press(KeyCode::Char('g'));
    assert_eq!(
        rig.selected_name(),
        "f03.rs",
        "the disarmed chord must not complete on the next lone g"
    );
}

#[test]
fn g_then_a_second_g_after_many_other_keys_needs_a_fresh_pair() {
    let mut rig = KeyRig::new(&flat_file_changes(6));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('g'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));

    rig.press(KeyCode::Char('g'));
    rig.press(KeyCode::Char('g'));
    assert_eq!(rig.selected_name(), "f01.rs");
}

#[test]
fn shift_g_jumps_to_the_last_visible_node() {
    let mut rig = KeyRig::new(&flat_file_changes(6));
    assert_eq!(rig.press(KeyCode::Char('G')), KeyAction::Continue);
    assert_eq!(rig.selected_name(), "f06.rs");
}

/// A jump lands on a *visible* row. Collapsing `src` hides `lib.rs`, so
/// the last row becomes `src` itself — and the jump must not reopen it.
#[test]
fn shift_g_skips_rows_a_collapsed_directory_hides() {
    let mut rig = KeyRig::new(&[modified("a.rs"), modified("src/lib.rs")]);
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('h'));
    assert!(!rig.is_open(&["src"]), "precondition: src is collapsed");

    rig.press(KeyCode::Char('G'));
    assert_eq!(rig.selected_name(), "src");
    assert!(!rig.is_open(&["src"]), "a jump must not expand anything");
}

#[test]
fn gg_lands_on_the_first_row_without_expanding_it() {
    // Both siblings are directories, so the first row is one: a top-level
    // FILE would sort ahead of `src` (`RowsPutAFoldersOwnFilesFirst`) and
    // there would be nothing collapsible to land on.
    let mut rig = KeyRig::new(&[modified("src/lib.rs"), modified("z/z.rs")]);
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('h'));
    assert!(!rig.is_open(&["src"]), "precondition: src is collapsed");

    rig.press(KeyCode::Char('G'));
    rig.press(KeyCode::Char('g'));
    rig.press(KeyCode::Char('g'));
    assert_eq!(rig.selected_name(), "src");
    assert!(!rig.is_open(&["src"]), "a jump must not expand anything");
}

/// A 12-row pane has 10 visible rows, so half a page is 5. The leading `j`
/// establishes a selection: with nothing selected a half-page motion just
/// selects the first row, exactly as `j`/`k` do.
#[test]
fn ctrl_d_moves_the_cursor_half_a_page_down() {
    let mut rig = KeyRig::new(&flat_file_changes(20));
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.press_ctrl(KeyCode::Char('d')), KeyAction::Continue);
    assert_eq!(rig.selected_name(), "f06.rs");
    rig.press_ctrl(KeyCode::Char('d'));
    assert_eq!(rig.selected_name(), "f11.rs");
}

#[test]
fn ctrl_u_moves_the_cursor_half_a_page_up() {
    let mut rig = KeyRig::new(&flat_file_changes(20));
    rig.press(KeyCode::Char('G'));
    assert_eq!(rig.selected_name(), "f20.rs");

    assert_eq!(rig.press_ctrl(KeyCode::Char('u')), KeyAction::Continue);
    assert_eq!(rig.selected_name(), "f15.rs");
}

#[test]
fn ctrl_d_clamps_at_the_last_visible_node() {
    let mut rig = KeyRig::new(&flat_file_changes(6));
    rig.press(KeyCode::Char('j'));
    rig.press_ctrl(KeyCode::Char('d'));
    rig.press_ctrl(KeyCode::Char('d'));
    assert_eq!(rig.selected_name(), "f06.rs");
}

#[test]
fn ctrl_u_clamps_at_the_first_visible_node() {
    let mut rig = KeyRig::new(&flat_file_changes(6));
    rig.press(KeyCode::Char('j'));
    rig.press_ctrl(KeyCode::Char('u'));
    assert_eq!(rig.selected_name(), "f01.rs");
}

/// The distance is half the pane's *current* height, not a constant: a
/// 20-row pane (18 visible) jumps 9, where the default 12-row one jumps 5.
#[test]
fn half_a_page_scales_with_the_pane_height() {
    let mut tall = KeyRig::sized(&flat_file_changes(20), 20);
    tall.press(KeyCode::Char('j'));
    tall.press_ctrl(KeyCode::Char('d'));
    assert_eq!(tall.selected_name(), "f10.rs");

    let mut short = KeyRig::sized(&flat_file_changes(20), 8);
    short.press(KeyCode::Char('j'));
    short.press_ctrl(KeyCode::Char('d'));
    assert_eq!(short.selected_name(), "f04.rs");
}

/// A pane with a single visible row halves to zero. Zero is not a motion,
/// so the floor is one row.
#[test]
fn a_pane_too_short_to_halve_still_moves_one_row() {
    let mut rig = KeyRig::sized(&flat_file_changes(6), 3);
    rig.press(KeyCode::Char('j'));
    rig.press_ctrl(KeyCode::Char('d'));
    assert_eq!(rig.selected_name(), "f02.rs");
    rig.press_ctrl(KeyCode::Char('u'));
    assert_eq!(rig.selected_name(), "f01.rs");
}

#[test]
fn jump_motions_are_no_ops_on_an_empty_tree() {
    for keys in [vec!['g', 'g'], vec!['G']] {
        let mut rig = KeyRig::new(&[]);
        for key in keys {
            assert_eq!(rig.press(KeyCode::Char(key)), KeyAction::Continue);
        }
        assert!(rig.selected().is_empty());
    }

    let mut rig = KeyRig::new(&[]);
    assert_eq!(rig.press_ctrl(KeyCode::Char('d')), KeyAction::Continue);
    assert_eq!(rig.press_ctrl(KeyCode::Char('u')), KeyAction::Continue);
    assert!(rig.selected().is_empty());
}

/// `q` still exits with a chord armed — disarming must not swallow the key
/// that did the disarming.
#[test]
fn q_after_a_lone_g_still_exits() {
    let mut rig = KeyRig::new(&flat_file_changes(6));
    rig.press(KeyCode::Char('g'));
    assert_eq!(rig.press(KeyCode::Char('q')), KeyAction::Exit);
}

// ---- the keybinding table drives the pane (docs/specs/keybindings.allium) ----

/// A pane set up so `binding`'s context holds and its action takes effect.
fn rig_for(binding: &crate::keybindings::KeyBinding) -> KeyRig {
    let mut rig = KeyRig::with_agents(&three_node_changes(), vec![agent(1, false)]);
    match binding.context {
        Some(KeyContext::OnFile) => {
            rig.press(KeyCode::Char('j'));
        }
        Some(KeyContext::OnDirectory) => {
            for _ in 0..3 {
                rig.press(KeyCode::Char('j'));
            }
        }
        None => {}
        Some(other) => panic!("the tree pane has no context {other:?}"),
    }
    if binding.namespace == KeyNamespace::AgentTreeAgents {
        rig.state.focus = Focus::Agents;
    }
    rig.state.usage.clear();
    rig
}

/// press_every_row_key / RecordedActionMatchesRow for the tree and Agents
/// sections: each key of each row records the row's action id and the key.
#[test]
fn pressing_each_key_of_each_pane_row_records_the_rows_action() {
    let mut pressed = 0;
    for ns in [KeyNamespace::AgentTreeTree, KeyNamespace::AgentTreeAgents] {
        for binding in bindings_in(ns) {
            assert!(!binding.keys.contains(&ANY_OTHER_KEY));
            for key in binding.keys {
                let mut rig = rig_for(binding);
                if *key == "gg" {
                    rig.press(KeyCode::Char('j'));
                    rig.state.usage.clear();
                    rig.press(KeyCode::Char('g'));
                    assert!(rig.state.usage.is_empty(), "first g is pending input");
                    rig.press(KeyCode::Char('g'));
                } else {
                    let ev = crate::agent_tree::pane::test_key_event(key);
                    handle_key(&mut rig.state, &rig.tree, ev);
                }
                pressed += 1;
                let detail = match *key {
                    "Space" => " ".to_string(),
                    k if k.starts_with("Ctrl+") => k[5..].to_lowercase(),
                    k => k.to_string(),
                };
                assert_eq!(
                    recorded_usage(&rig.state),
                    vec![(binding.action.to_string(), Some(detail))],
                    "{} {key}",
                    ns.name()
                );
            }
        }
    }
    assert!(pressed > 30, "{pressed}");
}

/// A modified press with no row does nothing and records nothing, in either
/// section: Ctrl+j is not `j`, Ctrl+a is not `a`, Ctrl+q is not `q`.
#[test]
fn a_modified_press_with_no_row_does_nothing_in_the_pane() {
    for focus in [Focus::Tree, Focus::Commits, Focus::Agents] {
        for key in ["Ctrl+J", "Ctrl+A", "Ctrl+Q", "Ctrl+G", "Ctrl+L"] {
            let mut rig = KeyRig::with_agents(&three_node_changes(), vec![agent(1, false)]);
            rig.state.focus = focus;
            let before = rig.selected();
            let ev = crate::agent_tree::pane::test_key_event(key);
            assert_eq!(
                handle_key(&mut rig.state, &rig.tree, ev),
                KeyAction::Continue
            );
            assert!(rig.state.usage.is_empty(), "{key}");
            assert_eq!(rig.selected(), before, "{key}");
            assert!(rig.state.open_diffs().is_empty(), "{key}");
        }
    }
}

/// An unlisted key, and a press whose context fails, record nothing.
#[test]
fn unbound_and_contextless_presses_record_nothing() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('x'));
    // Nothing selected: Space/Enter/l have no applicable row.
    for code in [KeyCode::Char(' '), KeyCode::Enter, KeyCode::Char('l')] {
        rig.press(code);
    }
    assert!(rig.state.usage.is_empty(), "{:?}", rig.state.usage);
    // h/l and Space do nothing in the Agents section with no agent.
    rig.state.focus = Focus::Agents;
    rig.press(KeyCode::Char('h'));
    rig.press(KeyCode::Char('l'));
    rig.press(KeyCode::Char(' '));
    assert!(rig.state.usage.is_empty(), "{:?}", rig.state.usage);
    // h/l/Left/Right have no row in the commits section, and Space/Enter on
    // the row already selected ("unstaged work", the default) does nothing.
    rig.state.focus = Focus::Commits;
    for code in [
        KeyCode::Char('h'),
        KeyCode::Char('l'),
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Char(' '),
        KeyCode::Enter,
    ] {
        rig.press(code);
    }
    assert!(rig.state.usage.is_empty(), "{:?}", rig.state.usage);
}
