//! No public reducer inserts a client-built task without the aim check and the
//! identity stamp `create_task` runs (`tasks.allium: PhoenixRespawn`,
//! `workspaces.allium`: "Identities are the caller's").
//!
//! A source scan: a `ReducerContext` cannot be built inside this crate, and the
//! rule is about every reducer that will ever exist, so the test reads the
//! module's own sources.

use std::fs;
use std::path::Path;

/// Every `#[spacetimedb::reducer]` function in `dir`, as `(name, text)`.
fn reducers_in(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).expect("read module src") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "tests") {
                continue;
            }
            reducers_in(&path, out);
            continue;
        }
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("read source");
        out.extend(reducer_chunks(&text));
    }
}

fn reducer_chunks(text: &str) -> Vec<(String, String)> {
    let mut chunks = Vec::new();
    for part in text.split("#[spacetimedb::reducer").skip(1) {
        let body = part.split("\n}\n").next().unwrap_or(part);
        let Some(after_fn) = body.split("fn ").nth(1) else {
            continue;
        };
        let name = after_fn
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .next()
            .unwrap_or("")
            .to_string();
        chunks.push((name, body.to_string()));
    }
    chunks
}

/// A reducer that takes a whole `Task` row from the caller and writes a task.
fn inserts_a_client_built_task(body: &str) -> bool {
    let signature = body.split(") ->").next().unwrap_or(body);
    let takes_task_row =
        signature.contains(": Task,") || signature.trim_end().trim_end_matches(',').ends_with(": Task");
    takes_task_row && (body.contains("write_task(") || body.contains("store_task("))
}

/// The module's `src/`. This file is compiled by the module's own crate and,
/// through a path dependency, by `spacetime/module-tests`, whose manifest
/// directory is not the module's; `file!()` is relative to the one that built it.
fn module_src() -> std::path::PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let own = manifest.join("src");
    if own.join("tasks_epics.rs").exists() {
        return own;
    }
    let beside = manifest.join(file!());
    beside
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .filter(|p| p.join("tasks_epics.rs").exists())
        .expect("find the module's src directory")
}

fn client_task_reducers() -> Vec<(String, String)> {
    let mut all = Vec::new();
    reducers_in(&module_src(), &mut all);
    all.into_iter()
        .filter(|(_, body)| inserts_a_client_built_task(body))
        .collect()
}

#[test]
fn the_scan_finds_the_create_reducers() {
    let names: Vec<String> = client_task_reducers().into_iter().map(|(n, _)| n).collect();
    assert!(names.contains(&"create_task".to_string()), "{names:?}");
    assert!(
        names.contains(&"create_task_signalled".to_string()),
        "{names:?}"
    );
}

#[test]
fn every_reducer_inserting_a_client_task_runs_the_aim_check_and_the_stamp() {
    for (name, body) in client_task_reducers() {
        assert!(
            body.contains("check_new_task_aim("),
            "reducer `{name}` inserts a client-built task without check_new_task_aim"
        );
        assert!(
            body.contains("stamp_new_task("),
            "reducer `{name}` inserts a client-built task without stamp_new_task"
        );
    }
}

#[test]
fn the_scan_flags_a_reducer_missing_the_checks() {
    let bad = "(ctx: &ReducerContext, successor: Task) -> Result<(), String> {\n    write_task(ctx, successor)?;\n    Ok(())";
    assert!(inserts_a_client_built_task(bad));
    assert!(!bad.contains("check_new_task_aim("));
}
