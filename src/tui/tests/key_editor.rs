//! The key editor (`KeyEditor` and its rules in
//! `docs/specs/keybindings.allium`): opening it from the help overlay,
//! capturing a press as a key, staging, the live checks, saving and closing.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::keybindings::{set_key_overrides, KeyChange, KeyNamespace, KeySetError, KEY_BINDINGS};
use crate::tui::commands::{SettingsCommand, UsageCommand};
use crate::tui::key_editor::KeyEditor;
use crate::tui::messages::KeyEditorMessage;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(code: KeyCode) -> KeyEvent {
    let shift = matches!(code, KeyCode::Char(c) if c.is_ascii_uppercase());
    KeyEvent::new(
        code,
        if shift {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        },
    )
}

fn ch(c: char) -> KeyEvent {
    key(KeyCode::Char(c))
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn recorded(cmds: &[Command]) -> Vec<String> {
    cmds.iter()
        .filter_map(|c| match c {
            Command::Usage(UsageCommand::Record(e)) => Some(e.action.clone()),
            _ => None,
        })
        .collect()
}

fn row_of(ns: KeyNamespace, action: &str) -> usize {
    KEY_BINDINGS
        .iter()
        .position(|b| b.namespace == ns && b.action == action && b.context.is_none())
        .unwrap()
}

fn editor(app: &App) -> &KeyEditor {
    app.input
        .key_editor
        .as_deref()
        .expect("the key editor is open")
}

fn staged(app: &App, row: usize) -> Option<Vec<String>> {
    editor(app).staged.get(&row).cloned()
}

fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
    let area = buf.area();
    (area.top()..area.bottom())
        .map(|y| buffer_line(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

fn strings(keys: &[&str]) -> Vec<String> {
    keys.iter().map(|k| (*k).to_string()).collect()
}

/// A board with the help overlay open and the override file at a path of
/// its own.
fn help_app() -> App {
    let mut app = make_app();
    app.key_override_file = Some(std::path::PathBuf::from("/data/keybindings.json"));
    app.update(Message::System(
        crate::tui::messages::SystemMessage::ToggleHelp,
    ));
    assert_eq!(app.input.mode, InputMode::Help);
    app
}

/// The key editor open, its cursor on `row`.
fn editor_on(row: usize) -> App {
    let mut app = help_app();
    app.handle_key(ch('e'));
    let e = app.input.key_editor.as_deref_mut().unwrap();
    e.cursor = e
        .visible(KEY_BINDINGS)
        .iter()
        .position(|&r| r == row)
        .unwrap();
    e.picked = 0;
    app
}

#[test]
fn e_in_the_help_overlay_opens_the_editor_and_reads_the_file() {
    let mut app = help_app();
    let cmds = app.handle_key(ch('e'));
    assert_eq!(app.input.mode, InputMode::KeyEditor);
    assert!(editor(&app).staged.is_empty());
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Settings(SettingsCommand::ReadKeyOverrides { file })
            if file == std::path::Path::new("/data/keybindings.json")
    )));
    assert_eq!(recorded(&cmds), vec!["open_key_editor"]);
}

#[test]
fn closing_with_nothing_staged_returns_to_the_help_overlay_where_it_was() {
    let mut app = help_app();
    app.interaction.help_scroll = 3;
    app.handle_key(ch('e'));
    let cmds = app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Help);
    assert!(app.input.key_editor.is_none());
    assert_eq!(app.interaction.help_scroll, 3);
    assert_eq!(recorded(&cmds), vec!["key_editor_close"]);
}

#[test]
fn a_capture_takes_the_next_press_a_modified_one_included() {
    let row = row_of(KeyNamespace::BoardNormal, "navigate_row_last");
    let mut app = editor_on(row);
    app.handle_key(ch('a'));
    assert_eq!(app.input.mode, InputMode::KeyCapture);
    let cmds = app.handle_key(ctrl('k'));
    assert!(recorded(&cmds).is_empty(), "the captured press is data");
    assert_eq!(app.input.mode, InputMode::KeyEditor);
    assert_eq!(staged(&app, row), Some(strings(&["]", "G", "Ctrl+K"])));
}

#[test]
fn a_letter_the_editor_binds_is_captured_as_data() {
    let row = row_of(KeyNamespace::BoardNormal, "navigate_row_last");
    let mut app = editor_on(row);
    app.handle_key(ch('a'));
    app.handle_key(ch('q'));
    assert_eq!(
        app.input.mode,
        InputMode::KeyEditor,
        "q did not close the editor"
    );
    assert_eq!(staged(&app, row), Some(strings(&["]", "G", "q"])));
}

#[test]
fn esc_leaves_a_capture_and_stages_nothing() {
    let row = row_of(KeyNamespace::BoardNormal, "navigate_row_last");
    let mut app = editor_on(row);
    app.handle_key(ch('a'));
    let cmds = app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::KeyEditor);
    assert_eq!(staged(&app, row), None);
    assert_eq!(recorded(&cmds), vec!["cancel_key_capture"]);
}

#[test]
fn a_chord_is_two_characters_and_never_a_named_key() {
    let row = row_of(KeyNamespace::BoardNormal, "navigate_row_last");
    let mut app = editor_on(row);
    app.handle_key(ch('c'));
    app.handle_key(ch('x'));
    assert_eq!(
        app.input.mode,
        InputMode::KeyCapture,
        "waiting for the second half"
    );
    app.handle_key(ch('y'));
    assert_eq!(staged(&app, row), Some(strings(&["]", "G", "xy"])));

    let mut app = editor_on(row);
    app.handle_key(ch('c'));
    app.handle_key(ch('U'));
    app.handle_key(ch('p'));
    assert_eq!(staged(&app, row), None, "Up is a named key, not a chord");
    assert!(editor(&app)
        .notice
        .as_deref()
        .unwrap()
        .contains("not a chord"));

    let mut app = editor_on(row);
    app.handle_key(ch('c'));
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::KeyEditor);
    assert_eq!(staged(&app, row), None, "a chord is characters only");
}

#[test]
fn replace_puts_the_captured_key_in_place_of_the_picked_one() {
    let row = row_of(KeyNamespace::BoardNormal, "navigate_row_last");
    let mut app = editor_on(row);
    app.handle_key(ch('l'));
    app.handle_key(key(KeyCode::Enter));
    app.handle_key(ch('V'));
    assert_eq!(staged(&app, row), Some(strings(&["]", "V"])));
}

#[test]
fn removing_the_last_key_changes_nothing_and_records_nothing() {
    let row = row_of(KeyNamespace::BoardNormal, "navigate_row_last");
    let mut app = editor_on(row);
    let cmds = app.handle_key(ch('x'));
    assert_eq!(recorded(&cmds), vec!["key_editor_remove_key"]);
    assert_eq!(staged(&app, row), Some(strings(&["G"])));
    let cmds = app.handle_key(ch('x'));
    assert!(recorded(&cmds).is_empty());
    assert_eq!(staged(&app, row), Some(strings(&["G"])));
    assert!(editor(&app).notice.is_some());
}

#[test]
fn restoring_a_row_with_a_file_entry_stages_its_removal() {
    let row = row_of(KeyNamespace::BoardNormal, "navigate_row_last");
    let mut app = editor_on(row);
    let cmds = app.handle_key(ch('r'));
    assert!(recorded(&cmds).is_empty(), "nothing to restore");

    let entry = serde_json::json!({"namespace": "board.normal", "action": "navigate_row_last", "keys": ["V"]});
    app.update(Message::KeyEditor(KeyEditorMessage::FileRead(Ok(vec![
        entry,
    ]))));
    assert_eq!(editor(&app).file_keys(row), strings(&["V"]).as_slice());
    let cmds = app.handle_key(ch('r'));
    assert_eq!(recorded(&cmds), vec!["key_editor_restore_default"]);
    assert_eq!(staged(&app, row), Some(vec![]));
    assert_eq!(
        editor(&app).changes()[0].keys,
        Vec::<String>::new(),
        "an empty list removes the entry, as keybinding_set takes it"
    );
}

#[test]
fn a_tmux_row_takes_the_key_after_the_prefix() {
    let row = row_of(KeyNamespace::TmuxGlobal, "toggle_agent_tree");
    let mut app = editor_on(row);
    app.handle_key(key(KeyCode::Enter));
    app.handle_key(ch('t'));
    assert_eq!(staged(&app, row), Some(strings(&["Prefix+t"])));
    assert!(editor(&app).problems.is_empty());
}

/// StagedEditsAreChecked: a clash shows under its row with the words
/// keybinding_set refuses the same change with.
#[test]
fn a_clash_is_shown_with_keybinding_sets_message_before_it_is_saved() {
    let row = row_of(KeyNamespace::BoardNormal, "navigate_row_last");
    let mut app = editor_on(row);
    app.handle_key(ch('a'));
    app.handle_key(ch('j'));
    let shown: Vec<String> = editor(&app).problems_of(row).map(str::to_string).collect();
    assert_eq!(shown.len(), 1, "{:?}", editor(&app).problems);

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("keybindings.json");
    let refused = set_key_overrides(&file, &[KeyChange::for_row(row, strings(&["]", "G", "j"]))]);
    let Err(KeySetError::Refused(messages)) = refused else {
        panic!("keybinding_set accepted the clash");
    };
    assert_eq!(shown, messages);
    assert!(!file.exists());
}

#[test]
fn a_swap_clashes_after_its_first_edit_and_is_clean_after_its_second() {
    let down = row_of(KeyNamespace::BoardNormal, "navigate_row_down");
    let up = row_of(KeyNamespace::BoardNormal, "navigate_row_up");
    let mut app = editor_on(down);
    let e = app.input.key_editor.as_deref_mut().unwrap();
    e.stage(down, strings(&["k", "Up"]));
    assert!(!e.problems.is_empty());
    e.stage(up, strings(&["j", "Down"]));
    assert!(e.problems.is_empty(), "{:?}", e.problems);
    let cmds = app.handle_key(ch('w'));
    let changes = cmds
        .iter()
        .find_map(|c| match c {
            Command::Settings(SettingsCommand::SaveKeyOverrides { changes, .. }) => {
                Some(changes.clone())
            }
            _ => None,
        })
        .expect("w saves");
    assert_eq!(changes.len(), 2);
    assert_eq!(recorded(&cmds), vec!["key_editor_save"]);
}

#[test]
fn saving_with_nothing_staged_does_nothing() {
    let mut app = editor_on(0);
    let cmds = app.handle_key(ch('w'));
    assert!(cmds.is_empty(), "{cmds:?}");
}

#[test]
fn a_saved_write_clears_the_staged_edits_and_a_refused_one_keeps_them() {
    let row = row_of(KeyNamespace::BoardNormal, "navigate_row_last");
    let mut app = editor_on(row);
    app.handle_key(ch('a'));
    app.handle_key(ch('V'));
    app.update(Message::KeyEditor(KeyEditorMessage::Saved(Err(vec![
        "it clashes".to_string(),
    ]))));
    assert!(staged(&app, row).is_some());
    assert!(editor(&app)
        .notice
        .as_deref()
        .unwrap()
        .contains("it clashes"));

    app.update(Message::KeyEditor(KeyEditorMessage::Saved(Ok(true))));
    assert!(editor(&app).staged.is_empty());
    assert!(editor(&app)
        .notice
        .as_deref()
        .unwrap()
        .contains("next starts"));
}

#[test]
fn an_unreadable_file_is_shown_and_a_save_against_it_is_refused() {
    let row = row_of(KeyNamespace::BoardNormal, "navigate_row_last");
    let mut app = editor_on(row);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("keybindings.json");
    std::fs::write(&file, "{").unwrap();
    let read = crate::keybindings::read_override_entries(&file);
    app.update(Message::KeyEditor(KeyEditorMessage::FileRead(read)));
    assert!(editor(&app).file_error().is_some());
    app.handle_key(ch('a'));
    app.handle_key(ch('V'));
    let Err(KeySetError::Refused(messages)) = set_key_overrides(&file, &editor(&app).changes())
    else {
        panic!("keybinding_set wrote over an unreadable file");
    };
    let shown: Vec<String> = editor(&app)
        .problems
        .iter()
        .map(|p| p.message.clone())
        .collect();
    assert_eq!(shown, messages);
}

#[test]
fn closing_with_edits_staged_asks_first() {
    let row = row_of(KeyNamespace::BoardNormal, "navigate_row_last");
    let mut app = editor_on(row);
    app.handle_key(ch('a'));
    app.handle_key(ch('V'));
    app.handle_key(ch('q'));
    assert_eq!(app.input.mode, InputMode::ConfirmDiscardKeyEdits);
    let cmds = app.handle_key(ch('n'));
    assert_eq!(recorded(&cmds), vec!["confirm_discard_key_edits_no"]);
    assert_eq!(app.input.mode, InputMode::KeyEditor);
    assert!(staged(&app, row).is_some(), "the edits stay");

    app.handle_key(ch('q'));
    app.handle_key(ch('y'));
    assert_eq!(app.input.mode, InputMode::Help);
    assert!(app.input.key_editor.is_none());
}

#[test]
fn the_search_filters_the_rows_and_esc_clears_it() {
    let mut app = editor_on(0);
    app.handle_key(ch('/'));
    assert_eq!(app.input.mode, InputMode::KeyEditorSearch);
    for c in "toggle_agent_tree".chars() {
        app.handle_key(ch(c));
    }
    let rows = editor(&app).visible(KEY_BINDINGS);
    assert!(!rows.is_empty());
    assert!(rows
        .iter()
        .all(|&r| KEY_BINDINGS[r].action.contains("toggle_agent_tree")
            || KEY_BINDINGS[r]
                .description
                .to_lowercase()
                .contains("toggle_agent_tree")));
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::KeyEditor);
    assert_eq!(editor(&app).query, "toggle_agent_tree");

    app.handle_key(ch('/'));
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(editor(&app).query, "");
    assert_eq!(editor(&app).visible(KEY_BINDINGS).len(), KEY_BINDINGS.len());
}

#[test]
fn the_editor_shows_current_default_pending_and_staged_keys() {
    let row = row_of(KeyNamespace::BoardNormal, "navigate_row_last");
    let mut app = editor_on(row);
    let entry = serde_json::json!({"namespace": "board.normal", "action": "navigate_row_last", "keys": ["V"]});
    app.update(Message::KeyEditor(KeyEditorMessage::FileRead(Ok(vec![
        entry,
    ]))));
    let screen = buffer_text(&render_to_buffer(&mut app, 160, 50));
    assert!(screen.contains("after restart: V"), "{screen}");
    app.handle_key(key(KeyCode::Enter));
    app.handle_key(ch('P'));
    let screen = buffer_text(&render_to_buffer(&mut app, 160, 50));
    assert!(screen.contains("staged: P"), "{screen}");
}

/// The text inside the editor's frame, line by line joined with spaces and
/// runs of spaces collapsed, so a wrapped message reads as one string.
fn editor_text(app: &mut App, width: u16, height: u16) -> String {
    let buf = render_to_buffer(app, width, height);
    let area = buf.area();
    let inside: Vec<String> = (area.top()..area.bottom())
        .filter_map(|y| {
            let line = buffer_line(&buf, y);
            let start = line.find('║')?;
            let end = line.rfind('║')?;
            (end > start).then(|| line[start + '║'.len_utf8()..end].trim().to_string())
        })
        .collect();
    inside
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// On a small board, with the cursor far down the table, a staged clash's
/// whole message and the save key are on screen.
#[test]
fn a_late_rows_clash_and_the_save_key_fit_a_small_board() {
    let row = row_of(KeyNamespace::TmuxGlobal, "toggle_agent_tree");
    let mut app = editor_on(row);
    app.handle_key(key(KeyCode::Enter));
    app.handle_key(key(KeyCode::Char(' ')));
    let messages: Vec<String> = editor(&app).problems_of(row).map(str::to_string).collect();
    assert_eq!(messages.len(), 1, "{:?}", editor(&app).problems);
    let text = editor_text(&mut app, 100, 30);
    let message = messages[0].split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(text.contains(&message), "{message}\nnot in:\n{text}");
    assert!(text.contains("[w] save"), "{text}");
    assert!(text.contains("[Esc] close"), "{text}");
}

/// Two staged rows that clash with each other each show the problem.
#[test]
fn a_clash_between_two_staged_rows_shows_under_both() {
    let first = row_of(KeyNamespace::BoardNormal, "navigate_row_last");
    let second = row_of(KeyNamespace::BoardNormal, "navigate_row_first");
    let mut app = editor_on(first);
    let e = app.input.key_editor.as_deref_mut().unwrap();
    e.stage(first, strings(&["]", "G", "V"]));
    e.stage(second, strings(&["[", "gg", "V"]));
    assert!(e.problems_of(first).next().is_some(), "{:?}", e.problems);
    assert!(e.problems_of(second).next().is_some(), "{:?}", e.problems);
}

#[test]
fn restoring_a_row_whose_removal_is_staged_records_nothing() {
    let row = row_of(KeyNamespace::BoardNormal, "navigate_row_last");
    let mut app = editor_on(row);
    let entry = serde_json::json!({"namespace": "board.normal", "action": "navigate_row_last", "keys": ["V"]});
    app.update(Message::KeyEditor(KeyEditorMessage::FileRead(Ok(vec![
        entry,
    ]))));
    assert_eq!(
        recorded(&app.handle_key(ch('r'))),
        vec!["key_editor_restore_default"]
    );
    assert!(recorded(&app.handle_key(ch('r'))).is_empty());
}

/// What the operator sees: the editor on a small board, a row with a file
/// entry waiting for a restart, and a staged clash.
#[test]
fn snapshot_key_editor() {
    let row = row_of(KeyNamespace::BoardNormal, "navigate_row_last");
    let mut app = editor_on(row);
    let entry = serde_json::json!({"namespace": "board.normal", "action": "navigate_row_last", "keys": ["]", "V"]});
    app.update(Message::KeyEditor(KeyEditorMessage::FileRead(Ok(vec![
        entry,
    ]))));
    app.handle_key(ch('a'));
    app.handle_key(ch('j'));
    let buf = render_to_buffer(&mut app, 100, 30);
    insta::assert_snapshot!(buffer_text(&buf));
}
