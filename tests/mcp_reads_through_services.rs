//! Guard: MCP handlers read tasks and epics through `task_svc` / `epic_svc`,
//! never `state.db.get_task` / `state.db.get_epic`. Two read paths let the
//! service-level behaviour (not-found mapping, error shape) drift apart.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::PathBuf;

#[test]
fn mcp_handlers_do_not_read_tasks_or_epics_from_state_db() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/mcp");
    let files = common::rust_files(&root, true);
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
