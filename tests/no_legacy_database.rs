#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Task #16755: no production path opens SQLite, nothing touches a leftover
//! `tasks.db`, and the one-shot commands need a host file they never write.
//!
//! Spec: `storage.allium` (`StoreInUseNeverOpensSqlite`,
//! `CodeNeverTouchesLegacyDatabase`), `cli.allium`
//! (`CliCommandsNeedAHostFile`, `CliCommandsReachTheStoreWithoutManagingIt`,
//! `NoSubcommandOpensSqlite`, the removal of `spacetime dump` and
//! `spacetime seed`), `host.allium` (`IdentityLivesInHostFile`).
//!
//! Every command here is pointed at a store address nothing listens on, so
//! none can reach a real store on 127.0.0.1:3000.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use dispatch_tui::host_file::HostIdentity;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_dispatch"))
}

/// An `http://` address nothing listens on: claimed from the OS, then released.
fn dead_store_address() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}

fn plan_file() -> tempfile::NamedTempFile {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    writeln!(
        f,
        "# A plan \u{2014} Implementation Plan\n\n**Goal:** Goal."
    )
    .unwrap();
    f
}

/// Run `args` against `--data-dir <data_dir>` and a dead store.
fn run(data_dir: &Path, args: &[&str]) -> Output {
    binary()
        .env_remove("DISPATCH_SPACETIME_SERVER")
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "--spacetime-server",
            &dead_store_address(),
        ])
        .args(args)
        .output()
        .unwrap()
}

/// The four store-backed one-shot commands, as argv.
fn store_backed_commands(plan: &Path) -> Vec<Vec<String>> {
    [
        vec!["repo", "list"],
        vec!["repo", "set-verify", "/r", "true"],
        vec!["prune-repo-paths"],
        vec!["plan", "1", plan.to_str().unwrap()],
    ]
    .into_iter()
    .map(|a| a.into_iter().map(String::from).collect())
    .collect()
}

/// A pre-#16755 `tasks.db` left behind, with no host identity in it. Dispatch
/// has no SQLite to write one with any more, so the fixture is a file with the
/// SQLite magic header and some payload: all the guarantee needs is bytes whose
/// digest and mtime a stray open, truncate or rewrite would change.
async fn legacy_database(data_dir: &Path) {
    let mut bytes = b"SQLite format 3\0".to_vec();
    bytes.extend((0..4096u32).map(|n| (n % 251) as u8));
    std::fs::write(data_dir.join("tasks.db"), bytes).unwrap();
    let left = footprint(data_dir);
    assert!(
        !left.wal && !left.shm,
        "fixture: the legacy database must be self-contained"
    );
}

#[derive(Debug, PartialEq)]
struct LegacyFootprint {
    /// A digest rather than the bytes, so a failure prints one line.
    content: u64,
    modified: std::time::SystemTime,
    wal: bool,
    shm: bool,
}

fn footprint(data_dir: &Path) -> LegacyFootprint {
    let companion = |suffix: &str| {
        let db_file = data_dir.join("tasks.db");
        let mut name = db_file.as_os_str().to_owned();
        name.push(suffix);
        PathBuf::from(name).exists()
    };
    LegacyFootprint {
        content: {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            std::fs::read(data_dir.join("tasks.db"))
                .unwrap()
                .hash(&mut h);
            h.finish()
        },
        modified: std::fs::metadata(data_dir.join("tasks.db"))
            .unwrap()
            .modified()
            .unwrap(),
        wal: companion("-wal"),
        shm: companion("-shm"),
    }
}

// ---------------------------------------------------------------------------
// CliCommandsNeedAHostFile
// ---------------------------------------------------------------------------

/// With no host file, `repo`, `plan` and `prune-repo-paths` fail before they
/// reach for the store, say that one `dispatch tui` creates this machine's
/// identity, and create nothing — no host file, no database.
#[test]
fn one_shot_commands_without_a_host_file_fail_and_create_nothing() {
    let plan = plan_file();
    for args in store_backed_commands(plan.path()) {
        let dir = tempfile::tempdir().unwrap();
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();

        let out = run(dir.path(), &argv);

        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success(),
            "{args:?} must fail with no host file"
        );
        assert!(
            stderr.contains("dispatch tui"),
            "{args:?} must say that one `dispatch tui` creates the identity, got: {stderr}"
        );
        assert!(
            !stderr.contains("Could not connect"),
            "{args:?} must fail on the missing identity before trying the store, got: {stderr}"
        );
        assert!(
            !dir.path().join("host.json").exists(),
            "{args:?} must never mint an identity (host.allium: MintHostIdentity is the board's)"
        );
        assert!(
            !dir.path().join("tasks.db").exists(),
            "{args:?} must not create a database (StoreInUseNeverOpensSqlite)"
        );
    }
}

/// With a host file, the same commands get as far as the store (and fail
/// there, it being dead) without opening the populated `tasks.db` beside it:
/// its bytes and mtime are unchanged and no `-wal`/`-shm` is left. The host
/// file itself is not written either.
#[tokio::test]
async fn one_shot_commands_with_a_host_file_never_touch_the_legacy_database() {
    let plan = plan_file();
    for args in store_backed_commands(plan.path()) {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_path_buf();
        legacy_database(&data_dir).await;
        let host_file = dir.path().join("host.json");
        std::fs::write(
            &host_file,
            serde_json::to_vec(&HostIdentity {
                host_id: "host-from-file".to_string(),
                label: Some("laptop".to_string()),
                user_identity: Some("c0ffee".to_string()),
                credential: Some("token".to_string()),
            })
            .unwrap(),
        )
        .unwrap();
        let host_before = std::fs::read(&host_file).unwrap();
        let before = footprint(&data_dir);
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();

        let out = run(dir.path(), &argv);

        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{args:?}: the store is dead");
        assert!(
            stderr.contains("Could not connect to the shared store"),
            "{args:?} with a host file must get as far as the store, got: {stderr}"
        );
        assert_eq!(
            footprint(&data_dir),
            before,
            "{args:?} must not open, write or delete the leftover tasks.db"
        );
        assert_eq!(
            std::fs::read(&host_file).unwrap(),
            host_before,
            "{args:?}: a one-shot command never writes the host file"
        );
    }
}

// ---------------------------------------------------------------------------
// `spacetime dump` and `spacetime seed` are removed
// ---------------------------------------------------------------------------

#[test]
fn spacetime_dump_and_seed_are_unknown_subcommands() {
    for removed in ["dump", "seed"] {
        let dir = tempfile::tempdir().unwrap();

        let out = run(dir.path(), &["spacetime", removed]);

        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            out.status.code(),
            Some(2),
            "`spacetime {removed}` must end with usage_error, got stderr: {stderr}"
        );
        assert!(
            stderr.contains("unrecognized subcommand"),
            "`spacetime {removed}` must be rejected by the parser, got: {stderr}"
        );
        assert!(
            !dir.path().join("tasks.db").exists(),
            "`spacetime {removed}` must not open a database"
        );
    }
}

#[test]
fn spacetime_dump_server_and_restore_remain() {
    for kept in ["dump-server", "restore"] {
        let out = binary()
            .args(["spacetime", kept, "--help"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "`spacetime {kept}` must still exist: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

// ---------------------------------------------------------------------------
// uninstall --purge (mcp-task-tools.allium, "Other CLI subcommands")
// ---------------------------------------------------------------------------

/// At the binary, with `$HOME` and `$XDG_DATA_HOME` both pointed into a temp
/// directory so nothing real is reachable: `--purge` forgets the host file
/// and log, states no task count (counting would mean opening a database),
/// and leaves the populated leftover `tasks.db` exactly as it was. Every
/// prompt is answered yes on stdin.
#[tokio::test]
async fn purge_states_no_task_count_and_never_opens_the_legacy_database() {
    let home = tempfile::tempdir().unwrap();
    let xdg = home.path().join("xdg");
    let data_dir = xdg.join("dispatch");
    std::fs::create_dir_all(&data_dir).unwrap();
    legacy_database(&data_dir).await;
    std::fs::write(data_dir.join("host.json"), br#"{"host_id":"h"}"#).unwrap();
    std::fs::write(data_dir.join("app.log"), b"log").unwrap();
    let before = footprint(&data_dir);

    let mut child = binary()
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", &xdg)
        .env_remove("DISPATCH_DATA_DIR")
        .args(["uninstall", "--yes", "--purge"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"y\ny\ny\ny\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        !stderr.contains("task(s)") && !stdout.contains("task(s)"),
        "the purge confirmation must state no task count, got stderr: {stderr}"
    );
    assert!(
        !data_dir.join("host.json").exists(),
        "purge forgets this machine's identity; stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(!data_dir.join("app.log").exists(), "purge removes the log");
    assert_eq!(
        footprint(&data_dir),
        before,
        "purge must not open, read-and-checkpoint, or delete the leftover tasks.db"
    );
}

// ---------------------------------------------------------------------------
// StoreInUseNeverOpensSqlite / CodeNeverTouchesLegacyDatabase, at the code
// ---------------------------------------------------------------------------

/// Production source under `src/`: every `.rs` file except generated
/// bindings and test files, with inline
/// `#[cfg(test)] mod … { … }` blocks and `//` comments removed.
fn production_sources() -> Vec<(PathBuf, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path.strip_prefix(&root).unwrap().to_path_buf();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if path.is_dir() {
                if name != "tests" && name != "bindings" {
                    stack.push(path);
                }
                continue;
            }
            if !name.ends_with(".rs") || name == "tests.rs" || name.ends_with("_tests.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            out.push((rel, strip_tests_and_comments(&text)));
        }
    }
    out
}

fn strip_tests_and_comments(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut kept = String::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim_start();
        if line.starts_with("#[cfg(test)]") {
            // Skip attributes, then a `mod name {` block by brace depth.
            let mut j = i + 1;
            while j < lines.len() && lines[j].trim_start().starts_with("#[") {
                j += 1;
            }
            if j < lines.len() {
                let head = lines[j].trim_start();
                let is_mod = head.starts_with("mod ") || head.starts_with("pub(crate) mod ");
                if is_mod && head.contains('{') {
                    let mut depth: i64 = 0;
                    let mut k = j;
                    loop {
                        depth += lines[k].matches('{').count() as i64;
                        depth -= lines[k].matches('}').count() as i64;
                        k += 1;
                        if depth <= 0 || k >= lines.len() {
                            break;
                        }
                    }
                    i = k;
                    continue;
                }
            }
        }
        if !line.starts_with("//") {
            kept.push_str(lines[i]);
            kept.push('\n');
        }
        i += 1;
    }
    kept
}

/// Dispatch links no SQLite at all: not in `Cargo.toml`, not in a source file
/// (`storage.allium: StoreInUseNeverOpensSqlite`). The store is the shared
/// store, real or the in-memory fake, and nothing else.
#[test]
fn dispatch_links_no_sqlite() {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    let manifest_hits: Vec<&str> = manifest
        .lines()
        .filter(|l| !l.trim_start().starts_with('#') && l.to_lowercase().contains("sqlite"))
        .collect();
    assert!(
        manifest_hits.is_empty(),
        "Cargo.toml must name no SQLite crate; found: {manifest_hits:?}"
    );

    let mut hits = Vec::new();
    let mut stack = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                for (n, line) in text.lines().enumerate() {
                    if line.contains("rusqlite") {
                        hits.push(format!("{}:{}", path.display(), n + 1));
                    }
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "no source may use rusqlite; found: {hits:?}"
    );
}

/// No production path constructs a SQLite store: not the board, not MCP, not
/// any subcommand, not `uninstall`.
#[test]
fn no_production_code_opens_a_sqlite_database() {
    let mut hits = Vec::new();
    for (path, text) in production_sources() {
        for line in text.lines() {
            if [
                "Store::open(",
                "Connection::open(",
                "Connection::open_with_flags(",
            ]
            .iter()
            .any(|p| line.contains(p))
            {
                hits.push(format!("src/{}: {}", path.display(), line.trim()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "production code must open no SQLite database (storage.allium: \
         StoreInUseNeverOpensSqlite); found:\n{}",
        hits.join("\n")
    );
}

/// Nothing names the legacy database's WAL companions — the files only a
/// path that opens, deletes or moves the database would need to name.
#[test]
fn no_production_code_names_the_legacy_database_companions() {
    let mut hits = Vec::new();
    for (path, text) in production_sources() {
        for line in text.lines() {
            if line.contains("-wal\"") || line.contains("-shm\"") {
                hits.push(format!("src/{}: {}", path.display(), line.trim()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "nothing may delete, rename or move tasks.db-wal/-shm \
         (storage.allium: CodeNeverTouchesLegacyDatabase); found:\n{}",
        hits.join("\n")
    );
}
