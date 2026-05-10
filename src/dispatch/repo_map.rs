//! Repo map (T2 / ctags) — structural summary injected into dispatch prompts.
//!
//! At TUI startup, [`detect_ctags`] probes for a Universal Ctags binary.
//! At dispatch time, [`generate`] shells out to that binary, parses its NDJSON
//! output, and renders a token-budgeted, file-grouped, kind-prefixed summary.
//! Failure modes (binary missing, non-zero exit, timeout) all return `None`
//! so dispatch never fails because of a missing or broken map.
//!
//! See the `AugmentDispatchPromptWithRepoMap` rule in `docs/specs/tasks.allium`.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::process::ProcessRunner;

/// Default per-file symbol cap. Prevents one file with thousands of tags from
/// monopolising the budget.
pub const PER_FILE_SYMBOL_CAP: usize = 50;

/// Default watchdog for `ctags -R` invocations.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Approximate "characters per token" for budget sizing. The output budget is
/// expressed in tokens; the renderer multiplies by this constant before
/// truncating.
pub const CHARS_PER_TOKEN: usize = 4;

/// Process-wide repo-map settings. Set once at TUI startup
/// (see `runtime::run_tui`) and read by dispatch code without further
/// plumbing. Absent when `--no-repo-map` is set or detection failed.
#[derive(Debug, Clone)]
pub struct RepoMapSettings {
    /// Detected Universal Ctags executable. `None` means "no map will be
    /// generated" (the binary is missing or rejected).
    pub binary: Option<CtagsBinary>,
    /// Token budget for the rendered map.
    pub budget_tokens: usize,
    /// Watchdog for `ctags -R` invocations.
    pub timeout: Duration,
}

impl RepoMapSettings {
    pub fn disabled() -> Self {
        Self {
            binary: None,
            budget_tokens: 0,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

static REPO_MAP_SETTINGS: OnceLock<RepoMapSettings> = OnceLock::new();

/// Install the process-wide repo-map settings. Idempotent — only the first
/// call succeeds; subsequent calls are silently ignored. Tests that need a
/// different value should use [`generate`] directly with a `MockCtagsExec`.
pub fn install_settings(settings: RepoMapSettings) {
    let _ = REPO_MAP_SETTINGS.set(settings);
}

/// Snapshot of the installed settings, or [`RepoMapSettings::disabled`] when
/// `install_settings` was never called (e.g. in unit tests).
pub fn settings() -> RepoMapSettings {
    REPO_MAP_SETTINGS
        .get()
        .cloned()
        .unwrap_or_else(RepoMapSettings::disabled)
}

/// A detected Universal Ctags executable name (e.g. `"ctags"` or
/// `"universal-ctags"`). Held by [`RepoMapSettings`] after startup detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CtagsBinary(pub String);

impl CtagsBinary {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One ctags entry, parsed from a single NDJSON line.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TagEntry {
    pub path: String,
    pub name: String,
    pub kind: String,
}

/// Abstraction over the ctags shell-out, used by [`generate`]. Production code
/// uses [`SystemCtagsExec`] which spawns a real child process with a timeout
/// watchdog. Tests use [`mock::MockCtagsExec`] to inject canned NDJSON output
/// or simulate slow / failing invocations without touching the filesystem.
pub trait CtagsExec: Send + Sync {
    /// Execute `binary -R --output-format=json <worktree>`. Returns the
    /// captured stdout on success, `None` on failure (non-zero exit, missing
    /// binary, or timeout).
    fn execute(&self, binary: &str, worktree: &Path, timeout: Duration) -> Option<Vec<u8>>;
}

/// Real ctags executor — spawns the child, polls for completion, kills on
/// timeout. Used in production.
pub struct SystemCtagsExec;

impl CtagsExec for SystemCtagsExec {
    fn execute(&self, binary: &str, worktree: &Path, timeout: Duration) -> Option<Vec<u8>> {
        let mut child = std::process::Command::new(binary)
            .arg("-R")
            .arg("--output-format=json")
            .arg(worktree)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;

        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        return None;
                    }
                    return child.wait_with_output().ok().map(|out| out.stdout);
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => return None,
            }
        }
    }
}

/// Probe the system for a Universal Ctags binary. Tries `ctags` first, then
/// `universal-ctags`. Accepts the first whose `--version` stdout begins with
/// "Universal Ctags" (Exuberant Ctags is rejected — its JSON output is
/// incompatible). Returns `None` if neither probe succeeds.
pub fn detect_ctags(runner: &dyn ProcessRunner) -> Option<CtagsBinary> {
    for candidate in ["ctags", "universal-ctags"] {
        let Ok(output) = runner.run(candidate, &["--version"]) else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim_start().starts_with("Universal Ctags") {
            return Some(CtagsBinary(candidate.to_string()));
        }
    }
    None
}

/// Parse line-delimited JSON output from `ctags --output-format=json`.
/// Malformed lines are skipped (not fatal). Kinds are normalised to lowercase
/// with whitespace replaced by `-`.
pub fn parse_ctags_json(stdout: &[u8]) -> Vec<TagEntry> {
    let text = String::from_utf8_lossy(stdout);
    let mut tags = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(mut entry) = serde_json::from_str::<TagEntry>(line) {
            entry.kind = normalise_kind(&entry.kind);
            tags.push(entry);
        }
    }
    tags
}

fn normalise_kind(kind: &str) -> String {
    kind.split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase()
}

/// Format tags as a token-budgeted plain-text summary grouped by file.
///
/// Files are ranked by symbol count descending. Per file: file path on its
/// own line, then one line per kind with a comma-separated list of names.
/// Per-file output is capped at [`PER_FILE_SYMBOL_CAP`] symbols. Output never
/// exceeds `budget_tokens * CHARS_PER_TOKEN` characters; truncation occurs at
/// a whole file block (never mid-line).
pub fn format_grouped(tags: &[TagEntry], budget_tokens: usize) -> String {
    if tags.is_empty() || budget_tokens == 0 {
        return String::new();
    }
    let max_chars = budget_tokens * CHARS_PER_TOKEN;

    let mut by_file: BTreeMap<&str, Vec<&TagEntry>> = BTreeMap::new();
    for tag in tags {
        by_file.entry(&tag.path).or_default().push(tag);
    }

    // Stable rank: symbol count desc, then path asc (tie-break for determinism).
    let mut files: Vec<(&str, Vec<&TagEntry>)> = by_file.into_iter().collect();
    files.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(b.0)));

    let mut out = String::new();
    for (path, mut entries) in files {
        if entries.len() > PER_FILE_SYMBOL_CAP {
            entries.truncate(PER_FILE_SYMBOL_CAP);
        }
        let block = format_file_block(path, &entries);
        if out.len() + block.len() > max_chars {
            break;
        }
        out.push_str(&block);
    }
    out
}

fn format_file_block(path: &str, entries: &[&TagEntry]) -> String {
    let mut by_kind: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in entries {
        by_kind
            .entry(e.kind.as_str())
            .or_default()
            .push(e.name.as_str());
    }
    let mut block = String::new();
    block.push_str(path);
    block.push('\n');
    for (kind, mut names) in by_kind {
        names.sort();
        block.push_str("  ");
        block.push_str(kind);
        block.push(' ');
        block.push_str(&names.join(", "));
        block.push('\n');
    }
    block
}

/// Generate a repo map for `worktree`. Returns `None` when the binary is
/// absent, the invocation fails, the watchdog fires, or the result is empty.
/// On success, also writes the rendered text to
/// `<worktree>/.dispatch/repo-map.txt` (overwriting any prior cache).
pub fn generate(
    exec: &dyn CtagsExec,
    binary: Option<&CtagsBinary>,
    worktree: &Path,
    budget_tokens: usize,
    timeout: Duration,
) -> Option<String> {
    let binary = binary?;
    if budget_tokens == 0 {
        return None;
    }

    let stdout = exec.execute(binary.as_str(), worktree, timeout)?;
    let tags = parse_ctags_json(&stdout);
    let formatted = format_grouped(&tags, budget_tokens);
    if formatted.is_empty() {
        return None;
    }

    write_cache(worktree, &formatted);
    Some(formatted)
}

fn write_cache(worktree: &Path, contents: &str) {
    let dir = worktree.join(".dispatch");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::debug!(error = ?e, dir = %dir.display(), "failed to create .dispatch dir for repo-map cache");
        return;
    }
    let path = dir.join("repo-map.txt");
    if let Err(e) = std::fs::write(&path, contents) {
        tracing::debug!(error = ?e, path = %path.display(), "failed to write repo-map cache");
    }
}

#[cfg(any(test, debug_assertions))]
pub mod mock {
    //! In-process `CtagsExec` for tests and dispatch-wiring fixtures.

    use super::{CtagsExec, Duration};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    type MockResponse = (Duration, Option<Vec<u8>>);

    pub struct MockCtagsExec {
        responses: Mutex<Vec<MockResponse>>,
        calls: Mutex<Vec<(String, PathBuf, Duration)>>,
    }

    impl MockCtagsExec {
        pub fn new(responses: Vec<MockResponse>) -> Self {
            Self {
                responses: Mutex::new(responses),
                calls: Mutex::new(Vec::new()),
            }
        }

        /// Helper: a single immediate-success response.
        pub fn ok(stdout: &[u8]) -> Self {
            Self::new(vec![(Duration::from_millis(0), Some(stdout.to_vec()))])
        }

        /// Helper: a single immediate-failure response.
        pub fn fail() -> Self {
            Self::new(vec![(Duration::from_millis(0), None)])
        }

        #[allow(clippy::unwrap_used)] // test helper — panics on poisoned mutex
        pub fn recorded_calls(&self) -> Vec<(String, PathBuf, Duration)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl CtagsExec for MockCtagsExec {
        #[allow(clippy::unwrap_used)] // test helper — panics on poisoned mutex
        fn execute(&self, binary: &str, worktree: &Path, timeout: Duration) -> Option<Vec<u8>> {
            self.calls
                .lock()
                .unwrap()
                .push((binary.to_string(), worktree.to_path_buf(), timeout));
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return None;
            }
            let (delay, payload) = responses.remove(0);
            // If the canned delay exceeds the requested timeout, simulate a
            // timeout: return None without actually sleeping.
            if delay > timeout {
                return None;
            }
            payload
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::mock::MockCtagsExec;
    use super::*;
    use crate::process::{exit_ok, MockProcessRunner};
    use std::process::Output;
    use tempfile::tempdir;

    fn version_ok(stdout: &str) -> anyhow::Result<Output> {
        Ok(Output {
            status: exit_ok(),
            stdout: stdout.as_bytes().to_vec(),
            stderr: vec![],
        })
    }

    fn version_err() -> anyhow::Result<Output> {
        Err(anyhow::anyhow!("command-not-found"))
    }

    // ---- detect_ctags ----

    #[test]
    fn detect_ctags_prefers_ctags_when_universal() {
        let runner = MockProcessRunner::new(vec![version_ok(
            "Universal Ctags 6.2.1, Copyright (C) ...\n",
        )]);
        let detected = detect_ctags(&runner);
        assert_eq!(detected, Some(CtagsBinary("ctags".to_string())));
        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "ctags");
    }

    #[test]
    fn detect_ctags_falls_back_to_universal_ctags() {
        let runner =
            MockProcessRunner::new(vec![version_err(), version_ok("Universal Ctags 6.2\n")]);
        let detected = detect_ctags(&runner);
        assert_eq!(detected, Some(CtagsBinary("universal-ctags".to_string())));
        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].0, "universal-ctags");
    }

    #[test]
    fn detect_ctags_rejects_exuberant_ctags() {
        let runner =
            MockProcessRunner::new(vec![version_ok("Exuberant Ctags 5.8\n"), version_err()]);
        let detected = detect_ctags(&runner);
        assert_eq!(detected, None);
    }

    #[test]
    fn detect_ctags_returns_none_when_neither_present() {
        let runner = MockProcessRunner::new(vec![version_err(), version_err()]);
        let detected = detect_ctags(&runner);
        assert_eq!(detected, None);
    }

    // ---- parse_ctags_json ----

    const SAMPLE_NDJSON: &[u8] = concat!(
        r#"{"_type":"tag","name":"foo","path":"src/a.rs","kind":"function"}"#,
        "\n",
        r#"{"_type":"tag","name":"Bar","path":"src/a.rs","kind":"struct"}"#,
        "\n",
        r#"{"_type":"tag","name":"baz","path":"src/b.rs","kind":"function"}"#,
        "\n",
    )
    .as_bytes();

    #[test]
    fn parse_ctags_json_extracts_path_kind_name() {
        let tags = parse_ctags_json(SAMPLE_NDJSON);
        assert_eq!(tags.len(), 3);
        assert_eq!(tags[0].name, "foo");
        assert_eq!(tags[0].path, "src/a.rs");
        assert_eq!(tags[0].kind, "function");
        assert_eq!(tags[1].kind, "struct");
        assert_eq!(tags[2].path, "src/b.rs");
    }

    #[test]
    fn parse_ctags_json_normalises_kind_lowercase_dashes() {
        let line = r#"{"_type":"tag","name":"f","path":"a.ts","kind":"arrow function"}"#;
        let tags = parse_ctags_json(line.as_bytes());
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].kind, "arrow-function");
    }

    #[test]
    fn parse_ctags_json_skips_malformed_lines() {
        let mixed = concat!(
            r#"{"_type":"tag","name":"f","path":"a.rs","kind":"function"}"#,
            "\n",
            "this is not json\n",
            r#"{"name":"g""#,
            " (truncated)\n",
            r#"{"_type":"tag","name":"h","path":"b.rs","kind":"struct"}"#,
            "\n",
        );
        let tags = parse_ctags_json(mixed.as_bytes());
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].name, "f");
        assert_eq!(tags[1].name, "h");
    }

    // ---- format_grouped ----

    fn tag(path: &str, name: &str, kind: &str) -> TagEntry {
        TagEntry {
            path: path.to_string(),
            name: name.to_string(),
            kind: kind.to_string(),
        }
    }

    #[test]
    fn format_groups_by_file_kind_prefix() {
        let tags = vec![
            tag("src/a.rs", "foo", "function"),
            tag("src/a.rs", "bar", "function"),
            tag("src/a.rs", "Baz", "struct"),
            tag("src/b.rs", "qux", "function"),
        ];
        let out = format_grouped(&tags, 4_000);
        assert!(out.contains("src/a.rs\n"));
        assert!(
            out.contains("  function bar, foo\n"),
            "function names sorted: {out}"
        );
        assert!(out.contains("  struct Baz\n"));
        assert!(out.contains("src/b.rs\n"));
    }

    #[test]
    fn format_respects_token_budget() {
        let mut tags = Vec::new();
        for i in 0..50 {
            tags.push(tag(
                &format!("src/file_{i}.rs"),
                &format!("symbol_{i}"),
                "function",
            ));
        }
        let out = format_grouped(&tags, 12);
        assert!(out.len() <= 12 * CHARS_PER_TOKEN);
        if !out.is_empty() {
            assert!(out.ends_with('\n'));
        }
    }

    #[test]
    fn format_caps_symbols_per_file() {
        let mut tags = Vec::new();
        for i in 0..(PER_FILE_SYMBOL_CAP * 4) {
            tags.push(tag("src/big.rs", &format!("sym_{i}"), "function"));
        }
        let out = format_grouped(&tags, 1_000_000);
        let line = out
            .lines()
            .find(|l| l.starts_with("  function "))
            .expect("function line present");
        let names = line.trim_start_matches("  function ").split(", ").count();
        assert_eq!(names, PER_FILE_SYMBOL_CAP);
    }

    #[test]
    fn format_ranks_files_by_symbol_count_desc() {
        let mut tags = Vec::new();
        for i in 0..2 {
            tags.push(tag("src/small.rs", &format!("a_{i}"), "function"));
        }
        for i in 0..5 {
            tags.push(tag("src/big.rs", &format!("b_{i}"), "function"));
        }
        let out = format_grouped(&tags, 4_000);
        let big = out.find("src/big.rs").expect("big present");
        let small = out.find("src/small.rs").expect("small present");
        assert!(big < small, "big (5 syms) must precede small (2 syms)");
    }

    #[test]
    fn format_empty_tags_returns_empty_string() {
        let out = format_grouped(&[], 4_000);
        assert!(out.is_empty());
    }

    proptest::proptest! {
        #[test]
        fn format_budget_invariant_property(
            seeds in proptest::collection::vec(
                (0usize..20, 0usize..20, 0usize..5),
                0..50,
            ),
            budget in 0usize..1_000,
        ) {
            let tags: Vec<TagEntry> = seeds
                .into_iter()
                .map(|(file, name, kind)| {
                    tag(
                        &format!("src/file_{file}.rs"),
                        &format!("sym_{name}"),
                        match kind {
                            0 => "function",
                            1 => "struct",
                            2 => "enum",
                            3 => "constant",
                            _ => "trait",
                        },
                    )
                })
                .collect();
            let out = format_grouped(&tags, budget);
            proptest::prop_assert!(out.len() <= budget * CHARS_PER_TOKEN);
        }
    }

    // ---- generate ----

    #[test]
    fn generate_returns_none_when_binary_is_none() {
        let exec = MockCtagsExec::new(vec![]);
        let dir = tempdir().expect("tempdir");
        let result = generate(&exec, None, dir.path(), 4_000, DEFAULT_TIMEOUT);
        assert_eq!(result, None);
        assert!(exec.recorded_calls().is_empty());
    }

    #[test]
    fn generate_returns_none_when_ctags_fails() {
        let exec = MockCtagsExec::fail();
        let dir = tempdir().expect("tempdir");
        let bin = CtagsBinary("ctags".to_string());
        let result = generate(&exec, Some(&bin), dir.path(), 4_000, DEFAULT_TIMEOUT);
        assert_eq!(result, None);
    }

    #[test]
    fn generate_returns_none_when_ctags_times_out() {
        let exec = MockCtagsExec::new(vec![(
            Duration::from_millis(200),
            Some(SAMPLE_NDJSON.to_vec()),
        )]);
        let dir = tempdir().expect("tempdir");
        let bin = CtagsBinary("ctags".to_string());
        let result = generate(
            &exec,
            Some(&bin),
            dir.path(),
            4_000,
            Duration::from_millis(50),
        );
        assert_eq!(result, None);
    }

    #[test]
    fn generate_writes_cache_file() {
        let exec = MockCtagsExec::ok(SAMPLE_NDJSON);
        let dir = tempdir().expect("tempdir");
        let bin = CtagsBinary("ctags".to_string());
        let result = generate(&exec, Some(&bin), dir.path(), 4_000, DEFAULT_TIMEOUT);
        assert!(result.is_some());
        let cache = dir.path().join(".dispatch").join("repo-map.txt");
        assert!(cache.exists(), "cache file written");
        let contents = std::fs::read_to_string(&cache).expect("read cache");
        assert_eq!(contents, result.expect("some"));
    }

    #[test]
    fn generate_always_regenerates_overwrites_existing_cache() {
        let dir = tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".dispatch")).expect("mkdir");
        std::fs::write(
            dir.path().join(".dispatch").join("repo-map.txt"),
            "stale content",
        )
        .expect("seed");

        let exec = MockCtagsExec::ok(SAMPLE_NDJSON);
        let bin = CtagsBinary("ctags".to_string());
        let result = generate(&exec, Some(&bin), dir.path(), 4_000, DEFAULT_TIMEOUT).expect("some");
        let contents = std::fs::read_to_string(dir.path().join(".dispatch").join("repo-map.txt"))
            .expect("read");
        assert_ne!(contents, "stale content");
        assert_eq!(contents, result);
    }

    #[test]
    fn generate_invokes_exec_with_binary_and_worktree() {
        let exec = MockCtagsExec::ok(SAMPLE_NDJSON);
        let dir = tempdir().expect("tempdir");
        let bin = CtagsBinary("ctags".to_string());
        let _ = generate(&exec, Some(&bin), dir.path(), 4_000, DEFAULT_TIMEOUT);
        let calls = exec.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "ctags");
        assert_eq!(calls[0].1, dir.path());
        assert_eq!(calls[0].2, DEFAULT_TIMEOUT);
    }

    // Real ctags integration check. Requires `ctags` (Universal Ctags) on PATH.
    // Run with: `cargo test -- --ignored repo_map`
    #[test]
    #[ignore]
    fn real_ctags_polyglot_fixture() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("a.rs"),
            "fn rust_func() {}\nstruct RustStruct {}\n",
        )
        .expect("rs");
        std::fs::write(
            dir.path().join("b.py"),
            "def py_func():\n    pass\n\nclass PyClass:\n    pass\n",
        )
        .expect("py");
        std::fs::write(
            dir.path().join("c.ts"),
            "export function tsFunc(): void {}\nexport class TsClass {}\n",
        )
        .expect("ts");

        let runner = crate::process::RealProcessRunner;
        let bin = detect_ctags(&runner).expect("Universal Ctags on PATH");
        let exec = SystemCtagsExec;
        let result =
            generate(&exec, Some(&bin), dir.path(), 4_000, DEFAULT_TIMEOUT).expect("non-empty map");
        assert!(result.contains("a.rs"), "Rust file present: {result}");
        assert!(result.contains("b.py"), "Python file present: {result}");
        assert!(result.contains("c.ts"), "TypeScript file present: {result}");
    }
}
