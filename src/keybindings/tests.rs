//! Well-formedness of the real table (`KeybindingDriftGate.TableIsWellFormed`
//! in `docs/specs/keybindings.allium`), asserted over `KEY_BINDINGS` itself.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn every_row_has_a_key_and_a_one_line_description() {
    for b in KEY_BINDINGS {
        assert!(!b.keys.is_empty(), "{b:?} has no key");
        assert!(!b.description.is_empty(), "{b:?} has no description");
        assert!(
            !b.description.contains('\n'),
            "{b:?} description is not one line"
        );
    }
}

#[test]
fn the_catch_all_stands_alone() {
    for b in KEY_BINDINGS {
        if b.keys.contains(&ANY_OTHER_KEY) {
            assert_eq!(b.keys.len(), 1, "{b:?} mixes the catch-all with keys");
        }
    }
}

#[test]
fn a_key_is_listed_once_per_row() {
    for b in KEY_BINDINGS {
        let mut seen = std::collections::HashSet::new();
        for k in b.keys {
            assert!(seen.insert(*k), "{b:?} lists {k} twice");
        }
    }
}

/// KeyUniquePerContext
#[test]
fn no_two_rows_share_a_key_and_a_context() {
    for (i, a) in KEY_BINDINGS.iter().enumerate() {
        for b in &KEY_BINDINGS[i + 1..] {
            if a.namespace == b.namespace && a.context == b.context {
                for k in a.keys {
                    assert!(
                        !b.keys.contains(k),
                        "{k} is bound twice in {} / {:?}: {} and {}",
                        a.namespace.name(),
                        a.context,
                        a.action,
                        b.action
                    );
                }
            }
        }
    }
}

/// ContextsDoNotOverlapForAKey
#[test]
fn no_key_has_two_rows_that_can_apply_at_once() {
    for (i, a) in KEY_BINDINGS.iter().enumerate() {
        for b in &KEY_BINDINGS[i + 1..] {
            if a.namespace != b.namespace {
                continue;
            }
            // The catch-all overlaps everything by design: a listed key
            // always beats it (KeypressRunsItsRowsAction).
            let shared: Vec<_> = a
                .keys
                .iter()
                .filter(|k| **k != ANY_OTHER_KEY && b.keys.contains(k))
                .collect();
            if shared.is_empty() {
                continue;
            }
            let (Some(ca), Some(cb)) = (a.context, b.context) else {
                panic!(
                    "{shared:?} in {}: {} and {} - a row with no context overlaps every other row sharing its key",
                    a.namespace.name(),
                    a.action,
                    b.action
                );
            };
            assert!(
                !contexts_overlap(ca, cb),
                "{shared:?} in {}: {} and {} can apply at once",
                a.namespace.name(),
                a.action,
                b.action
            );
        }
    }
}

/// ActionOncePerContext
#[test]
fn an_action_has_one_row_per_context_in_a_namespace() {
    for (i, a) in KEY_BINDINGS.iter().enumerate() {
        for b in &KEY_BINDINGS[i + 1..] {
            if a.namespace == b.namespace && a.context == b.context {
                assert_ne!(
                    a.action,
                    b.action,
                    "{} / {:?}: action {} has two rows; make it one row with two keys",
                    a.namespace.name(),
                    a.context,
                    a.action
                );
            }
        }
    }
}

/// LastRowKeyWarnsItDoesNotEnterAnEpic
#[test]
fn the_last_row_key_carries_its_warning() {
    let rows: Vec<_> = bindings_in(KeyNamespace::BoardNormal)
        .filter(|b| b.action == "navigate_row_last")
        .collect();
    assert!(!rows.is_empty());
    for b in rows {
        let note = b.note.expect("navigate_row_last must carry a note");
        assert!(note.contains("does not enter an epic"), "{note}");
        assert!(note.contains("Space"), "{note}");
    }
}

#[test]
fn rows_in_the_catalogue_are_grouped_in_namespace_order() {
    let order: Vec<usize> = KEY_BINDINGS
        .iter()
        .map(|b| {
            KeyNamespace::ALL
                .iter()
                .position(|n| *n == b.namespace)
                .unwrap()
        })
        .collect();
    assert!(
        order.windows(2).all(|w| w[0] <= w[1]),
        "table rows must be listed in KeyNamespace order"
    );
}

#[test]
fn namespace_names_are_unique_and_round_trip() {
    let mut seen = std::collections::HashSet::new();
    for n in KeyNamespace::ALL {
        assert!(seen.insert(n.name()), "{} is duplicated", n.name());
        assert_eq!(KeyNamespace::parse(n.name()), Some(n));
    }
}

#[test]
fn family_names_select_every_namespace_of_the_family() {
    let confirm = namespaces_matching("board.confirm").unwrap();
    assert_eq!(confirm.len(), 14);
    assert!(confirm
        .iter()
        .all(|n| n.name().starts_with("board.confirm.")));
    let picker = namespaces_matching("board.picker").unwrap();
    assert_eq!(picker.len(), 7);
    assert_eq!(namespaces_matching("board.normal").unwrap().len(), 1);
    assert!(namespaces_matching("board").is_none());
    assert!(namespaces_matching("nonsense").is_none());
}

#[test]
fn only_tmux_global_is_received_by_tmux() {
    for n in KeyNamespace::ALL {
        let expected = if n == KeyNamespace::TmuxGlobal {
            KeyReceiver::Tmux
        } else {
            KeyReceiver::Dispatch
        };
        assert_eq!(n.receiver(), expected, "{}", n.name());
    }
}

// ---- lookup (KeypressRunsItsRowsAction) -----------------------------------
//
// `lookup` is the one function every key handler answers a key through: the
// row of `ns` that lists `key` and whose context holds, else the namespace's
// applicable catch-all, else nothing. Exercised over fixture tables so each
// precedence rule is pinned on its own, then over the real table.

static FIXTURE: &[KeyBinding] = &[
    row(
        N::BoardConfirmQuit,
        &["y", "Y"],
        None,
        "confirm_quit_yes",
        "Quit",
        None,
    ),
    row(
        N::BoardConfirmQuit,
        &[ANY_OTHER_KEY],
        None,
        "confirm_quit_no",
        "Stay",
        None,
    ),
    row(N::BoardNormal, &["j"], None, "navigate_row", "Down", None),
    row(
        N::BoardNormal,
        &["q"],
        Some(C::InsideEpicView),
        "exit_epic",
        "Leave epic",
        None,
    ),
    row(
        N::BoardNormal,
        &["q"],
        Some(C::TopLevel),
        "quit",
        "Quit",
        None,
    ),
    row(
        N::BoardNormal,
        &["Esc"],
        Some(C::SearchActive),
        "clear_search",
        "Clear",
        None,
    ),
    // A namespace where a context-guarded key coexists with a catch-all.
    row(
        N::BoardError,
        &["Esc"],
        Some(C::SearchActive),
        "guarded_esc",
        "Guarded",
        None,
    ),
    row(
        N::BoardError,
        &[ANY_OTHER_KEY],
        None,
        "dismiss_error",
        "Dismiss",
        None,
    ),
];

fn action_of(b: Option<&KeyBinding>) -> Option<&'static str> {
    b.map(|b| b.action)
}

#[test]
fn lookup_an_exact_key_beats_the_catch_all() {
    for key in ["y", "Y"] {
        assert_eq!(
            action_of(lookup(FIXTURE, N::BoardConfirmQuit, key, |_| false)),
            Some("confirm_quit_yes"),
            "{key}"
        );
    }
}

#[test]
fn lookup_the_catch_all_answers_a_key_no_row_lists() {
    for key in ["n", "Esc", "Enter", "w"] {
        assert_eq!(
            action_of(lookup(FIXTURE, N::BoardConfirmQuit, key, |_| false)),
            Some("confirm_quit_no"),
            "{key}"
        );
    }
}

#[test]
fn lookup_picks_the_row_whose_context_holds() {
    assert_eq!(
        action_of(lookup(FIXTURE, N::BoardNormal, "q", |c| c == C::InsideEpicView)),
        Some("exit_epic")
    );
    assert_eq!(
        action_of(lookup(FIXTURE, N::BoardNormal, "q", |c| c == C::TopLevel)),
        Some("quit")
    );
}

#[test]
fn lookup_a_row_whose_context_does_not_hold_does_not_apply() {
    assert_eq!(lookup(FIXTURE, N::BoardNormal, "q", |_| false), None);
    assert_eq!(lookup(FIXTURE, N::BoardNormal, "Esc", |_| false), None);
}

#[test]
fn lookup_a_row_with_no_context_always_applies() {
    assert_eq!(
        action_of(lookup(FIXTURE, N::BoardNormal, "j", |_| false)),
        Some("navigate_row")
    );
    assert_eq!(
        action_of(lookup(FIXTURE, N::BoardNormal, "j", |_| true)),
        Some("navigate_row")
    );
}

/// A listed key whose row's context does not hold is not "listed" for the
/// press: the catch-all answers it, as for any other unlisted key.
#[test]
fn lookup_a_guarded_key_whose_context_fails_falls_to_the_catch_all() {
    assert_eq!(
        action_of(lookup(FIXTURE, N::BoardError, "Esc", |_| false)),
        Some("dismiss_error")
    );
    assert_eq!(
        action_of(lookup(FIXTURE, N::BoardError, "Esc", |c| c == C::SearchActive)),
        Some("guarded_esc")
    );
}

/// KeyWithoutARowCannotAct: no applicable row and no catch-all means nothing.
#[test]
fn lookup_a_key_with_no_row_is_none() {
    assert_eq!(lookup(FIXTURE, N::BoardNormal, "W", |_| true), None);
    // Rows are looked up in the press's namespace only.
    assert_eq!(lookup(FIXTURE, N::BoardHelp, "j", |_| true), None);
    assert_eq!(lookup(FIXTURE, N::BoardNormal, "y", |_| true), None);
    assert_eq!(lookup(&[], N::BoardNormal, "j", |_| true), None);
}

#[test]
fn lookup_returns_a_row_of_the_table_it_was_given() {
    let b = lookup(FIXTURE, N::BoardNormal, "j", |_| false).unwrap();
    assert!(std::ptr::eq(b, &FIXTURE[2]));
}

#[test]
fn lookup_over_the_real_table_reads_g_as_the_last_row_not_enter_epic() {
    let b = lookup(KEY_BINDINGS, N::BoardNormal, "G", |_| false).unwrap();
    assert_eq!(b.action, "navigate_row_last");
    let space_on_epic = lookup(KEY_BINDINGS, N::BoardNormal, "Space", |c| {
        c == C::OnEpicCard
    })
    .unwrap();
    assert_eq!(space_on_epic.action, "enter_epic");
}

// ---- KeyContext words ------------------------------------------------------

/// The plain words are the spec's: the help overlay and list_keybindings show
/// them, never the literal.
#[test]
fn view_level_context_words_are_the_specs() {
    assert_eq!(
        C::EpicViewNoSearch.words(),
        "inside an epic view, no search active"
    );
    assert_eq!(C::InsideEpicView.words(), "inside an epic view");
    assert_eq!(
        C::WithSelection.words(),
        "on the top-level board, no search active, with cards selected or the cursor on a select-all row"
    );
}

/// The `Esc` exit_epic row is guarded by epic_view_no_search, and `q`'s
/// exit_epic is its own row under inside_epic_view: Esc ranks a search above
/// the epic view, q ignores search, so the two keys need two literals.
#[test]
fn q_and_esc_exit_the_epic_under_different_contexts() {
    let exit: Vec<_> = bindings_in(N::BoardNormal)
        .filter(|b| b.action == "exit_epic")
        .collect();
    assert_eq!(exit.len(), 2, "{exit:?}");
    let q = exit.iter().find(|b| b.keys.contains(&"q")).unwrap();
    assert_eq!(q.keys, &["q"]);
    assert_eq!(q.context, Some(C::InsideEpicView));
    let esc = exit.iter().find(|b| b.keys.contains(&"Esc")).unwrap();
    assert_eq!(esc.keys, &["Esc"]);
    assert_eq!(esc.context, Some(C::EpicViewNoSearch));
}

// ---- Rows for every board namespace ---------------------------------------

fn is_board(n: KeyNamespace) -> bool {
    n.name().starts_with("board.")
}

/// Non-vacuity of the drift gate: a namespace with no rows would pass
/// press_every_row_key trivially, and by KeyWithoutARowCannotAct every key in
/// it would be inert.
#[test]
fn every_board_namespace_has_at_least_one_row() {
    let empty: Vec<&str> = KeyNamespace::ALL
        .iter()
        .copied()
        .filter(|n| is_board(*n))
        .filter(|n| bindings_in(*n).next().is_none())
        .map(|n| n.name())
        .collect();
    assert!(empty.is_empty(), "board namespaces with no rows: {empty:?}");
}

fn confirm_namespaces() -> Vec<KeyNamespace> {
    namespaces_matching("board.confirm").unwrap()
}

fn catch_all_rows(n: KeyNamespace) -> Vec<&'static KeyBinding> {
    bindings_in(n)
        .filter(|b| b.keys.contains(&ANY_OTHER_KEY))
        .collect()
}

/// Today's confirm_dialog: y/Y confirms, any other key is `<dialog>_no`.
/// The three dialogs with their own key sets (retry: r/f/Esc; the two tree
/// picker confirmations: y/n/Esc/q) ignore an unlisted key, so they have no
/// catch-all.
#[test]
fn confirm_dialogs_with_a_yes_row_catch_every_other_key_as_no() {
    let without_catch_all = [
        N::BoardConfirmRetry,
        N::BoardConfirmMoveToEpic,
        N::BoardConfirmReparentEpic,
    ];
    let mut checked = 0;
    for n in confirm_namespaces() {
        let catch_all = catch_all_rows(n);
        if without_catch_all.contains(&n) {
            assert!(
                catch_all.is_empty(),
                "{} must have no catch-all row: {catch_all:?}",
                n.name()
            );
            continue;
        }
        let has_yes = bindings_in(n).any(|b| b.keys.contains(&"y") || b.keys.contains(&"Y"));
        if !has_yes {
            continue;
        }
        checked += 1;
        assert_eq!(catch_all.len(), 1, "{}: {catch_all:?}", n.name());
        assert!(
            catch_all[0].action.ends_with("_no"),
            "{}: catch-all runs {}",
            n.name(),
            catch_all[0].action
        );
        let yes = bindings_in(n)
            .find(|b| b.keys.contains(&"y"))
            .unwrap_or_else(|| panic!("{} binds Y but not y", n.name()));
        assert!(yes.keys.contains(&"Y"), "{}: y and Y are one row", n.name());
        assert!(yes.action.ends_with("_yes"), "{}: {}", n.name(), yes.action);
    }
    assert_eq!(checked, 11, "eleven confirm dialogs answer y/Y");
}

#[test]
fn the_error_popup_is_one_catch_all_row_that_dismisses() {
    let rows: Vec<_> = bindings_in(N::BoardError).collect();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].keys, &[ANY_OTHER_KEY]);
    assert_eq!(rows[0].action, "dismiss_error");
    assert_eq!(rows[0].context, None);
}

/// HelpOverlayIsTheTable: the overlay scrolls, so its scroll keys are rows.
#[test]
fn the_help_overlay_scrolls_with_j_down_k_up() {
    let rows: Vec<_> = bindings_in(N::BoardHelp)
        .filter(|b| b.action == "scroll_help")
        .collect();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].keys, &["j", "Down", "k", "Up"]);
    assert_eq!(rows[0].context, None);
}

/// Characters typed into a text-bearing mode are data (observability.allium:
/// is_text_entry), not bindings: no row of a typing mode lists a printable
/// character.
#[test]
fn typing_modes_have_no_rows_for_printable_characters() {
    for n in [
        N::BoardSearch,
        N::BoardText,
        N::BoardPickerRepoPath,
        N::BoardPickerBaseBranch,
        N::BoardPickerQuickDispatch,
    ] {
        for b in bindings_in(n) {
            for k in b.keys {
                let printable = k.chars().count() == 1 && !k.chars().all(char::is_whitespace);
                assert!(
                    !printable,
                    "{}: `{k}` is typed text, not a binding ({})",
                    n.name(),
                    b.action
                );
                assert_ne!(*k, "Space", "{}: Space is typed text", n.name());
            }
        }
    }
}

/// Non-vacuity for the whole table: every namespace but tmux.global has rows
/// (the agent-tree and diff panes included).
#[test]
fn every_namespace_but_tmux_global_has_at_least_one_row() {
    let empty: Vec<&str> = KeyNamespace::ALL
        .iter()
        .copied()
        .filter(|n| *n != KeyNamespace::TmuxGlobal)
        .filter(|n| bindings_in(*n).next().is_none())
        .map(|n| n.name())
        .collect();
    assert!(empty.is_empty(), "namespaces with no rows: {empty:?}");
}

/// Only text-editing rows opt out of usage recording.
#[test]
fn only_text_editing_rows_are_unrecorded() {
    let odd: Vec<&str> = KEY_BINDINGS
        .iter()
        .filter(|b| !b.records_usage)
        .filter(|b| !b.action.starts_with("text_"))
        .map(|b| b.action)
        .collect();
    assert!(odd.is_empty(), "{odd:?}");
    assert!(KEY_BINDINGS.iter().any(|b| !b.records_usage));
}

/// Every modified key a row lists is written modifier-first with an uppercase
/// letter, the form `key_name` produces.
#[test]
fn modified_keys_are_written_in_key_names_form() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
    assert_eq!(key_name(ctrl('d')), "Ctrl+D");
    assert_eq!(
        key_name(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL)),
        "Ctrl+Left"
    );
    assert_eq!(
        key_name(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT)),
        "Alt+B"
    );
    assert_eq!(
        key_name(KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT)),
        "L"
    );
    assert_eq!(
        key_name(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
        "Space"
    );
    for b in KEY_BINDINGS {
        for k in b.keys {
            if let Some(rest) = k.strip_prefix("Ctrl+").or_else(|| k.strip_prefix("Alt+")) {
                let first = rest.chars().next().unwrap();
                assert!(
                    rest.chars().count() > 1 && rest.chars().all(char::is_alphabetic)
                        || first.is_ascii_uppercase() && rest.chars().count() == 1,
                    "{k}"
                );
            }
        }
    }
}

/// A modified press is looked up under its modified name and never reaches a
/// catch-all.
#[test]
fn lookup_never_answers_a_modified_press_with_the_catch_all() {
    let caught = lookup(KEY_BINDINGS, KeyNamespace::BoardError, "Ctrl+X", |_| false);
    assert!(caught.is_none());
    let plain = lookup(KEY_BINDINGS, KeyNamespace::BoardError, "x", |_| false);
    assert_eq!(plain.map(|b| b.action), Some("dismiss_error"));
}

/// keybindings.allium's pane inventory: agent_tree.commits sits between the
/// tree's and the agents section's namespaces, under its published name.
#[test]
fn the_commits_namespace_sits_between_the_tree_and_the_agents_section() {
    let at = |n: KeyNamespace| KeyNamespace::ALL.iter().position(|x| *x == n).unwrap();
    assert_eq!(KeyNamespace::AgentTreeCommits.name(), "agent_tree.commits");
    assert_eq!(
        at(KeyNamespace::AgentTreeCommits),
        at(KeyNamespace::AgentTreeTree) + 1
    );
    assert_eq!(
        at(KeyNamespace::AgentTreeAgents),
        at(KeyNamespace::AgentTreeCommits) + 1
    );
}

/// The commits section adds no new key: its pane-wide rows are the tree's,
/// key for key, so focus — not a new key — decides what a press means.
#[test]
fn the_commits_namespace_shares_the_pane_wide_keys_with_the_tree() {
    for action in [
        "exit_pane",
        "toggle_focus",
        "toggle_all_diffs",
        "navigate_row",
        "navigate_row_first",
        "navigate_row_last",
        "navigate_half_page",
    ] {
        let keys = |ns: KeyNamespace| -> Vec<&str> {
            bindings_in(ns)
                .filter(|b| b.action == action)
                .flat_map(|b| b.keys.iter().copied())
                .collect()
        };
        let tree = keys(KeyNamespace::AgentTreeTree);
        assert!(!tree.is_empty(), "{action}");
        assert_eq!(keys(KeyNamespace::AgentTreeCommits), tree, "{action}");
    }
}

/// Space/Enter is select_source in the commits namespace, with no context,
/// as it is jump_to_agent in the agents namespace.
#[test]
fn select_source_is_space_and_enter_in_the_commits_namespace() {
    let rows: Vec<&KeyBinding> = bindings_in(KeyNamespace::AgentTreeCommits)
        .filter(|b| b.action == "select_source")
        .collect();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].keys, &["Space", "Enter"]);
    assert_eq!(rows[0].context, None);
}

/// h, l, Left and Right have no row in the commits namespace, and nothing
/// but the pane-wide rows and select_source is there at all.
#[test]
fn the_commits_namespace_has_only_the_pane_wide_rows_and_select_source() {
    let actions: Vec<&str> = bindings_in(KeyNamespace::AgentTreeCommits)
        .map(|b| b.action)
        .collect();
    assert_eq!(
        actions,
        vec![
            "exit_pane",
            "toggle_focus",
            "toggle_all_diffs",
            "navigate_row",
            "navigate_row_first",
            "navigate_row_last",
            "navigate_half_page",
            "select_source",
        ]
    );
    for key in ["h", "l", "Left", "Right"] {
        assert!(
            !bindings_in(KeyNamespace::AgentTreeCommits).any(|b| b.keys.contains(&key)),
            "{key}"
        );
    }
}

/// Row lists for the panes: the modified keys that act have rows.
#[test]
fn pane_modified_keys_have_rows() {
    for ns in [
        KeyNamespace::AgentTreeTree,
        KeyNamespace::AgentTreeCommits,
        KeyNamespace::AgentTreeAgents,
        KeyNamespace::AgentDiff,
    ] {
        let has = |k: &str| bindings_in(ns).any(|b| b.keys.contains(&k));
        assert!(
            has("Ctrl+D") && has("Ctrl+U") && has("Ctrl+C"),
            "{}",
            ns.name()
        );
    }
}

/// The overlay title is the board.help rows' keys, not a literal.
#[test]
fn the_help_overlay_title_names_its_own_keys() {
    assert_eq!(
        help_overlay_title(KEY_BINDINGS),
        " Help \u{2014} j/Down/k/Up scroll, ? or Esc close "
    );
    static REBOUND: [KeyBinding; 2] = [
        row(
            KeyNamespace::BoardHelp,
            &["x"],
            None,
            "scroll_help",
            "d",
            None,
        ),
        row(
            KeyNamespace::BoardHelp,
            &["w", "Q"],
            None,
            "close_help",
            "d",
            None,
        ),
    ];
    assert_eq!(
        help_overlay_title(&REBOUND),
        " Help \u{2014} x scroll, w or Q close "
    );
}

/// The tmux.global toggle row names the key the runtime actually binds.
#[test]
fn the_tmux_toggle_row_names_the_bound_key() {
    let want = format!("Prefix+{}", crate::runtime::AGENT_TREE_TOGGLE_KEY);
    assert!(
        bindings_in(KeyNamespace::TmuxGlobal)
            .any(|b| b.action == "toggle_agent_tree" && b.keys.contains(&want.as_str())),
        "{want}"
    );
}

/// The exit keys the startup
/// screen answers are the `exit_pane` row's.
#[test]
fn the_startup_screens_exit_keys_are_the_exit_pane_rows_keys() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    for ns in [KeyNamespace::AgentTreeTree, KeyNamespace::AgentDiff] {
        let press =
            |code, mods| crate::agent_tree::pane::is_quit_key(&KeyEvent::new(code, mods), ns);
        assert!(press(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(press(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(!press(KeyCode::Char('q'), KeyModifiers::ALT));
        assert!(!press(KeyCode::Char('x'), KeyModifiers::NONE));
    }
}

/// The tmux.global jump-back row names the key the runtime binds.
#[test]
fn the_tmux_jump_back_row_names_the_bound_key() {
    let mut chars = crate::runtime::JUMP_BACK_KEY.chars();
    let first = chars.next().unwrap().to_ascii_uppercase();
    let want = format!("Prefix+{first}{}", chars.as_str());
    assert!(
        bindings_in(KeyNamespace::TmuxGlobal)
            .any(|b| b.action == "jump_to_dispatch" && b.keys.contains(&want.as_str())),
        "{want}"
    );
}

/// `contexts_overlap` must report pairs that can really hold together, or the
/// table check above can never fail for them.
#[test]
fn contexts_overlap_reports_real_co_occurrence() {
    use KeyContext::*;
    for (a, b) in [
        (SearchActive, InsideEpicView),
        (InsideEpicView, EpicViewNoSearch),
        (OffEpicCard, OnTaskCard),
        (OnTaskCard, BacklogTask),
        (TopLevel, OnEpicCard),
    ] {
        assert!(contexts_overlap(a, b), "{a:?} and {b:?} can hold together");
        assert!(contexts_overlap(b, a), "{b:?} and {a:?} must be symmetric");
    }
    for (a, b) in [
        (EpicViewNoSearch, SearchActive),
        (OnEpicCard, OnTaskCard),
        (OnFoldedSection, BacklogTask),
        (BacklogTask, StuckTask),
        (OnDirectory, OnFile),
        (OnDirectory, TopLevel),
    ] {
        assert!(!contexts_overlap(a, b), "{a:?} and {b:?} are exclusive");
        assert!(!contexts_overlap(b, a), "{b:?} and {a:?} must be symmetric");
    }
}
