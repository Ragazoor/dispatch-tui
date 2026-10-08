//! Guard: MCP handlers read tasks and epics through `task_svc` / `epic_svc`,
//! never `state.db.get_task` / `state.db.get_epic`. Two read paths let the
//! service-level behaviour (not-found mapping, error shape) drift apart.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "tests") {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs")
            && !path.file_name().unwrap().to_string_lossy().contains("test")
        {
            out.push(path);
        }
    }
}

#[test]
fn mcp_handlers_do_not_read_tasks_or_epics_from_state_db() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/mcp");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    let mut offenders = Vec::new();
    for file in files {
        let body = std::fs::read_to_string(&file).unwrap();
        for (n, line) in body.lines().enumerate() {
            if line.contains("state.db.get_task(") || line.contains("state.db.get_epic(") {
                offenders.push(format!("{}:{}: {}", file.display(), n + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "direct reads:\n{}",
        offenders.join("\n")
    );
}
