//! The board's key editor (`KeyEditor` in `docs/specs/keybindings.allium`):
//! the rows, what the override file gives them, the staged edits and the
//! problems the loader's checks find in them. Pure state; the keys that drive
//! it are in `input::key_editor` and the reads and writes of the file are
//! commands.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::keybindings::{
    check_overrides, is_chord, is_key_name, plan_key_overrides, unreadable_file_refusal,
    ChangeProblem, KeyBinding, KeyChange, KeyLoadError, KeyNamespace, KEY_BINDINGS, TMUX_PREFIX,
};

/// What the editor does with the press it waits for (`KeyCaptureKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureKind {
    AddKey,
    AddChord,
    ReplaceKey,
}

/// How the override file has a row: no entry, an entry the load accepts, or
/// one it refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileEntry {
    None,
    Accepted,
    Refused,
}

#[derive(Debug, Clone)]
pub struct KeyEditor {
    /// The file's entries as last read, or the message a writer refuses an
    /// unreadable file with.
    entries: Result<Vec<Value>, String>,
    /// The keys the file gives each row now, in `KEY_BINDINGS` order.
    file_keys: Vec<Vec<String>>,
    file_entries: Vec<FileEntry>,
    /// The file's own refused entries, as the load reports them.
    pub file_errors: Vec<KeyLoadError>,
    /// Staged keys by row; an empty list removes the row's entry.
    pub staged: BTreeMap<usize, Vec<String>>,
    pub problems: Vec<ChangeProblem>,
    /// The cursor, as a position in [`Self::visible`].
    pub cursor: usize,
    /// The picked key of the cursor row.
    pub picked: usize,
    pub query: String,
    pub capture: Option<CaptureKind>,
    pub chord_first: Option<char>,
    /// The last thing the editor has to tell the user.
    pub notice: Option<String>,
    /// The first line drawn, kept so the cursor row stays on screen.
    pub scroll: std::cell::Cell<usize>,
}

impl Default for KeyEditor {
    fn default() -> Self {
        let mut editor = Self {
            entries: Ok(Vec::new()),
            file_keys: Vec::new(),
            file_entries: Vec::new(),
            file_errors: Vec::new(),
            staged: BTreeMap::new(),
            problems: Vec::new(),
            cursor: 0,
            picked: 0,
            query: String::new(),
            capture: None,
            chord_first: None,
            notice: None,
            scroll: std::cell::Cell::new(0),
        };
        editor.set_file(Ok(Vec::new()));
        editor
    }
}

fn default_keys(row: usize) -> Vec<String> {
    KEY_BINDINGS[row]
        .keys
        .iter()
        .map(|k| (*k).to_string())
        .collect()
}

impl KeyEditor {
    /// Take the file as read now, and check the staged edits against it.
    pub fn set_file(&mut self, read: Result<Vec<Value>, KeyLoadError>) {
        let rows = KEY_BINDINGS.len();
        self.file_entries = vec![FileEntry::None; rows];
        match read {
            Ok(entries) => {
                let check = check_overrides(&entries);
                for e in &check.entries {
                    if let Some(row) = e.row {
                        self.file_entries[row] = if e.error.is_none() {
                            FileEntry::Accepted
                        } else {
                            FileEntry::Refused
                        };
                    }
                }
                self.file_errors = check.errors();
                self.file_keys = check.keys;
                self.entries = Ok(entries);
            }
            Err(e) => {
                self.file_keys = (0..rows).map(default_keys).collect();
                self.entries = Err(unreadable_file_refusal(&e));
                self.file_errors = vec![e];
            }
        }
        self.check();
    }

    /// Why the file cannot be written, when it cannot.
    pub fn file_error(&self) -> Option<&str> {
        self.entries.as_ref().err().map(String::as_str)
    }

    /// The keys the file gives `row` now.
    pub fn file_keys(&self, row: usize) -> &[String] {
        &self.file_keys[row]
    }

    pub fn has_file_entry(&self, row: usize) -> bool {
        self.file_entries[row] != FileEntry::None
    }

    /// The rows the search keeps, by index into `table`, in table order
    /// grouped by namespace.
    pub fn visible(&self, table: &[KeyBinding]) -> Vec<usize> {
        let q = self.query.to_lowercase();
        KeyNamespace::ALL
            .iter()
            .flat_map(|ns| {
                table
                    .iter()
                    .enumerate()
                    .filter(move |(_, b)| b.namespace == *ns)
            })
            .filter(|(_, b)| {
                q.is_empty()
                    || b.namespace.name().to_lowercase().contains(&q)
                    || b.action.to_lowercase().contains(&q)
                    || b.description.to_lowercase().contains(&q)
                    || b.keys.iter().any(|k| k.to_lowercase().contains(&q))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// The row under the cursor.
    pub fn cursor_row(&self, table: &[KeyBinding]) -> Option<usize> {
        self.visible(table).get(self.cursor).copied()
    }

    /// The keys an edit of `row` starts from: its staged keys, else the keys
    /// the file gives it.
    pub fn editing_keys(&self, row: usize) -> Vec<String> {
        match self.staged.get(&row) {
            Some(keys) if keys.is_empty() => default_keys(row),
            Some(keys) => keys.clone(),
            None => self.file_keys[row].clone(),
        }
    }

    /// Stage `keys` for `row` (an empty list restores its default keys) and
    /// check every staged edit again. An edit that leaves the row as the file
    /// has it is dropped.
    pub fn stage(&mut self, row: usize, keys: Vec<String>) {
        let same_as_file = if keys.is_empty() {
            self.file_entries[row] == FileEntry::None
        } else {
            keys == self.file_keys[row]
                && match self.file_entries[row] {
                    FileEntry::Accepted => true,
                    FileEntry::None => keys == default_keys(row),
                    FileEntry::Refused => false,
                }
        };
        if same_as_file {
            self.staged.remove(&row);
        } else {
            self.staged.insert(row, keys);
        }
        self.picked = self
            .picked
            .min(self.editing_keys(row).len().saturating_sub(1));
        self.check();
    }

    /// The staged edits as one keybinding_set call's changes.
    pub fn changes(&self) -> Vec<KeyChange> {
        self.staged
            .iter()
            .map(|(&row, keys)| KeyChange::for_row(row, keys.clone()))
            .collect()
    }

    /// Run keybinding_set's checks over the file and the staged edits
    /// (`StagedEditsAreChecked`).
    fn check(&mut self) {
        self.problems = if self.staged.is_empty() {
            Vec::new()
        } else {
            match &self.entries {
                Err(message) => vec![ChangeProblem {
                    row: None,
                    message: message.clone(),
                }],
                Ok(entries) => plan_key_overrides(entries, &self.changes())
                    .err()
                    .unwrap_or_default(),
            }
        };
    }

    /// The problems of `row`'s staged edit.
    pub fn problems_of(&self, row: usize) -> impl Iterator<Item = &str> {
        self.problems
            .iter()
            .filter(move |p| p.row == Some(row))
            .map(|p| p.message.as_str())
    }
}

/// The key a capture stages for `row` from the pressed key's name: the
/// name itself, or "Prefix+" and the name for a tmux.global row, whose
/// prefix press the board never sees (`CaptureTakesThePress`).
pub fn captured_key(row: usize, name: &str) -> Result<String, String> {
    if !is_key_name(name) {
        return Err(format!("That key has no name dispatch knows ({name})."));
    }
    Ok(if KEY_BINDINGS[row].namespace == KeyNamespace::TmuxGlobal {
        format!("{TMUX_PREFIX}{name}")
    } else {
        name.to_string()
    })
}

/// The chord two pressed characters make, or why they make none.
pub fn captured_chord(first: char, second: char) -> Result<String, String> {
    let chord: String = [first, second].iter().collect();
    if is_chord(&chord) {
        Ok(chord)
    } else {
        Err(format!(
            "\"{chord}\" is the name of a key, not a chord; press two other characters."
        ))
    }
}
