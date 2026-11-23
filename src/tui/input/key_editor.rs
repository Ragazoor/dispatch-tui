//! The key editor's keys (`KeyEditor` in `docs/specs/keybindings.allium`):
//! moving over the rows, capturing a key press as data, staging, saving and
//! closing.

use crossterm::event::{KeyCode, KeyEvent};

use super::super::types::*;
use super::super::App;
use super::key_event;
use crate::keybindings::{key_has_ctrl_or_alt, key_name, KeyLoadError};
use crate::tui::commands::SettingsCommand;
use crate::tui::key_editor::{captured_chord, captured_key, CaptureKind, KeyEditor};

/// One step from `cur` towards the end (`forward`) or the start of a list
/// of `len` items, stopping at either end.
fn step(cur: usize, len: usize, forward: bool) -> usize {
    if forward {
        (cur + 1).min(len.saturating_sub(1))
    } else {
        cur.saturating_sub(1)
    }
}

/// What the editor says once a save wrote the file.
const SAVED: &str = "Saved. The keys apply when the board next starts; a companion or diff pane opened before then already uses them.";

/// What the editor says when the file already said what was staged.
const UNCHANGED: &str = "The file already says this; nothing was written.";

impl App {
    fn key_editor_mut(&mut self) -> Option<&mut KeyEditor> {
        self.input.key_editor.as_deref_mut()
    }

    /// `OpenKeyEditor`: from the help overlay, with nothing staged and the
    /// file read as it is now.
    pub(super) fn open_key_editor(&mut self, action: &str, label: &str) -> Vec<Command> {
        self.input.key_editor = Some(Box::default());
        self.input.mode = InputMode::KeyEditor;
        let mut cmds = Vec::new();
        if let Some(file) = &self.key_override_file {
            cmds.push(Command::Settings(SettingsCommand::ReadKeyOverrides {
                file: file.clone(),
            }));
        }
        cmds.push(key_event(action, label));
        cmds
    }

    /// Back to the help overlay, dropping whatever is staged.
    fn close_key_editor(&mut self) {
        self.input.key_editor = None;
        self.input.mode = InputMode::Help;
    }

    /// A row of board.key_editor. A press that changes nothing records
    /// nothing.
    pub(super) fn run_key_editor(&mut self, action: &'static str, label: &str) -> Vec<Command> {
        let table = self.key_table;
        let file = self.key_override_file.clone();
        let Some(editor) = self.input.key_editor.as_deref_mut() else {
            return vec![];
        };
        let row = editor.cursor_row(table);
        let mut cmds = Vec::new();
        let took_effect = match action {
            "key_editor_row_down" | "key_editor_row_up" => {
                let len = editor.visible(table).len();
                let to = step(editor.cursor, len, action == "key_editor_row_down");
                let moved = to != editor.cursor;
                editor.cursor = to;
                if moved {
                    editor.picked = 0;
                }
                moved
            }
            "key_editor_next_key" | "key_editor_previous_key" => {
                let Some(row) = row else { return vec![] };
                let len = editor.editing_keys(row).len();
                let to = step(editor.picked, len, action == "key_editor_next_key");
                let moved = to != editor.picked;
                editor.picked = to;
                moved
            }
            "key_editor_add_key" | "key_editor_add_chord" | "key_editor_replace_key" => {
                if row.is_none() {
                    return vec![];
                }
                editor.capture = Some(match action {
                    "key_editor_add_key" => CaptureKind::AddKey,
                    "key_editor_add_chord" => CaptureKind::AddChord,
                    _ => CaptureKind::ReplaceKey,
                });
                editor.chord_first = None;
                editor.notice = None;
                self.input.mode = InputMode::KeyCapture;
                true
            }
            "key_editor_remove_key" => {
                let Some(row) = row else { return vec![] };
                let mut keys = editor.editing_keys(row);
                if keys.len() <= 1 {
                    editor.notice = Some(
                        "A row keeps at least one key; restore its default keys instead."
                            .to_string(),
                    );
                    false
                } else {
                    keys.remove(editor.picked.min(keys.len() - 1));
                    editor.stage(row, keys);
                    true
                }
            }
            "key_editor_restore_default" => {
                let Some(row) = row else { return vec![] };
                let before = editor.staged.get(&row).cloned();
                editor.stage(row, Vec::new());
                editor.staged.get(&row) != before.as_ref()
            }
            "key_editor_save" => match file {
                Some(file) if !editor.staged.is_empty() => {
                    cmds.push(Command::Settings(SettingsCommand::SaveKeyOverrides {
                        file,
                        changes: editor.changes(),
                    }));
                    true
                }
                _ => false,
            },
            "key_editor_search" => {
                self.input.mode = InputMode::KeyEditorSearch;
                true
            }
            "key_editor_close" => {
                if editor.staged.is_empty() {
                    self.close_key_editor();
                } else {
                    self.input.mode = InputMode::ConfirmDiscardKeyEdits;
                }
                true
            }
            _ => false,
        };
        if took_effect {
            cmds.push(key_event(action, label));
        }
        cmds
    }

    /// The commit and cancel rows of board.key_editor_search.
    pub(super) fn run_key_editor_search(
        &mut self,
        action: &'static str,
        label: &str,
    ) -> Vec<Command> {
        let Some(editor) = self.key_editor_mut() else {
            return vec![];
        };
        match action {
            "key_editor_search_cancel" => {
                editor.query.clear();
                editor.cursor = 0;
                editor.picked = 0;
            }
            "key_editor_search_commit" => {}
            _ => return vec![],
        }
        self.input.mode = InputMode::KeyEditor;
        vec![key_event(action, label)]
    }

    /// A character typed into the row search, or Backspace (`erase`).
    /// Typing is not an action and records nothing.
    pub(super) fn edit_key_editor_query(&mut self, typed: Option<char>) {
        if let Some(editor) = self.key_editor_mut() {
            match typed {
                Some(c) => editor.query.push(c),
                None => {
                    editor.query.pop();
                }
            }
            editor.cursor = 0;
            editor.picked = 0;
        }
    }

    /// cancel_key_capture: leave capture with nothing staged.
    pub(super) fn run_key_capture(&mut self, action: &'static str, label: &str) -> Vec<Command> {
        let Some(editor) = self.key_editor_mut() else {
            return vec![];
        };
        editor.capture = None;
        editor.chord_first = None;
        self.input.mode = InputMode::KeyEditor;
        vec![key_event(action, label)]
    }

    /// `CaptureTakesThePress`: a press no row of board.key_capture lists is
    /// the key being captured. It records nothing; the key that opened the
    /// capture did.
    pub(super) fn capture_key_press(&mut self, key: KeyEvent) -> Vec<Command> {
        let table = self.key_table;
        let Some(editor) = self.input.key_editor.as_deref_mut() else {
            return vec![];
        };
        let (Some(kind), Some(row)) = (editor.capture, editor.cursor_row(table)) else {
            return vec![];
        };
        let captured = match kind {
            CaptureKind::AddChord => {
                let typed = match key.code {
                    KeyCode::Char(c) if c != ' ' && !key_has_ctrl_or_alt(key) => Some(c),
                    _ => None,
                };
                match (typed, editor.chord_first) {
                    (None, _) => Err("A chord is two characters; nothing was added.".to_string()),
                    (Some(c), None) => {
                        editor.chord_first = Some(c);
                        editor.notice = Some(format!("{c} … now the second character."));
                        return vec![];
                    }
                    (Some(c), Some(first)) => captured_chord(first, c),
                }
            }
            CaptureKind::AddKey | CaptureKind::ReplaceKey => captured_key(row, &key_name(key)),
        };
        editor.capture = None;
        editor.chord_first = None;
        self.input.mode = InputMode::KeyEditor;
        match captured {
            Err(message) => editor.notice = Some(message),
            Ok(k) => {
                editor.notice = None;
                let mut keys = editor.editing_keys(row);
                if kind == CaptureKind::ReplaceKey {
                    let at = editor.picked.min(keys.len().saturating_sub(1));
                    keys[at] = k;
                } else {
                    keys.push(k);
                }
                let mut seen = std::collections::HashSet::new();
                keys.retain(|k| seen.insert(k.clone()));
                editor.stage(row, keys);
            }
        }
        vec![]
    }

    /// board.confirm.discard_key_edits: yes drops the staged edits and
    /// closes the editor, anything else stays.
    pub(super) fn run_confirm_discard_key_edits(&mut self, yes: bool, label: &str) -> Vec<Command> {
        if yes {
            self.close_key_editor();
            vec![key_event("confirm_discard_key_edits_yes", label)]
        } else {
            self.input.mode = InputMode::KeyEditor;
            vec![key_event("confirm_discard_key_edits_no", label)]
        }
    }

    pub(in crate::tui) fn handle_key_editor_file_read(
        &mut self,
        read: Result<Vec<serde_json::Value>, KeyLoadError>,
    ) -> Vec<Command> {
        if let Some(editor) = self.key_editor_mut() {
            editor.set_file(read);
        }
        vec![]
    }

    pub(in crate::tui) fn handle_key_editor_saved(
        &mut self,
        result: Result<bool, Vec<String>>,
    ) -> Vec<Command> {
        if let Some(editor) = self.key_editor_mut() {
            match result {
                Ok(written) => {
                    editor.staged.clear();
                    editor.problems.clear();
                    editor.notice = Some(if written { SAVED } else { UNCHANGED }.to_string());
                }
                Err(problems) => {
                    editor.notice = Some(format!("Nothing was saved: {}", problems.join(" ")));
                }
            }
        }
        vec![]
    }
}
