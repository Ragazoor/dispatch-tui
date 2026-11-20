//! The override loader (`LoadKeyTable` and the drift-gate obligations
//! `NoFileIsTheDefaultTable`, `AnyOverrideFileGivesAWellFormedTable`, `EveryRefusalIsExercised`
//! and `TableIsWellFormed` in docs/specs/keybindings.allium).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::keybindings::key_name;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn keys_of(table: &[KeyBinding], ns: KeyNamespace, action: &str) -> Vec<&'static str> {
    table
        .iter()
        .find(|b| b.namespace == ns && b.action == action && b.context.is_none())
        .map(|b| b.keys.to_vec())
        .expect("a context-free row")
}

fn default_keys(ns: KeyNamespace, action: &str) -> Vec<&'static str> {
    keys_of(KEY_BINDINGS, ns, action)
}

/// A modified key no row of `ns` lists, for a good entry beside a bad one.
fn free_key(ns: KeyNamespace, skip: usize) -> &'static str {
    ["Ctrl+T", "Ctrl+Y", "Ctrl+O", "F5", "F6", "F7"]
        .into_iter()
        .filter(|k| {
            !KEY_BINDINGS
                .iter()
                .any(|b| b.namespace == ns && b.keys.contains(k))
        })
        .nth(skip)
        .expect("a free key")
}

fn entry(ns: &str, action: &str, keys: &[&str]) -> String {
    serde_json::json!({"namespace": ns, "action": action, "keys": keys}).to_string()
}

fn file_of(entries: &[String]) -> String {
    format!("[{}]", entries.join(","))
}

/// Every invariant a loaded table must keep, whatever the file said.
fn assert_well_formed(table: &[KeyBinding]) {
    assert_eq!(table.len(), KEY_BINDINGS.len(), "same rows, same order");
    let clashes = table_clashes(table);
    assert!(clashes.is_empty(), "{clashes:?}");
    for (b, d) in table.iter().zip(KEY_BINDINGS) {
        assert_eq!(
            (b.namespace, b.action, b.context),
            (d.namespace, d.action, d.context)
        );
        assert!(!b.keys.is_empty(), "{} has no key", b.action);
        if d.keys.contains(&ANY_OTHER_KEY) {
            assert_eq!(b.keys, d.keys, "a catch-all row is never overridden");
            continue;
        }
        if b.namespace == KeyNamespace::TmuxGlobal {
            assert_eq!(b.keys.len(), 1, "{}", b.action);
        }
        for k in b.keys {
            assert!(is_key_name(k), "{}: {k} is not canonical", b.action);
            assert_eq!(
                key_not_allowed_in(b.namespace, k),
                None,
                "{}: {k}",
                b.action
            );
        }
    }
}

// --- TableIsWellFormed: the loader's checks hold on the default table ---

#[test]
fn the_default_table_passes_the_loaders_checks() {
    assert_well_formed(KEY_BINDINGS);
}

#[test]
fn every_text_namespace_binds_no_typed_key() {
    let text: Vec<KeyNamespace> = KeyNamespace::ALL
        .into_iter()
        .filter(|n| namespace_takes_text(*n))
        .collect();
    assert_eq!(text.len(), 6, "the spec names six text namespaces");
    for b in KEY_BINDINGS
        .iter()
        .filter(|b| namespace_takes_text(b.namespace))
    {
        for k in b.keys {
            assert!(
                !is_typed_key(k) && !is_chord(k),
                "{}: {k}",
                b.namespace.name()
            );
        }
    }
}

/// Every name the validator accepts as a single key is one a press can
/// produce, or an accepted override would be a silent dead key.
#[test]
fn every_named_key_is_a_name_a_press_produces() {
    let code = |name: &str| match name {
        "Space" => KeyCode::Char(' '),
        "Enter" => KeyCode::Enter,
        "Esc" => KeyCode::Esc,
        "Tab" => KeyCode::Tab,
        "BackTab" => KeyCode::BackTab,
        "Backspace" => KeyCode::Backspace,
        "Delete" => KeyCode::Delete,
        "Insert" => KeyCode::Insert,
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        f => KeyCode::F(f.strip_prefix('F').unwrap().parse().unwrap()),
    };
    for name in NAMED_KEYS {
        assert!(is_key_name(name));
        let press = KeyEvent::new(code(name), KeyModifiers::NONE);
        assert_eq!(key_name(press), name);
        let ctrl = KeyEvent::new(code(name), KeyModifiers::CONTROL);
        assert_eq!(key_name(ctrl), format!("Ctrl+{name}"));
        assert!(is_key_name(&key_name(ctrl)));
    }
    for c in ['x', 'é', 'ß', '1', '+'] {
        let ctrl = key_name(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
        assert!(
            is_key_name(&ctrl),
            "{ctrl} is what a press of Ctrl+{c} is named"
        );
    }
    assert!(!is_key_name("Ctrl+é"), "a press never produces it");
    let ctrl_alt = KeyEvent::new(
        KeyCode::Char('x'),
        KeyModifiers::CONTROL | KeyModifiers::ALT,
    );
    assert_eq!(key_name(ctrl_alt), "Ctrl+Alt+X");
    assert!(is_key_name("Ctrl+Alt+X"));
}

#[test]
fn key_names_are_canonical_only() {
    for good in [
        "j",
        "G",
        "?",
        "Space",
        "F12",
        "Ctrl+D",
        "Alt+B",
        "Ctrl+Left",
        "gg",
        "Prefix+e",
        "Prefix+Space",
    ] {
        assert!(is_key_name(good), "{good}");
    }
    assert!(
        is_key_name("F0") && is_chord("F0"),
        "F0 is a chord, not a named key"
    );
    for bad in [
        "ctrl+d",
        "Ctrl+d",
        "enter",
        "Escape",
        "Shift+G",
        "Ctrl+I",
        "Ctrl+M",
        "Ctrl+[",
        " ",
        "",
        "ggg",
        "F13",
        "Prefix+gg",
        "Up+",
    ] {
        assert!(!is_key_name(bad), "{bad}");
    }
}

#[test]
fn a_near_miss_key_name_gets_its_canonical_spelling() {
    for (miss, good) in [
        ("ctrl+d", "Ctrl+D"),
        ("Ctrl+d", "Ctrl+D"),
        ("enter", "Enter"),
        ("Escape", "Esc"),
        ("Shift+g", "G"),
        ("shift+tab", "BackTab"),
        ("ctrl+i", "Tab"),
        ("alt+left", "Alt+Left"),
        ("pgdn", "PageDown"),
    ] {
        assert_eq!(suggest_key_name(miss).as_deref(), Some(good), "{miss}");
    }
    assert_eq!(suggest_key_name("bogus"), None);
}

/// The override file names a context by the spec's literal; every
/// KeyContext literal in the spec has a context, and back.
#[test]
fn context_literals_are_the_specs() {
    let spec = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/docs/specs/keybindings.allium"
    ))
    .unwrap();
    let body = spec.split("enum KeyContext {").nth(1).unwrap();
    let body = body.split('}').next().unwrap();
    let mut literals: Vec<&str> = body
        .split(|c: char| c == '|' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .collect();
    let mut ours: Vec<&str> = KeyContext::ALL.iter().map(|c| c.literal()).collect();
    literals.sort_unstable();
    ours.sort_unstable();
    assert_eq!(ours, literals);
    for c in KeyContext::ALL {
        assert_eq!(KeyContext::parse(c.literal()), Some(c));
    }
}

#[test]
fn problem_literals_are_the_specs() {
    let spec = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/docs/specs/keybindings.allium"
    ))
    .unwrap();
    let body = spec.split("enum KeyOverrideProblem {").nth(1).unwrap();
    let body = body.split('}').next().unwrap();
    let literals: Vec<&str> = body
        .split(|c: char| c == '|' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .collect();
    let ours: Vec<&str> = KeyOverrideProblem::ALL.iter().map(|p| p.as_str()).collect();
    assert_eq!(ours, literals);
}

// --- NoFileIsTheDefaultTable ---

#[test]
fn no_file_is_the_default_table() {
    let dir = tempfile::tempdir().unwrap();
    let load = load_key_table(dir.path(), KeyTableReader::Board);
    assert!(std::ptr::eq(load.table, KEY_BINDINGS));
    assert!(load.errors.is_empty());
    assert_eq!(load.error_summary(), None);
    assert_eq!(load.file, dir.path().join("keybindings.json"));
}

#[test]
fn a_file_that_cannot_be_read_is_one_error_and_the_default_table() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(KEY_OVERRIDE_FILE_NAME)).unwrap();
    let load = load_key_table(dir.path(), KeyTableReader::Board);
    assert!(std::ptr::eq(load.table, KEY_BINDINGS));
    assert_eq!(load.errors.len(), 1);
    assert_eq!(load.errors[0].problem, KeyOverrideProblem::FileUnreadable);
    assert!(load.errors[0].message.contains("no override applies"));
    assert!(load.errors[0]
        .message
        .contains("remove it to use the default keys"));
}

#[test]
fn a_file_in_the_data_directory_overrides_its_rows() {
    let dir = tempfile::tempdir().unwrap();
    let key = free_key(KeyNamespace::BoardNormal, 0);
    std::fs::write(
        dir.path().join(KEY_OVERRIDE_FILE_NAME),
        file_of(&[entry("board.normal", "navigate_row_last", &[key, key])]),
    )
    .unwrap();
    let load = load_key_table(dir.path(), KeyTableReader::Board);
    assert!(load.errors.is_empty(), "{:?}", load.errors);
    assert_eq!(
        keys_of(load.table, KeyNamespace::BoardNormal, "navigate_row_last"),
        vec![key],
        "a key listed twice counts once"
    );
    assert_well_formed(load.table);
    let changed = load
        .table
        .iter()
        .zip(KEY_BINDINGS)
        .filter(|(b, d)| b.keys != d.keys)
        .count();
    assert_eq!(changed, 1, "only the named row moves");
}

#[test]
fn an_entry_with_a_context_names_that_row() {
    let key = free_key(KeyNamespace::BoardNormal, 0);
    let text = serde_json::json!([{
        "namespace": "board.normal", "action": "select_all",
        "context": "on_column_select_all", "keys": [key]
    }])
    .to_string();
    let (table, errors) = merge_overrides(&text);
    assert!(errors.is_empty(), "{errors:?}");
    let row = table
        .iter()
        .find(|b| b.action == "select_all" && b.context == Some(KeyContext::OnColumnSelectAll))
        .unwrap();
    assert_eq!(row.keys, [key]);
    assert_eq!(
        keys_of(table, KeyNamespace::BoardNormal, "select_all"),
        default_keys(KeyNamespace::BoardNormal, "select_all")
    );
}

#[test]
fn swapping_two_rows_keys_is_accepted() {
    let ns = KeyNamespace::BoardNormal;
    let (down, up) = (
        default_keys(ns, "navigate_row_down"),
        default_keys(ns, "navigate_row_up"),
    );
    let (table, errors) = merge_overrides(&file_of(&[
        entry("board.normal", "navigate_row_down", &up),
        entry("board.normal", "navigate_row_up", &down),
    ]));
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(keys_of(table, ns, "navigate_row_down"), up);
    assert_eq!(keys_of(table, ns, "navigate_row_up"), down);
}

/// A refused entry puts its default key back, which can make another entry
/// clash: the check runs again, and the outcome does not depend on order.
#[test]
fn a_refusal_that_restores_a_default_key_refuses_the_entry_that_took_it() {
    let ns = KeyNamespace::BoardNormal;
    let last = default_keys(ns, "navigate_row_last");
    let key = free_key(ns, 0);
    let entries = vec![
        entry("board.normal", "navigate_row_down", &last[..1]),
        entry("board.normal", "navigate_row_last", &[key]),
        entry("board.normal", "navigate_row_up", &[key]),
    ];
    let (table, errors) = merge_overrides(&file_of(&entries));
    let mut refused: Vec<(String, KeyOverrideProblem)> = errors
        .iter()
        .map(|e| (e.action.clone().unwrap(), e.problem))
        .collect();
    refused.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        refused,
        vec![
            (
                "navigate_row_down".to_string(),
                KeyOverrideProblem::KeyClash
            ),
            (
                "navigate_row_last".to_string(),
                KeyOverrideProblem::KeyClash
            ),
            ("navigate_row_up".to_string(), KeyOverrideProblem::KeyClash),
        ]
    );
    assert!(std::ptr::eq(table, KEY_BINDINGS) || table == KEY_BINDINGS);

    let mut reversed = entries.clone();
    reversed.reverse();
    let (again, errors_again) = merge_overrides(&file_of(&reversed));
    assert_eq!(again, table);
    assert_eq!(errors_again.len(), errors.len());
}

// --- EveryRefusalIsExercised ---

/// A file with one entry that has `problem`, and text its message must
/// carry (what to write instead, or the valid names). Exhaustive, so a new
/// problem fails to compile until it has a case.
fn bad_file(problem: KeyOverrideProblem) -> (Vec<String>, &'static str) {
    use KeyOverrideProblem as P;
    let ns = KeyNamespace::BoardNormal;
    match problem {
        P::FileUnreadable => (vec!["{not json".to_string()], "Fix the JSON"),
        P::MalformedEntry => (vec![serde_json::json!({"namespace": "board.normal", "action": "navigate_row_last", "key": ["E"]}).to_string()], "an entry is"),
        P::DuplicateEntry => (
            vec![
                entry("board.normal", "navigate_row_last", &[free_key(ns, 1)]),
                entry("board.normal", "navigate_row_last", &[free_key(ns, 2)]),
            ],
            "keep one",
        ),
        P::UnknownNamespace => (vec![entry("board.confirm", "confirm_quit_yes", &["o"])], "board.confirm.quit"),
        P::UnknownAction => (vec![entry("board.normal", "jump_rope", &["E"])], "navigate_row_last"),
        P::UnknownContext => (
            vec![serde_json::json!({"namespace": "board.normal", "action": "select_all", "context": "on_nowhere", "keys": ["E"]}).to_string()],
            "on_column_select_all",
        ),
        P::AmbiguousRow => {
            let (ns, action) = KEY_BINDINGS
                .iter()
                .find(|b| {
                    let rows: Vec<_> = KEY_BINDINGS.iter().filter(|o| o.namespace == b.namespace && o.action == b.action).collect();
                    rows.len() > 1 && rows.iter().all(|o| o.context.is_some())
                })
                .map(|b| (b.namespace, b.action))
                .expect("an action with several rows, all context-guarded");
            (vec![entry(ns.name(), action, &[free_key(ns, 0)])], "add \"context\"")
        }
        P::EmptyKeys => (vec![entry("board.normal", "navigate_row_last", &[])], "remove the entry"),
        P::UnknownKeyName => (vec![entry("board.normal", "navigate_row_last", &["ctrl+e"])], "write \"Ctrl+E\""),
        P::KeyNotAllowedInNamespace => (vec![entry("board.text", "text_backspace", &["x"])], "takes typed text"),
        P::CatchAllRow => (vec![entry("board.error", "dismiss_error", &["q"])], "rebind the rows beside it"),
        P::KeyClash => (
            vec![entry("board.normal", "navigate_row_last", &default_keys(ns, "navigate_row_down")[..1])],
            "navigate_row_down",
        ),
        P::ChordPrefixClash => {
            let down = default_keys(ns, "navigate_row_down")[0];
            let chord: &'static str = Box::leak(format!("{down}z").into_boxed_str());
            (vec![entry("board.normal", "navigate_row_last", &[chord])], "starts with")
        }
    }
}

#[test]
fn every_refusal_is_exercised() {
    let ns = KeyNamespace::BoardNormal;
    let good_key = free_key(ns, 0);
    for problem in KeyOverrideProblem::ALL {
        let (mut entries, says) = bad_file(problem);
        let unreadable = problem == KeyOverrideProblem::FileUnreadable;
        let text = if unreadable {
            entries.remove(0)
        } else {
            entries.push(entry("board.normal", "navigate_row_first", &[good_key]));
            file_of(&entries)
        };
        let (table, errors) = merge_overrides(&text);
        let name = problem.as_str();
        assert!(!errors.is_empty(), "{name}: nothing refused");
        assert!(
            errors.iter().all(|e| e.problem == problem),
            "{name}: {errors:?}"
        );
        assert!(
            errors.iter().all(|e| e.message.contains(says)),
            "{name}: {errors:?} should say {says:?}"
        );
        assert_well_formed(table);
        // The refused rows keep their defaults; only the good entry moved anything.
        for (b, d) in table.iter().zip(KEY_BINDINGS) {
            if !(b.namespace == ns && b.action == "navigate_row_first") {
                assert_eq!(b.keys, d.keys, "{name}: {} moved", b.action);
            }
        }
        let first = keys_of(table, ns, "navigate_row_first");
        if unreadable {
            assert_eq!(errors.len(), 1);
            assert_eq!(
                first,
                default_keys(ns, "navigate_row_first"),
                "no entry applies"
            );
        } else {
            assert_eq!(
                first,
                vec![good_key],
                "{name}: the good entry still applies"
            );
        }
    }
}

/// An entry refused for its keys still names its row, so a second, good
/// entry for that row is a duplicate and the row keeps its default keys.
#[test]
fn an_entry_refused_for_its_keys_still_makes_a_second_entry_a_duplicate() {
    let ns = KeyNamespace::BoardNormal;
    let (table, errors) = merge_overrides(&file_of(&[
        entry("board.normal", "navigate_row_last", &["ctrl+e"]),
        entry("board.normal", "navigate_row_last", &[free_key(ns, 0)]),
    ]));
    let problems: Vec<KeyOverrideProblem> = errors.iter().map(|e| e.problem).collect();
    assert_eq!(
        problems,
        vec![
            KeyOverrideProblem::UnknownKeyName,
            KeyOverrideProblem::DuplicateEntry
        ]
    );
    assert_eq!(
        keys_of(table, ns, "navigate_row_last"),
        default_keys(ns, "navigate_row_last")
    );
}

#[test]
fn other_malformed_and_disallowed_entries_are_refused() {
    use KeyOverrideProblem as P;
    for (text, problem) in [
        ("{\"namespace\": \"board.normal\"}".to_string(), P::FileUnreadable),
        ("[42]".to_string(), P::MalformedEntry),
        ("[{\"namespace\": \"board.normal\", \"action\": \"navigate_row_last\"}]".to_string(), P::MalformedEntry),
        ("[{\"namespace\": \"board.normal\", \"action\": \"navigate_row_last\", \"keys\": [1]}]".to_string(), P::MalformedEntry),
        ("[{\"namespace\": \"board.normal\", \"action\": \"navigate_row_last\", \"context\": 3, \"keys\": [\"E\"]}]".to_string(), P::MalformedEntry),
        (file_of(&[entry("board.normal", "navigate_row_last", &["Prefix+e"])]), P::KeyNotAllowedInNamespace),
        (file_of(&[entry("tmux.global", "toggle_agent_tree", &["e"])]), P::KeyNotAllowedInNamespace),
        (file_of(&[entry("tmux.global", "toggle_agent_tree", &["Prefix+x", "Prefix+y"])]), P::KeyNotAllowedInNamespace),
        (file_of(&[entry("board.search", "text_backspace", &["Space"])]), P::KeyNotAllowedInNamespace),
        (file_of(&[entry("board.filter_search", "text_backspace", &["zz"])]), P::KeyNotAllowedInNamespace),
        (file_of(&[entry("board.normal", "navigate_row_last", &["any other key"])]), P::CatchAllRow),
        (file_of(&[entry("board.normal", "navigate_row_last", &["Shift+G"])]), P::UnknownKeyName),
    ] {
        let (table, errors) = merge_overrides(&text);
        assert_eq!(errors.len(), 1, "{text}: {errors:?}");
        assert_eq!(errors[0].problem, problem, "{text}: {errors:?}");
        assert_eq!(table, KEY_BINDINGS);
    }
}

#[test]
fn a_tmux_key_name_with_no_suggestion_is_told_the_prefix_form() {
    let (_, errors) = merge_overrides(&file_of(&[entry(
        "tmux.global",
        "toggle_agent_tree",
        &["Hyper+e"],
    )]));
    assert_eq!(errors[0].problem, KeyOverrideProblem::UnknownKeyName);
    assert!(
        errors[0].message.contains("\"Prefix+\" and one key"),
        "{}",
        errors[0].message
    );
}

#[test]
fn a_message_repeats_little_of_a_long_entry() {
    let long = "x".repeat(500);
    let (_, errors) = merge_overrides(&format!("[\"{long}\"]"));
    assert_eq!(errors[0].problem, KeyOverrideProblem::MalformedEntry);
    assert!(errors[0].message.len() < 300, "{}", errors[0].message);
}

#[test]
fn a_tmux_row_takes_one_prefix_key() {
    let (table, errors) = merge_overrides(&file_of(&[entry(
        "tmux.global",
        "toggle_agent_tree",
        &["Prefix+a"],
    )]));
    assert!(errors.is_empty(), "{errors:?}");
    let row = table
        .iter()
        .find(|b| b.action == "toggle_agent_tree")
        .unwrap();
    assert_eq!(row.keys, ["Prefix+a"]);
}

/// The popup shows every message whole but a long list of valid names,
/// which it cuts and points to the log; the log keeps it whole.
#[test]
fn the_popup_cuts_only_a_long_list_of_names() {
    for problem in KeyOverrideProblem::ALL {
        let (mut entries, _) = bad_file(problem);
        let text = if problem == KeyOverrideProblem::FileUnreadable {
            entries.remove(0)
        } else {
            file_of(&entries)
        };
        let (_, errors) = merge_overrides(&text);
        for e in &errors {
            let listing = matches!(
                problem,
                KeyOverrideProblem::UnknownNamespace | KeyOverrideProblem::UnknownAction
            );
            if listing {
                assert!(e.popup_message.contains("app.log"), "{}", e.popup_message);
                assert!(e.popup_message.len() < e.message.len());
            } else {
                assert_eq!(e.popup_message, e.message, "{}", problem.as_str());
            }
        }
    }
}

/// Past a few refusals the popup names how many more there are.
#[test]
fn the_popup_lists_a_few_refusals_and_counts_the_rest() {
    let entries: Vec<String> = [
        "navigate_row_last",
        "navigate_row_first",
        "navigate_row_down",
        "navigate_row_up",
        "navigate_column_left",
        "navigate_column_right",
    ]
    .iter()
    .map(|a| entry("board.normal", a, &["ctrl+x"]))
    .collect();
    let (table, errors) = merge_overrides(&file_of(&entries));
    let load = KeyTableLoad {
        file: PathBuf::from("/data/keybindings.json"),
        table,
        errors,
    };
    let summary = load.error_summary().unwrap();
    assert_eq!(summary.matches("\n- ").count(), POPUP_ERRORS);
    assert!(
        summary.ends_with("and 3 more, listed in app.log"),
        "{summary}"
    );
}

#[test]
fn the_summary_names_the_file_and_each_message() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(KEY_OVERRIDE_FILE_NAME),
        file_of(&[entry("board.normal", "navigate_row_last", &["ctrl+e"])]),
    )
    .unwrap();
    let load = load_key_table(dir.path(), KeyTableReader::Board);
    let summary = load.error_summary().unwrap();
    assert!(summary.contains(
        &dir.path()
            .join(KEY_OVERRIDE_FILE_NAME)
            .display()
            .to_string()
    ));
    assert!(summary.contains("keep their default keys"));
    assert!(summary.contains("write \"Ctrl+E\""), "{summary}");
}

// --- AnyOverrideFileGivesAWellFormedTable ---

/// Generated files, good and bad entries mixed: the table always keeps
/// every invariant.
#[test]
fn any_override_file_gives_a_well_formed_table() {
    let rows: Vec<&KeyBinding> = KEY_BINDINGS.iter().collect();
    let pool: Vec<&str> = vec![
        "j",
        "k",
        "G",
        "g",
        "gg",
        "jz",
        "E",
        "Space",
        "Enter",
        "Esc",
        "Ctrl+D",
        "Ctrl+T",
        "F5",
        "ctrl+x",
        "Prefix+e",
        "Prefix+z",
        "any other key",
        "x",
        "q",
        "Tab",
        "Alt+B",
    ];
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    for _ in 0..300 {
        let mut entries: Vec<String> = Vec::new();
        for _ in 0..(1 + next(8)) {
            let row = rows[next(rows.len())];
            let keys: Vec<&str> = (0..next(3)).map(|_| pool[next(pool.len())]).collect();
            let mut value = serde_json::json!({
                "namespace": row.namespace.name(), "action": row.action, "keys": keys
            });
            if let (Some(c), true) = (row.context, next(4) > 0) {
                value["context"] = serde_json::json!(c.literal());
            }
            match next(12) {
                0 => entries.push("\"not an object\"".to_string()),
                1 => entries.push(serde_json::json!({"namespace": "board.nowhere", "action": row.action, "keys": keys}).to_string()),
                _ => entries.push(value.to_string()),
            }
        }
        let (table, errors) = merge_overrides(&file_of(&entries));
        assert_well_formed(table);
        assert!(errors.len() <= entries.len());
    }
}
