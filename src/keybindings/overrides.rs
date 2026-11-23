//! The user's key overrides: the file in the data directory that replaces the
//! keys of chosen rows, and the merge of the default table with the entries
//! it accepts. See `LoadKeyTable` in `docs/specs/keybindings.allium`.
//!
//! The load never fails. Whatever the file holds, the result is a table that
//! keeps every invariant of the default table: a refused entry leaves its row
//! on its default keys and adds one [`KeyLoadError`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    contexts_overlap, is_chord, modified_char, KeyBinding, KeyContext, KeyNamespace, ANY_OTHER_KEY,
    KEY_BINDINGS,
};

/// The override file's name in the data directory
/// (`config.key_override_file_name`).
pub const KEY_OVERRIDE_FILE_NAME: &str = "keybindings.json";

/// The named keys, as the table writes them (`is_key_name`).
pub const NAMED_KEYS: [&str; 28] = [
    "Space",
    "Enter",
    "Esc",
    "Tab",
    "BackTab",
    "Backspace",
    "Delete",
    "Insert",
    "Up",
    "Down",
    "Left",
    "Right",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
];

/// The three modified names a terminal delivers as another key: Ctrl+I is
/// Tab, Ctrl+M is Enter and Ctrl+[ is Esc.
const TERMINAL_ALIASES: [(&str, &str); 3] =
    [("Ctrl+I", "Tab"), ("Ctrl+M", "Enter"), ("Ctrl+[", "Esc")];

/// The modifier prefixes a modified key may carry, longest first.
const MODIFIERS: [&str; 3] = ["Ctrl+Alt+", "Ctrl+", "Alt+"];

/// The tmux prefix a tmux.global key starts with.
const TMUX_PREFIX: &str = "Prefix+";

fn is_named_key(k: &str) -> bool {
    NAMED_KEYS.contains(&k)
}

/// One printable character other than space, as typed.
fn is_character(k: &str) -> bool {
    let mut chars = k.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if !c.is_whitespace() && !c.is_control())
}

/// A character or a named key, with or without a Ctrl/Alt modifier.
fn is_single_key(k: &str) -> bool {
    if is_character(k) || is_named_key(k) {
        return true;
    }
    if TERMINAL_ALIASES.iter().any(|(alias, _)| *alias == k) {
        return false;
    }
    MODIFIERS.iter().any(|m| {
        k.strip_prefix(m).is_some_and(|base| {
            is_named_key(base)
                || (is_character(base) && base.chars().all(|c| modified_char(c) == base))
        })
    })
}

/// Whether `k` is a canonical key name (`is_key_name` in the spec): a
/// character, a named key, a modified key, a two-character chord, or
/// `Prefix+` before one of the first three. Lookup is an exact match on the
/// name, so nothing else may be listed.
pub fn is_key_name(k: &str) -> bool {
    if let Some(rest) = k.strip_prefix(TMUX_PREFIX) {
        return is_single_key(rest);
    }
    is_single_key(k) || (is_chord(k) && k.chars().all(|c| is_character(&c.to_string())))
}

/// A tmux.global key in tmux's own spelling (`TmuxKeysComeFromTheTable`):
/// the part after "Prefix+", a named key by tmux's name for it, "Ctrl+X" as
/// "C-x" and "Alt+X" as "M-x". `None` when `key` is not "Prefix+" and one key.
pub fn tmux_key_name(key: &str) -> Option<String> {
    let rest = key.strip_prefix(TMUX_PREFIX)?;
    if !is_single_key(rest) {
        return None;
    }
    let (modifier, base) = MODIFIERS
        .iter()
        .find_map(|m| rest.strip_prefix(m).map(|base| (*m, base)))
        .unwrap_or(("", rest));
    let tmux_modifier = match modifier {
        "Ctrl+Alt+" => "C-M-",
        "Ctrl+" => "C-",
        "Alt+" => "M-",
        _ => "",
    };
    let tmux_base = match base {
        "Esc" => "Escape".to_string(),
        "BackTab" => "BTab".to_string(),
        "Backspace" => "BSpace".to_string(),
        "Delete" => "DC".to_string(),
        "Insert" => "IC".to_string(),
        "PageUp" => "PPage".to_string(),
        "PageDown" => "NPage".to_string(),
        // tmux reads a bare `;` argument as the end of a command.
        ";" => "\\;".to_string(),
        named if is_named_key(named) => named.to_string(),
        c if !modifier.is_empty() => c.to_lowercase(),
        c => c.to_string(),
    };
    Some(format!("{tmux_modifier}{tmux_base}"))
}

/// A key typed as data in a text namespace: a character or Space.
pub fn is_typed_key(k: &str) -> bool {
    is_character(k) || k == "Space"
}

/// The six namespaces that take typed text (`namespace_takes_text`).
pub fn namespace_takes_text(ns: KeyNamespace) -> bool {
    matches!(
        ns,
        KeyNamespace::BoardText
            | KeyNamespace::BoardSearch
            | KeyNamespace::BoardFilterSearch
            | KeyNamespace::BoardPickerRepoPath
            | KeyNamespace::BoardPickerBaseBranch
            | KeyNamespace::BoardPickerQuickDispatch
    )
}

/// Why `key` may not be a key of a row of `ns`, or `None` when it may
/// (`TextNamespacesBindNoTypedKey`, the tmux.global key form, chords).
pub fn key_not_allowed_in(ns: KeyNamespace, key: &str) -> Option<String> {
    let tmux_key = key.starts_with(TMUX_PREFIX);
    if ns == KeyNamespace::TmuxGlobal {
        return (!tmux_key).then(|| {
            format!("tmux.global keys are \"Prefix+\" and one key (\"Prefix+e\"); `{key}` is not")
        });
    }
    if tmux_key {
        return Some(format!(
            "`{key}` is a tmux key; only tmux.global rows take \"Prefix+\" keys"
        ));
    }
    if namespace_takes_text(ns) && (is_typed_key(key) || is_chord(key)) {
        return Some(format!(
            "{} takes typed text, so `{key}` there is typed, not a key; use a named key other than Space (Esc, F2) or a Ctrl/Alt key (Ctrl+G)",
            ns.name()
        ));
    }
    None
}

/// The canonical spelling of a near-miss key name ("ctrl+d" is "Ctrl+D",
/// "escape" is "Esc"), or `None` when there is no obvious one.
pub fn suggest_key_name(k: &str) -> Option<String> {
    let parts: Vec<&str> = if k.len() > 1 && k.ends_with('+') {
        // "Ctrl++": the key is the plus sign itself.
        let head = &k[..k.len() - 1];
        let mut parts: Vec<&str> = head.split('+').filter(|p| !p.is_empty()).collect();
        parts.push("+");
        parts
    } else {
        k.split('+').collect()
    };
    let (base, mods) = parts.split_last()?;
    let (mut prefix, mut ctrl, mut alt, mut shift) = (false, false, false, false);
    for m in mods {
        match m.to_ascii_lowercase().as_str() {
            "prefix" => prefix = true,
            "ctrl" | "control" | "c" => ctrl = true,
            "alt" | "meta" | "option" | "m" => alt = true,
            "shift" | "s" => shift = true,
            _ => return None,
        }
    }
    let lower = base.to_ascii_lowercase();
    let named = match lower.as_str() {
        "escape" => Some("Esc"),
        "return" | "enter" => Some("Enter"),
        "del" => Some("Delete"),
        "ins" => Some("Insert"),
        "pgup" => Some("PageUp"),
        "pgdn" | "pgdown" => Some("PageDown"),
        "spacebar" | " " => Some("Space"),
        "backtab" => Some("BackTab"),
        "tab" if shift => Some("BackTab"),
        _ => NAMED_KEYS
            .iter()
            .copied()
            .find(|n| n.eq_ignore_ascii_case(base)),
    };
    let key = match named {
        Some(n) => n.to_string(),
        None if base.chars().count() == 1 => match base.chars().next() {
            Some(c) if shift || ctrl || alt => modified_char(c),
            _ => (*base).to_string(),
        },
        None => return None,
    };
    let mut name = String::new();
    if prefix {
        name.push_str(TMUX_PREFIX);
    }
    if ctrl {
        name.push_str("Ctrl+");
    }
    if alt {
        name.push_str("Alt+");
    }
    name.push_str(&key);
    if let Some((_, key)) = TERMINAL_ALIASES.iter().find(|(alias, _)| *alias == name) {
        return Some((*key).to_string());
    }
    (is_key_name(&name) && name != k).then_some(name)
}

/// Why an entry of the override file is refused (`KeyOverrideProblem`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyOverrideProblem {
    FileUnreadable,
    MalformedEntry,
    DuplicateEntry,
    UnknownNamespace,
    UnknownAction,
    UnknownContext,
    AmbiguousRow,
    EmptyKeys,
    UnknownKeyName,
    KeyNotAllowedInNamespace,
    CatchAllRow,
    KeyClash,
    ChordPrefixClash,
}

impl KeyOverrideProblem {
    /// Every problem, in the spec's order.
    pub const ALL: [KeyOverrideProblem; 13] = [
        Self::FileUnreadable,
        Self::MalformedEntry,
        Self::DuplicateEntry,
        Self::UnknownNamespace,
        Self::UnknownAction,
        Self::UnknownContext,
        Self::AmbiguousRow,
        Self::EmptyKeys,
        Self::UnknownKeyName,
        Self::KeyNotAllowedInNamespace,
        Self::CatchAllRow,
        Self::KeyClash,
        Self::ChordPrefixClash,
    ];

    /// The spec's literal.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FileUnreadable => "file_unreadable",
            Self::MalformedEntry => "malformed_entry",
            Self::DuplicateEntry => "duplicate_entry",
            Self::UnknownNamespace => "unknown_namespace",
            Self::UnknownAction => "unknown_action",
            Self::UnknownContext => "unknown_context",
            Self::AmbiguousRow => "ambiguous_row",
            Self::EmptyKeys => "empty_keys",
            Self::UnknownKeyName => "unknown_key_name",
            Self::KeyNotAllowedInNamespace => "key_not_allowed_in_namespace",
            Self::CatchAllRow => "catch_all_row",
            Self::KeyClash => "key_clash",
            Self::ChordPrefixClash => "chord_prefix_clash",
        }
    }
}

/// One line of what a load reports: the problem, the entry it is about (by
/// the names the entry gave; all absent for `file_unreadable`) and a message
/// that says what to write instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyLoadError {
    pub problem: KeyOverrideProblem,
    pub namespace: Option<String>,
    pub action: Option<String>,
    pub context: Option<String>,
    pub message: String,
    /// The message as the board's popup shows it, when that differs: a long
    /// list of valid names cut short. See [`KeyLoadError::popup_text`].
    pub popup_message: Option<String>,
}

impl KeyLoadError {
    /// The message as the board's popup shows it.
    pub fn popup_text(&self) -> &str {
        self.popup_message.as_deref().unwrap_or(&self.message)
    }
}

/// Which process is loading the table (`KeyTableReader`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyTableReader {
    Board,
    CompanionPane,
    DiffPane,
}

/// One process's load of the table: the merged table, in the default
/// table's row order, and what the load refused.
#[derive(Debug, Clone)]
pub struct KeyTableLoad {
    pub file: PathBuf,
    pub table: &'static [KeyBinding],
    pub errors: Vec<KeyLoadError>,
}

impl KeyTableLoad {
    /// The load of a data directory with no override file: the default
    /// table and no errors. For a caller with no file to read, such as a test.
    pub fn defaults(data_dir: &Path) -> Self {
        Self {
            file: data_dir.join(KEY_OVERRIDE_FILE_NAME),
            table: KEY_BINDINGS,
            errors: Vec::new(),
        }
    }

    /// The text the board shows for a load that refused anything
    /// (`BoardShowsKeyLoadErrors`): the file, that the rows keep their
    /// default keys, and each error's message. `None` with no errors.
    pub fn error_summary(&self) -> Option<String> {
        if self.errors.is_empty() {
            return None;
        }
        let mut text = format!(
            "Key overrides in {} were refused; their rows keep their default keys.",
            self.file.display()
        );
        for e in self.errors.iter().take(POPUP_ERRORS) {
            text.push_str("\n- ");
            text.push_str(e.popup_text());
        }
        if self.errors.len() > POPUP_ERRORS {
            text.push_str(&format!(
                "\n… and {} more, listed in app.log",
                self.errors.len() - POPUP_ERRORS
            ));
        }
        Some(text)
    }
}

/// How many valid names a popup message lists before it points to the log,
/// which holds the whole list.
const POPUP_LIST_NAMES: usize = 5;

/// A message that ends in a list of valid names: whole for the log, and, when
/// the list is longer than [`POPUP_LIST_NAMES`], cut short for the popup.
fn listing(head: &str, names: &[String]) -> (String, Option<String>) {
    let whole = format!("{head}{}", names.join(", "));
    let short = (names.len() > POPUP_LIST_NAMES).then(|| {
        format!(
            "{head}{}, … and {} more (all are listed in app.log)",
            names[..POPUP_LIST_NAMES].join(", "),
            names.len() - POPUP_LIST_NAMES
        )
    });
    (whole, short)
}

/// How many refusals the popup lists before it points to the log.
const POPUP_ERRORS: usize = 3;

/// Build this process's table (`LoadKeyTable`): the default rows with the
/// accepted entries of `<data_dir>/keybindings.json` applied. A missing file
/// is the default table. Every error is logged (`KeyLoadErrorsAreLogged`).
/// The table is leaked: it is built once per process and lives as long as it.
pub fn load_key_table(data_dir: &Path, reader: KeyTableReader) -> KeyTableLoad {
    let file = data_dir.join(KEY_OVERRIDE_FILE_NAME);
    let (table, errors) = match read_override_entries(&file) {
        Ok(list) => merge_entries(&list),
        Err(e) => (KEY_BINDINGS, vec![e]),
    };
    for e in &errors {
        tracing::warn!(
            ?reader,
            problem = e.problem.as_str(),
            file = %file.display(),
            "key override refused: {}",
            e.message
        );
    }
    KeyTableLoad {
        file,
        table,
        errors,
    }
}

fn file_unreadable(message: String) -> KeyLoadError {
    KeyLoadError {
        problem: KeyOverrideProblem::FileUnreadable,
        namespace: None,
        action: None,
        context: None,
        message,
        popup_message: None,
    }
}

/// One entry of the file as the load judged it.
struct Entry {
    namespace: Option<String>,
    action: Option<String>,
    context: Option<String>,
    /// The row of `KEY_BINDINGS` the entry names, once resolved.
    row: Option<usize>,
    keys: Vec<String>,
    /// The problem, its message and the popup's shorter form of it, if any.
    problem: Option<Refusal>,
}

impl Entry {
    fn refuse(&mut self, problem: KeyOverrideProblem, message: String) {
        if self.problem.is_none() {
            self.problem = Some((problem, message, None));
        }
    }
}

/// The entries of the override file at `file`, as written: an empty list
/// when there is no file, and the one `file_unreadable` error when it cannot
/// be read or is not a JSON list. The loader, `keybinding_set` and
/// `keybinding_search` all read the file through this.
pub fn read_override_entries(file: &Path) -> Result<Vec<Value>, KeyLoadError> {
    match std::fs::read_to_string(file) {
        Ok(text) => parse_override_list(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(file_unreadable(format!(
            "{} cannot be read ({e}); no override applies. Make it a readable file, or remove it to use the default keys",
            file.display()
        ))),
    }
}

/// The file's text as a list of entries, or the one `file_unreadable` error.
fn parse_override_list(text: &str) -> Result<Vec<Value>, KeyLoadError> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Array(list)) => Ok(list),
        Ok(_) => Err(file_unreadable(format!(
            "{KEY_OVERRIDE_FILE_NAME} is not a JSON list; no override applies. Write a list of entries: [{{\"namespace\": \"board.normal\", \"action\": \"navigate_row_last\", \"keys\": [\"O\"]}}]"
        ))),
        Err(e) => Err(file_unreadable(format!(
            "{KEY_OVERRIDE_FILE_NAME} is not valid JSON ({e}); no override applies. Fix the JSON at that place, or remove the file to use the default keys"
        ))),
    }
}

/// The merge itself, over the file's text: the merged table (same rows, same
/// order as `KEY_BINDINGS`) and one error per refused entry, in file order.
/// It leaks the merged table (and each accepted entry's keys), which is right
/// once per process at startup. A caller that checks a file per request uses
/// [`check_overrides`], which runs the same checks and leaks nothing.
#[cfg(test)]
pub fn merge_overrides(text: &str) -> (&'static [KeyBinding], Vec<KeyLoadError>) {
    match parse_override_list(text) {
        Ok(list) => merge_entries(&list),
        Err(e) => (KEY_BINDINGS, vec![e]),
    }
}

/// The merge over entries already parsed: the table a load builds, leaked.
fn merge_entries(list: &[Value]) -> (&'static [KeyBinding], Vec<KeyLoadError>) {
    if list.is_empty() {
        return (KEY_BINDINGS, Vec::new());
    }
    let check = check_overrides(list);
    let errors = check.errors();
    if check
        .entries
        .iter()
        .all(|e| e.error.is_some() || e.row.is_none())
    {
        return (KEY_BINDINGS, errors);
    }
    let rows: Vec<KeyBinding> = KEY_BINDINGS
        .iter()
        .zip(check.keys)
        .map(|(b, keys)| {
            let mut b = *b;
            if b.keys.iter().copied().ne(keys.iter().map(String::as_str)) {
                b.keys = leak_keys(keys);
            }
            b
        })
        .collect();
    (Box::leak(rows.into_boxed_slice()), errors)
}

/// One entry of the file as the checks judged it.
#[derive(Debug, Clone)]
pub struct CheckedEntry {
    /// The row of `KEY_BINDINGS` the entry names: set once it is well
    /// formed and its namespace, action and context resolve, whatever its
    /// keys say.
    pub row: Option<usize>,
    /// Why the entry is refused; `None` when it is accepted.
    pub error: Option<KeyLoadError>,
}

/// What the loader's checks make of a list of entries, before anything is
/// leaked (`LoadKeyTable`; `WritersShareTheLoadersChecks`).
#[derive(Debug, Clone)]
pub struct OverrideCheck {
    /// One per entry, in file order.
    pub entries: Vec<CheckedEntry>,
    /// Every row's keys with the accepted entries applied, in
    /// `KEY_BINDINGS` order.
    pub keys: Vec<Vec<String>>,
}

impl OverrideCheck {
    /// The refused entries' errors, in file order.
    pub fn errors(&self) -> Vec<KeyLoadError> {
        self.entries
            .iter()
            .filter_map(|e| e.error.clone())
            .collect()
    }
}

/// Run every check `LoadKeyTable` runs over `list`: each entry on its own,
/// then duplicates, then clashes over the whole table.
pub fn check_overrides(list: &[Value]) -> OverrideCheck {
    let mut entries: Vec<Entry> = list.iter().map(judge_entry).collect();
    refuse_duplicates(&mut entries);
    refuse_clashes(&mut entries);
    let keys = row_keys(&entries)
        .into_iter()
        .map(|ks| ks.into_iter().map(str::to_string).collect())
        .collect();
    let entries = entries
        .into_iter()
        .map(|e| CheckedEntry {
            row: e.row,
            error: e
                .problem
                .map(|(problem, message, popup_message)| KeyLoadError {
                    problem,
                    namespace: e.namespace,
                    action: e.action,
                    context: e.context,
                    message,
                    popup_message,
                }),
        })
        .collect();
    OverrideCheck { entries, keys }
}

const ENTRY_SHAPE: &str = "an entry is {\"namespace\": \"board.normal\", \"action\": \"navigate_row_last\", \"keys\": [\"O\"]}, with an optional \"context\"";

/// The longest piece of the file a message repeats.
const ECHO_CHARS: usize = 40;

/// Text from the file as a message repeats it: cut past [`ECHO_CHARS`] so a
/// long entry cannot crowd the popup.
fn echo(text: &str) -> String {
    crate::text::truncate(text, ECHO_CHARS + 1)
}

/// A row as an error names it: namespace, action and any context literal.
fn row_label(b: &KeyBinding) -> String {
    match b.context {
        Some(c) => format!(
            "{} {} (context {})",
            b.namespace.name(),
            b.action,
            c.literal()
        ),
        None => format!("{} {}", b.namespace.name(), b.action),
    }
}

fn leak_keys(keys: Vec<String>) -> &'static [&'static str] {
    let keys: Vec<&'static str> = keys
        .into_iter()
        .map(|k| &*Box::leak(k.into_boxed_str()))
        .collect();
    Box::leak(keys.into_boxed_slice())
}

/// The per-entry checks, in the spec's order; the first problem found is the
/// entry's.
fn judge_entry(value: &Value) -> Entry {
    let field = |name: &str| value.get(name).and_then(Value::as_str).map(str::to_string);
    let mut entry = Entry {
        namespace: field("namespace"),
        action: field("action"),
        context: field("context"),
        row: None,
        keys: Vec::new(),
        problem: None,
    };
    let Some(obj) = value.as_object() else {
        entry.refuse(
            KeyOverrideProblem::MalformedEntry,
            format!(
                "{} is not an object; {ENTRY_SHAPE}",
                echo(&value.to_string())
            ),
        );
        return entry;
    };
    if let Some(other) = obj
        .keys()
        .find(|k| !matches!(k.as_str(), "namespace" | "action" | "context" | "keys"))
    {
        entry.refuse(
            KeyOverrideProblem::MalformedEntry,
            format!("an entry has no field \"{}\"; {ENTRY_SHAPE}", echo(other)),
        );
        return entry;
    }
    let keys: Option<Vec<String>> = obj.get("keys").and_then(Value::as_array).and_then(|list| {
        list.iter()
            .map(|k| k.as_str().map(str::to_string))
            .collect()
    });
    let missing = ["namespace", "action"]
        .into_iter()
        .find(|f| obj.get(*f).and_then(Value::as_str).is_none());
    let bad_context = obj.get("context").is_some_and(|c| !c.is_string());
    let (Some(keys), None, false) = (keys, missing, bad_context) else {
        let what = match (missing, bad_context) {
            (Some(f), _) => format!("\"{f}\" is missing or not a string"),
            (None, true) => "\"context\" is not a string".to_string(),
            (None, false) => "\"keys\" is missing or not a list of strings".to_string(),
        };
        entry.refuse(
            KeyOverrideProblem::MalformedEntry,
            format!("{what}; {ENTRY_SHAPE}"),
        );
        return entry;
    };
    let ns_name = entry.namespace.clone().unwrap_or_default();
    let action = entry.action.clone().unwrap_or_default();
    let row = match resolve_row(&ns_name, &action, entry.context.as_deref()) {
        Ok(row) => row,
        Err(problem) => {
            if entry.problem.is_none() {
                entry.problem = Some(problem);
            }
            return entry;
        }
    };
    entry.row = Some(row);
    let default = &KEY_BINDINGS[row];
    let ns = default.namespace;

    if default.keys.contains(&ANY_OTHER_KEY) {
        entry.refuse(
            KeyOverrideProblem::CatchAllRow,
            format!(
                "{} answers any key no other row lists and cannot be rebound; rebind the rows beside it instead",
                row_label(default)
            ),
        );
        return entry;
    }
    if keys.iter().any(|k| k == ANY_OTHER_KEY) {
        entry.refuse(
            KeyOverrideProblem::CatchAllRow,
            format!(
                "\"{ANY_OTHER_KEY}\" is not a key a row can be given ({}); list the keys themselves",
                row_label(default)
            ),
        );
        return entry;
    }
    if keys.is_empty() {
        entry.refuse(
            KeyOverrideProblem::EmptyKeys,
            format!(
                "\"keys\" is empty for {}; a row keeps at least one key. To restore its default keys, remove the entry",
                row_label(default)
            ),
        );
        return entry;
    }
    let mut unique: Vec<String> = Vec::new();
    for k in keys {
        if !unique.contains(&k) {
            unique.push(k);
        }
    }
    if let Some(bad) = unique.iter().find(|k| !is_key_name(k)) {
        let instead = match suggest_key_name(bad) {
            Some(good) => format!("write \"{good}\""),
            None if ns == KeyNamespace::TmuxGlobal => "a tmux.global key is \"Prefix+\" and one key: a character, a named key or a Ctrl/Alt key (\"Prefix+e\", \"Prefix+Space\")".to_string(),
            None => "a key is one character, a named key (Space, Enter, Esc, Tab, Up, F1, …), Ctrl+ or Alt+ before one (\"Ctrl+D\"), or a two-character chord (\"gg\")".to_string(),
        };
        entry.refuse(
            KeyOverrideProblem::UnknownKeyName,
            format!(
                "\"{}\" is not a key name ({}); {instead}",
                echo(bad),
                row_label(default)
            ),
        );
        return entry;
    }
    if ns == KeyNamespace::TmuxGlobal && unique.len() > 1 {
        entry.refuse(
            KeyOverrideProblem::KeyNotAllowedInNamespace,
            format!(
                "{} is one tmux binding and takes exactly one key; list one",
                row_label(default)
            ),
        );
        return entry;
    }
    if let Some(why) = unique.iter().find_map(|k| key_not_allowed_in(ns, k)) {
        entry.refuse(
            KeyOverrideProblem::KeyNotAllowedInNamespace,
            format!("{why} ({})", row_label(default)),
        );
        return entry;
    }
    entry.keys = unique;
    entry
}

/// A refusal before it is attached to an entry: the problem, its message and
/// the popup's shorter form of it, if any.
type Refusal = (KeyOverrideProblem, String, Option<String>);

/// The row of `KEY_BINDINGS` that a namespace, action and optional context
/// literal name, as an entry names one: with no context, the action's
/// context-free row when there is one, else its only row.
fn resolve_row(ns_name: &str, action: &str, context: Option<&str>) -> Result<usize, Refusal> {
    let Some(ns) = KeyNamespace::parse(ns_name) else {
        let valid: Vec<String> = KeyNamespace::ALL
            .iter()
            .map(|n| n.name().to_string())
            .collect();
        let (whole, short) = listing(
            &format!("\"{}\" is not a namespace; write one of: ", echo(ns_name)),
            &valid,
        );
        return Err((KeyOverrideProblem::UnknownNamespace, whole, short));
    };
    let rows: Vec<usize> = (0..KEY_BINDINGS.len())
        .filter(|&i| KEY_BINDINGS[i].namespace == ns && KEY_BINDINGS[i].action == action)
        .collect();
    if rows.is_empty() {
        let mut actions: Vec<String> = Vec::new();
        for b in KEY_BINDINGS.iter().filter(|b| b.namespace == ns) {
            if !actions.iter().any(|a| a == b.action) {
                actions.push(b.action.to_string());
            }
        }
        let (whole, short) = listing(
            &format!(
                "{ns_name} has no action \"{}\"; its actions are: ",
                echo(action)
            ),
            &actions,
        );
        return Err((KeyOverrideProblem::UnknownAction, whole, short));
    }
    let contexts_of_rows = || {
        rows.iter()
            .map(|&i| {
                KEY_BINDINGS[i]
                    .context
                    .map_or("no context".to_string(), |c| format!("\"{}\"", c.literal()))
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    match context {
        Some(lit) => KeyContext::parse(lit)
            .and_then(|c| {
                rows.iter()
                    .copied()
                    .find(|&i| KEY_BINDINGS[i].context == Some(c))
            })
            .ok_or_else(|| {
                (
                    KeyOverrideProblem::UnknownContext,
                    format!(
                        "{ns_name} {action} has no row with context \"{}\"; its rows have: {}",
                        echo(lit),
                        contexts_of_rows()
                    ),
                    None,
                )
            }),
        None => {
            let free = rows
                .iter()
                .copied()
                .find(|&i| KEY_BINDINGS[i].context.is_none());
            match (free, rows.as_slice()) {
                (Some(row), _) | (None, &[row]) => Ok(row),
                (None, _) => Err((
                    KeyOverrideProblem::AmbiguousRow,
                    format!(
                        "{ns_name} {action} has several rows; add \"context\" with one of: {}",
                        contexts_of_rows()
                    ),
                    None,
                )),
            }
        }
    }
}

/// Every entry that names a row another entry names is refused, all of them.
fn refuse_duplicates(entries: &mut [Entry]) {
    let mut by_row: HashMap<usize, usize> = HashMap::new();
    for e in entries.iter() {
        if let Some(row) = e.row {
            *by_row.entry(row).or_default() += 1;
        }
    }
    for e in entries.iter_mut() {
        if let Some(row) = e.row {
            let count = by_row.get(&row).copied().unwrap_or_default();
            if count > 1 {
                e.refuse(
                    KeyOverrideProblem::DuplicateEntry,
                    format!(
                        "{} is named by {count} entries; keep one",
                        row_label(&KEY_BINDINGS[row])
                    ),
                );
            }
        }
    }
}

/// Every row's keys, in `KEY_BINDINGS` order, with every accepted entry
/// applied.
fn row_keys(entries: &[Entry]) -> Vec<Vec<&str>> {
    let mut keys: Vec<Vec<&str>> = KEY_BINDINGS.iter().map(|b| b.keys.to_vec()).collect();
    for e in entries.iter().filter(|e| e.problem.is_none()) {
        if let Some(row) = e.row {
            keys[row] = e.keys.iter().map(String::as_str).collect();
        }
    }
    keys
}

/// Refuse every accepted entry that takes part in a clash, round after round,
/// until the table has none.
fn refuse_clashes(entries: &mut [Entry]) {
    loop {
        if entries
            .iter()
            .all(|e| e.problem.is_some() || e.row.is_none())
        {
            return;
        }
        let clashes: Vec<(Clash, KeyOverrideProblem, String)> = {
            let keys = row_keys(entries);
            let rows: Vec<RowKeys<'_>> = KEY_BINDINGS
                .iter()
                .zip(keys)
                .map(|(b, keys)| RowKeys {
                    namespace: b.namespace,
                    context: b.context,
                    keys,
                })
                .collect();
            clashes_in(&rows)
                .into_iter()
                .map(|c| {
                    let (problem, message) = c.describe();
                    (c, problem, message)
                })
                .collect()
        };
        if clashes.is_empty() {
            return;
        }
        let mut refused_any = false;
        for (clash, problem, message) in &clashes {
            for row in [clash.a, clash.b] {
                if let Some(e) = entries
                    .iter_mut()
                    .find(|e| e.problem.is_none() && e.row == Some(row))
                {
                    e.refuse(*problem, message.clone());
                    refused_any = true;
                }
            }
        }
        if !refused_any {
            // A clash no entry takes part in is in the default table, which
            // the drift gate rules out. Stop rather than loop.
            return;
        }
    }
}

/// Two rows of one namespace that a key brings into conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clash {
    /// For a chord clash, the row listing the chord.
    pub a: usize,
    /// For a chord clash, the row listing the chord's first character.
    pub b: usize,
    /// The shared key, or the chord.
    pub key: String,
    pub chord: bool,
}

impl Clash {
    /// The refusal for a clash between two rows of `KEY_BINDINGS`.
    fn describe(&self) -> (KeyOverrideProblem, String) {
        let (a, b) = (
            row_label(&KEY_BINDINGS[self.a]),
            row_label(&KEY_BINDINGS[self.b]),
        );
        if self.chord {
            let first: String = self.key.chars().take(1).collect();
            (
                KeyOverrideProblem::ChordPrefixClash,
                format!(
                    "the chord \"{}\" of {a} starts with \"{first}\", a key of {b}; a chord's first character cannot also be a key in the namespace. Pick another chord or another key",
                    self.key
                ),
            )
        } else {
            (
                KeyOverrideProblem::KeyClash,
                format!(
                    "\"{}\" would be a key of both {a} and {b}, which can apply at once; pick a key the other row does not list",
                    self.key
                ),
            )
        }
    }
}

/// What the clash check reads of a row.
struct RowKeys<'a> {
    namespace: KeyNamespace,
    context: Option<KeyContext>,
    keys: Vec<&'a str>,
}

/// Every clash in `rows`: two rows of a namespace that can apply at once
/// sharing a key (`KeyUniquePerContext`, `ContextsDoNotOverlapForAKey`), and
/// a chord whose first character is a key of the namespace
/// (`ChordPrefixIsFree`). Empty for a well-formed table. Row numbers are
/// indices into `rows`.
#[cfg(test)]
pub fn table_clashes(rows: &[KeyBinding]) -> Vec<Clash> {
    let rows: Vec<RowKeys<'_>> = rows
        .iter()
        .map(|b| RowKeys {
            namespace: b.namespace,
            context: b.context,
            keys: b.keys.to_vec(),
        })
        .collect();
    clashes_in(&rows)
}

fn clashes_in(rows: &[RowKeys<'_>]) -> Vec<Clash> {
    let mut clashes = Vec::new();
    for (i, a) in rows.iter().enumerate() {
        for (j, b) in rows.iter().enumerate() {
            if a.namespace != b.namespace {
                continue;
            }
            if i < j {
                let exclusive =
                    matches!((a.context, b.context), (Some(x), Some(y)) if !contexts_overlap(x, y));
                if !exclusive {
                    if let Some(key) = a.keys.iter().find(|k| b.keys.contains(k)) {
                        clashes.push(Clash {
                            a: i,
                            b: j,
                            key: (*key).to_string(),
                            chord: false,
                        });
                    }
                }
            }
            for chord in a.keys.iter().filter(|k| is_chord(k)) {
                let first: String = chord.chars().take(1).collect();
                if b.keys.contains(&first.as_str()) {
                    clashes.push(Clash {
                        a: i,
                        b: j,
                        key: (*chord).to_string(),
                        chord: true,
                    });
                }
            }
        }
    }
    clashes
}

mod set;
pub use set::{set_key_overrides, KeyChange, KeySetError, KeySetOutcome};

#[cfg(test)]
mod tests;
