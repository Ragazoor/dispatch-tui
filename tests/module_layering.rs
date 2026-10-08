//! Guard: lower layers never reach up or sideways into higher ones.
//!
//! Each rule names a source directory and the `crate::` modules it must not
//! import. `feed` sits below `runtime` and `mcp`; `mcp` below `cli`; the
//! storage and identity layers below `startup`; `sync` and `dispatch` below
//! `service`; `cli` below `runtime`, sharing the store wiring
//! (`store_connection`) with it instead. `agent_tree` is a feature of its own:
//! `cli` is its entry point, so it never reaches back up into `cli`, nor across
//! into the board's `tui` (both draw with the shared `palette`). A shared type both sides need belongs in a leaf module
//! (`models`, `clock`, `embeddings`, …) that each depends on downward.
//! Test files are scanned too: a test reaching upward pins the edge just as
//! firmly as production code does.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::PathBuf;

/// `(module under src/, a directory or one file; modules it must not import)`.
const RULES: &[(&str, &[&str])] = &[
    ("feed", &["runtime", "mcp"]),
    ("mcp", &["cli"]),
    ("host_file", &["startup"]),
    ("spacetime", &["startup"]),
    ("sync", &["startup", "service"]),
    ("store", &["startup"]),
    ("service", &["startup"]),
    ("dispatch", &["service"]),
    ("cli", &["runtime", "tui"]),
    (
        "agent_tree",
        &[
            "runtime",
            "tui",
            "cli",
            "mcp",
            "dispatch",
            "service",
            "store",
            "sync",
            "startup",
            "spacetime",
        ],
    ),
    ("palette", &["tui", "agent_tree"]),
    ("store_connection", &["runtime", "cli"]),
];

/// Every `crate::<module>` path `source` names, including each member of a
/// grouped `use crate::{a, b::{c, d}}` import, nested or spread over several
/// lines as rustfmt writes a long one. Comment lines are skipped.
fn crate_modules(source: &str) -> Vec<String> {
    let code: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut found = Vec::new();
    for (i, _) in code.match_indices("crate::") {
        let rest = &code[i + "crate::".len()..];
        match rest.strip_prefix('{') {
            Some(group) => found.extend(group_heads(group)),
            None => found.push(ident(rest)),
        }
    }
    found
}

/// The first segment of each top-level member of a `{...}` group, given the
/// text just after its opening brace.
fn group_heads(group: &str) -> Vec<String> {
    let mut heads = Vec::new();
    let mut depth = 0;
    let mut at_member_start = true;
    for (i, c) in group.char_indices() {
        match c {
            '{' => depth += 1,
            '}' if depth == 0 => break,
            '}' => depth -= 1,
            ',' if depth == 0 => at_member_start = true,
            c if at_member_start && !c.is_whitespace() => {
                at_member_start = false;
                heads.push(ident(&group[i..]));
            }
            _ => {}
        }
    }
    heads
}

fn ident(text: &str) -> String {
    text.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

#[test]
fn crate_modules_reads_plain_and_grouped_paths() {
    assert_eq!(crate_modules("use crate::runtime::x;"), vec!["runtime"]);
    assert_eq!(
        crate_modules("use crate::{models::Task, mcp::BoardEvent};"),
        vec!["models", "mcp"]
    );
    assert_eq!(
        crate_modules("    &crate::cli::pane_key_event(a, k),"),
        vec!["cli"]
    );
}

#[test]
fn crate_modules_reads_nested_and_multi_line_groups() {
    assert_eq!(
        crate_modules("use crate::{models::{A, B}, runtime::X};"),
        vec!["models", "runtime"]
    );
    assert_eq!(
        crate_modules("use crate::{\n    models::Task,\n    runtime::poll,\n};"),
        vec!["models", "runtime"]
    );
    assert!(crate_modules("// use crate::runtime::x;").is_empty());
}

#[test]
fn lower_layers_do_not_import_higher_ones() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for (dir, forbidden) in RULES {
        // A module is a directory, or a single `<name>.rs` file.
        let module_dir = src.join(dir);
        let files = if module_dir.is_dir() {
            common::rust_files(&module_dir, false)
        } else {
            vec![src.join(format!("{dir}.rs"))]
        };
        for file in files {
            let body = std::fs::read_to_string(&file).unwrap();
            for module in crate_modules(&body) {
                if forbidden.contains(&module.as_str()) {
                    offenders.push(format!(
                        "{}: {dir} -> {module}",
                        file.strip_prefix(&src).unwrap().display()
                    ));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "lower layers import higher ones:\n{}",
        offenders.join("\n")
    );
}
