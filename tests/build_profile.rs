//! Pins `DevBuildsKeepLineTablesNotFullDebugInfo` (docs/specs/dispatch.allium).
//!
//! `cargo test` links ~30 binaries; full DWARF in each made a one-file rebuild
//! ~20s slower (#16776). Nothing else notices if the setting is dropped.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

#[test]
fn dev_profile_keeps_line_tables_only() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let body = std::fs::read_to_string(&path).unwrap();
    let dev = body
        .split("[profile.dev]")
        .nth(1)
        .expect("Cargo.toml has a [profile.dev] section")
        .split("\n[")
        .next()
        .unwrap();
    assert!(
        dev.lines()
            .any(|l| l.trim().replace(' ', "") == "debug=\"line-tables-only\""),
        "[profile.dev] must set debug = \"line-tables-only\"; got:\n{dev}"
    );
}
