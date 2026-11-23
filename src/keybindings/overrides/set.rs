//! Writing the override file: `keybinding_set` (`SetKeyOverridesViaMcp` in
//! `docs/specs/keybindings.allium`). A call's changes are checked together,
//! by the loader's own checks ([`check_overrides`]) over the default table,
//! the file's entries and the changes; one that would add a problem writes
//! nothing (`ARefusedChangeWritesNothing`, `WritersShareTheLoadersChecks`).

use std::path::Path;

use serde_json::{json, Map, Value};

use super::{
    check_overrides, read_override_entries, resolve_row, row_label, KeyLoadError, KEY_BINDINGS,
};

/// One change of a call: the row, named as an entry names it, and its keys.
/// An empty key list removes the row's entry, which restores its default
/// keys.
#[derive(Debug, Clone)]
pub struct KeyChange {
    pub namespace: String,
    pub action: String,
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

/// Apply `changes` to the override file at `file`, all or nothing. `Err`
/// holds one message per problem, each saying what to fix; the file is then
/// left exactly as it was.
pub fn set_key_overrides(file: &Path, changes: &[KeyChange]) -> Result<KeySetOutcome, Vec<String>> {
    if changes.is_empty() {
        return Err(vec![
            "overrides is empty; pass at least one change".to_string()
        ]);
    }
    let entries = read_override_entries(file).map_err(|e| {
        vec![format!(
            "{} The file was not changed; fix it or remove it, then set the keys again.",
            e.message
        )]
    })?;

    // Each change must name a row, whatever its keys.
    let mut rows: Vec<usize> = Vec::with_capacity(changes.len());
    let mut problems: Vec<String> = Vec::new();
    for c in changes {
        match resolve_row(&c.namespace, &c.action, c.context.as_deref()) {
            Ok(row) if rows.contains(&row) => problems.push(format!(
                "{} is named by more than one change in this call; keep one",
                row_label(&KEY_BINDINGS[row])
            )),
            Ok(row) => rows.push(row),
            Err((_, message, _)) => problems.push(message),
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }

    let before = check_overrides(&entries);

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
    let added: Vec<String> = after
        .entries
        .iter()
        .zip(&origin)
        .filter_map(|(checked, from)| {
            let error = checked.error.as_ref()?;
            let was_accepted = from.is_some_and(|i| before.entries[i].error.is_none());
            (from.is_none() || was_accepted).then(|| error.message.clone())
        })
        .collect();
    if !added.is_empty() {
        let mut added = added;
        added.dedup();
        return Err(added);
    }

    let written = list != entries;
    if written {
        let mut text = serde_json::to_string_pretty(&Value::Array(list))
            .map_err(|e| vec![format!("the override file could not be written ({e})")])?;
        text.push('\n');
        crate::atomic_write::write_bytes(
            file,
            crate::atomic_write::Mode::KeepOr { default: 0o644 },
            text.as_bytes(),
        )
        .map_err(|e| vec![format!("{} could not be written: {e:#}", file.display())])?;
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
