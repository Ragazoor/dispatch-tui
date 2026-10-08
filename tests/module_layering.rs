//! Guard: lower layers never reach up or sideways into higher ones.
//!
//! Each rule names a source directory and the `crate::` modules it must not
//! import. `feed` sits below `runtime` and `mcp`; `mcp` below `cli`; the
//! storage and identity layers below `startup`; `sync` and `dispatch` below
//! `service`. A shared type both sides need belongs in a leaf module
//! (`models`, `clock`, `embeddings`, …) that each depends on downward.
//! Test files are scanned too: a test reaching upward pins the edge just as
//! firmly as production code does.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `(directory under src/, modules it must not import)`.
const RULES: &[(&str, &[&str])] = &[
    ("feed", &["runtime", "mcp"]),
    ("mcp", &["cli"]),
    ("host_file", &["startup"]),
    ("spacetime", &["startup"]),
    ("sync", &["startup", "service"]),
    ("store", &["startup"]),
    ("service", &["startup"]),
    ("dispatch", &["service"]),
];

/// Every `crate::<module>` path a line names, including the members of a
/// grouped `use crate::{a, b::c}` import.
fn crate_modules(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (i, _) in line.match_indices("crate::") {
        let rest = &line[i + "crate::".len()..];
        if let Some(group) = rest.strip_prefix('{') {
            let group = group.split('}').next().unwrap_or("");
            for item in group.split(',') {
                let head = item.trim().split("::").next().unwrap_or("").trim();
                if !head.is_empty() {
                    found.push(head.to_string());
                }
            }
        } else {
            let head: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            found.push(head);
        }
    }
    found
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
fn lower_layers_do_not_import_higher_ones() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for (dir, forbidden) in RULES {
        let mut files = Vec::new();
        rust_files(&src.join(dir), &mut files);
        for file in files {
            let body = std::fs::read_to_string(&file).unwrap();
            for (n, line) in body.lines().enumerate() {
                let code = line.trim_start();
                if code.starts_with("//") {
                    continue;
                }
                for module in crate_modules(line) {
                    if forbidden.contains(&module.as_str()) {
                        offenders.push(format!(
                            "{}:{}: {dir} -> {module}: {}",
                            file.strip_prefix(&src).unwrap().display(),
                            n + 1,
                            code
                        ));
                    }
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
