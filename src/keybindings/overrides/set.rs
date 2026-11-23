//! Writing the override file: `keybinding_set` (`SetKeyOverridesViaMcp` in
//! `docs/specs/keybindings.allium`). A call's changes are checked together,
//! by the loader's own checks ([`check_overrides`]) over the default table,
//! the file's entries and the changes; one that would add a problem writes
//! nothing (`ARefusedChangeWritesNothing`, `WritersShareTheLoadersChecks`).

use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::{
    check_overrides, read_override_entries, resolve_row, row_label, KeyLoadError, OverrideCheck,
    KEY_BINDINGS,
};

/// One change of a call: the row, named as an entry names it, and its keys.
/// An empty key list removes the row's entry, which restores its default
/// keys.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyChange {
    pub namespace: String,
    pub action: String,
    #[serde(default)]
    pub context: Option<String>,
    pub keys: Vec<String>,
}

/// A row a call changed, as the answer lists it.
#[derive(Debug, Clone)]
pub struct ChangedRow {
    /// The row, as an error names it: namespace, action and any context.
    pub label: String,
    /// The keys the row has once the board next starts.
    pub keys: Vec<String>,
    /// The change removed the row's entry, so these are its default keys.
    pub restored: bool,
}

/// What an accepted call did.
#[derive(Debug, Clone)]
pub struct KeySetOutcome {
    /// The rows of the call, in the call's order.
    pub rows: Vec<ChangedRow>,
    /// False when the file already said what the call asks, so nothing was
    /// written.
    pub written: bool,
    /// Entries already in the file that the load refuses. Left as they are.
    pub kept_errors: Vec<KeyLoadError>,
}

impl KeyChange {
    /// The change that gives row `row` of `KEY_BINDINGS` the keys `keys`,
    /// naming the row as an entry of the file does.
    pub fn for_row(row: usize, keys: Vec<String>) -> Self {
        let b = &KEY_BINDINGS[row];
        Self {
            namespace: b.namespace.name().to_string(),
            action: b.action.to_string(),
            context: b.context.map(|c| c.literal().to_string()),
            keys,
        }
    }
}

/// A problem a call's changes would add: its message, and the row of
/// `KEY_BINDINGS` it is about when the change named one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeProblem {
    pub row: Option<usize>,
    pub message: String,
}

/// What a call's changes make of the file's entries, checked and not yet
/// written.
#[derive(Debug, Clone)]
pub struct KeyPlan {
    /// The entries the file holds once the changes are applied.
    pub list: Vec<Value>,
    /// The row each change names, in the call's order.
    pub rows: Vec<usize>,
    /// The loader's checks over `list`.
    pub after: OverrideCheck,
}

/// Why a call wrote nothing.
#[derive(Debug, Clone)]
pub enum KeySetError {
    /// The call would add a problem: one message per problem, each saying
    /// what to fix. The file is left exactly as it was.
    Refused(Vec<String>),
    /// The checks passed but the file could not be written.
    Write(String),
}

/// The message for a file the loader cannot read, as a writer refuses it.
pub fn unreadable_file_refusal(e: &KeyLoadError) -> String {
    format!(
        "{} The file was not changed; fix it or remove it, then set the keys again.",
        e.message
    )
}

/// Apply `changes` to the override file at `file`, all or nothing.
pub fn set_key_overrides(file: &Path, changes: &[KeyChange]) -> Result<KeySetOutcome, KeySetError> {
    use KeySetError::{Refused, Write};
    let entries =
        read_override_entries(file).map_err(|e| Refused(vec![unreadable_file_refusal(&e)]))?;
    let KeyPlan { list, rows, after } =
        plan_key_overrides(&entries, changes).map_err(|problems| {
            let mut messages: Vec<String> = problems.into_iter().map(|p| p.message).collect();
            messages.dedup();
            Refused(messages)
        })?;

    let written = list != entries;
    if written {
        let mut text = serde_json::to_string_pretty(&Value::Array(list))
            .map_err(|e| Write(format!("the override file could not be written ({e})")))?;
        text.push('\n');
        crate::atomic_write::write_bytes(
            file,
            crate::atomic_write::Mode::KeepOr { default: 0o644 },
            text.as_bytes(),
        )
        .map_err(|e| Write(format!("{} could not be written: {e:#}", file.display())))?;
    }

    let rows = rows
        .iter()
        .zip(changes)
        .map(|(&row, c)| ChangedRow {
            label: row_label(&KEY_BINDINGS[row]),
            keys: after.keys[row].clone(),
            restored: c.keys.is_empty(),
        })
        .collect();
    let kept_errors = after.errors();
    Ok(KeySetOutcome {
        rows,
        written,
        kept_errors,
    })
}

/// Apply `changes` to the file's `entries` and run the loader's checks over
/// the result, writing nothing. Refused with every problem the changes would
/// add; an entry the load already refuses is not one of them.
pub fn plan_key_overrides(
    entries: &[Value],
    changes: &[KeyChange],
) -> Result<KeyPlan, Vec<ChangeProblem>> {
    let problem = |row: Option<usize>, message: String| ChangeProblem { row, message };
    if changes.is_empty() {
        return Err(vec![problem(
            None,
            "overrides is empty; pass at least one change".to_string(),
        )]);
    }

    // Each change must name a row, whatever its keys.
    let mut rows: Vec<usize> = Vec::with_capacity(changes.len());
    let mut problems: Vec<ChangeProblem> = Vec::new();
    for c in changes {
        match resolve_row(&c.namespace, &c.action, c.context.as_deref()) {
            Ok(row) if rows.contains(&row) => problems.push(problem(
                Some(row),
                format!(
                    "{} is named by more than one change in this call; keep one",
                    row_label(&KEY_BINDINGS[row])
                ),
            )),
            Ok(row) => rows.push(row),
            Err((_, message, _)) => problems.push(problem(None, message)),
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }

    let before = check_overrides(entries);

    // The new list: each file entry the call names is replaced in place by
    // its change (or dropped, for a removal or a second entry for the row);
    // a change with no entry yet is appended. `origin` maps each new entry
    // to the file entry it keeps, or `None` for one the call wrote.
    let mut list: Vec<Value> = Vec::new();
    let mut origin: Vec<Option<usize>> = Vec::new();
    let mut placed = vec![false; changes.len()];
    for (i, (value, checked)) in entries.iter().zip(&before.entries).enumerate() {
        let Some(c) = checked
            .row
            .and_then(|row| rows.iter().position(|&r| r == row))
        else {
            list.push(value.clone());
            origin.push(Some(i));
            continue;
        };
        if !placed[c] {
            placed[c] = true;
            if !changes[c].keys.is_empty() {
                list.push(entry_value(&changes[c]));
                origin.push(None);
            }
        }
    }
    for (c, change) in changes.iter().enumerate() {
        if !placed[c] && !change.keys.is_empty() {
            list.push(entry_value(change));
            origin.push(None);
        }
    }

    // A problem on an entry the call wrote, or on a file entry the load
    // accepted before, is one the call adds.
    let after = check_overrides(&list);
    let added: Vec<ChangeProblem> = after
        .entries
        .iter()
        .zip(&origin)
        .filter_map(|(checked, from)| {
            let error = checked.error.as_ref()?;
            let was_accepted = from.is_some_and(|i| before.entries[i].error.is_none());
            (from.is_none() || was_accepted).then(|| problem(checked.row, error.message.clone()))
        })
        .collect();
    if !added.is_empty() {
        return Err(added);
    }
    Ok(KeyPlan { list, rows, after })
}

/// A change as an entry of the file: the names the call gave, and its keys.
fn entry_value(c: &KeyChange) -> Value {
    let mut entry = Map::new();
    entry.insert("namespace".into(), json!(c.namespace));
    entry.insert("action".into(), json!(c.action));
    if let Some(context) = &c.context {
        entry.insert("context".into(), json!(context));
    }
    entry.insert("keys".into(), json!(c.keys));
    Value::Object(entry)
}
