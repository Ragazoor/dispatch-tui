//! File discovery, hashing, and incremental scan against the stored index.
//!
//! Walks the repo for indexable files (honouring `.gitignore`, skipping the
//! `.dispatch/` directory), hashes each, and diffs against the `rag_files`
//! table to decide what to (re-)index and what to delete.

use std::path::Path;

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::dispatch::{ensure_dispatch_dir_and_gitignore, DISPATCH_DIR};

pub(crate) struct FileEntry {
    pub(crate) path: std::path::PathBuf,
    pub(crate) hash: String,
}

pub(crate) struct ScanResult {
    pub(crate) to_index: Vec<FileEntry>,
    pub(crate) to_delete: Vec<String>,
    pub(crate) skipped: usize,
}

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS rag_files (
        file_path    TEXT PRIMARY KEY,
        content_hash TEXT NOT NULL,
        indexed_at   INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS rag_chunks (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        file_path    TEXT NOT NULL REFERENCES rag_files(file_path) ON DELETE CASCADE,
        chunk_index  INTEGER NOT NULL,
        chunk_text   TEXT NOT NULL,
        embedding    BLOB NOT NULL
    );
    CREATE INDEX IF NOT EXISTS rag_chunks_file ON rag_chunks(file_path);
";

pub(crate) fn open_rag_db(repo_path: &Path) -> Result<rusqlite::Connection> {
    let dispatch_dir = repo_path.join(DISPATCH_DIR);
    std::fs::create_dir_all(&dispatch_dir)?;
    let conn = rusqlite::Connection::open(dispatch_dir.join("rag.db"))?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

// Extensions discovered by the walker. Keep in sync with the match arms in
// `chunking::chunk_for_extension` — an extension walked here but unhandled
// there silently falls back to a single whole-file chunk.
const INDEXABLE_EXTENSIONS: &[&str] = &["md", "rs", "allium"];

fn walk_indexable_files(repo_path: &Path) -> Result<Vec<std::path::PathBuf>> {
    let dispatch_dir = repo_path.join(DISPATCH_DIR);
    let mut files = Vec::new();
    for entry in ignore::WalkBuilder::new(repo_path).hidden(false).build() {
        let entry = entry?;
        let path = entry.path();
        if path.starts_with(&dispatch_dir) {
            continue;
        }
        if entry.file_type().is_some_and(|ft| ft.is_file()) {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if INDEXABLE_EXTENSIONS.contains(&ext) {
                files.push(path.to_owned());
            }
        }
    }
    Ok(files)
}

fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

pub(crate) fn scan_files(repo_path: &Path) -> Result<ScanResult> {
    ensure_dispatch_dir_and_gitignore(repo_path)?;
    let conn = open_rag_db(repo_path)?;
    let on_disk = walk_indexable_files(repo_path)?;

    let in_db: std::collections::HashMap<String, String> = {
        let mut stmt = conn.prepare("SELECT file_path, content_hash FROM rag_files")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };

    let mut to_index = Vec::new();
    let mut skipped = 0usize;
    let mut seen_paths = std::collections::HashSet::new();

    for path in on_disk {
        let hash = hash_file(&path)?;
        let key = path.to_string_lossy().into_owned();
        seen_paths.insert(key.clone());
        if in_db.get(&key).is_none_or(|h| h != &hash) {
            to_index.push(FileEntry { path, hash });
        } else {
            skipped += 1;
        }
    }

    let to_delete: Vec<String> = in_db
        .keys()
        .filter(|k| !seen_paths.contains(*k))
        .cloned()
        .collect();

    Ok(ScanResult {
        to_index,
        to_delete,
        skipped,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    // --- scan_files ---

    #[test]
    fn scan_files_new_files_are_in_to_index() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.md"), "# Note").unwrap();
        let result = scan_files(dir.path()).unwrap();
        assert_eq!(result.to_index.len(), 1);
        assert_eq!(result.skipped, 0);
        assert!(result.to_delete.is_empty());
    }

    #[test]
    fn scan_files_unchanged_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.md"), "# Note").unwrap();
        // First scan: discover the file and its hash.
        let scan = scan_files(dir.path()).unwrap();
        // Manually insert the file record so the second scan sees it.
        let conn = open_rag_db(dir.path()).unwrap();
        let hash = &scan.to_index[0].hash;
        let path_str = scan.to_index[0].path.to_string_lossy().into_owned();
        conn.execute(
            "INSERT INTO rag_files (file_path, content_hash, indexed_at) VALUES (?1, ?2, 0)",
            rusqlite::params![path_str, hash],
        )
        .unwrap();
        drop(conn);
        // Second scan: file unchanged → skipped
        let result2 = scan_files(dir.path()).unwrap();
        assert_eq!(result2.to_index.len(), 0);
        assert_eq!(result2.skipped, 1);
    }

    #[test]
    fn scan_files_deleted_db_files_are_in_to_delete() {
        let dir = tempfile::tempdir().unwrap();
        // Pre-populate DB with a record for a file that doesn't exist on disk
        let conn = open_rag_db(dir.path()).unwrap();
        conn.execute(
            "INSERT INTO rag_files (file_path, content_hash, indexed_at) VALUES (?1, ?2, 0)",
            rusqlite::params!["ghost.md", "deadbeef"],
        )
        .unwrap();
        drop(conn);
        let result = scan_files(dir.path()).unwrap();
        assert!(result.to_delete.contains(&"ghost.md".to_string()));
    }

    // --- open_rag_db ---

    #[test]
    fn open_rag_db_creates_tables() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_rag_db(dir.path()).unwrap();
        let files_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='table' AND name='rag_files'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let chunks_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='table' AND name='rag_chunks'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(files_count, 1);
        assert_eq!(chunks_count, 1);
    }

    #[test]
    fn open_rag_db_creates_dispatch_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!dir.path().join(".dispatch").exists());
        open_rag_db(dir.path()).unwrap();
        assert!(dir.path().join(".dispatch").is_dir());
        assert!(dir.path().join(".dispatch").join("rag.db").exists());
    }

    #[test]
    fn open_rag_db_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        open_rag_db(dir.path()).unwrap();
        let conn = open_rag_db(dir.path()).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM rag_files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    // --- walk_indexable_files ---

    #[test]
    fn walk_indexable_finds_md_rs_and_allium() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.md"), "# Note").unwrap();
        std::fs::write(dir.path().join("lib.rs"), "fn foo() {}").unwrap();
        std::fs::write(dir.path().join("spec.allium"), "entity Task {}").unwrap();
        std::fs::write(dir.path().join("config.txt"), "text").unwrap();
        let found = walk_indexable_files(dir.path()).unwrap();
        assert_eq!(found.len(), 3, "should find .md, .rs, .allium but not .txt");
    }

    #[test]
    fn walk_indexable_skips_dispatch_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".dispatch")).unwrap();
        std::fs::write(dir.path().join(".dispatch").join("ignored.rs"), "fn x() {}").unwrap();
        std::fs::write(dir.path().join("real.rs"), "fn y() {}").unwrap();
        let found = walk_indexable_files(dir.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with("real.rs"));
    }

    #[test]
    fn walk_indexable_ignores_unsupported_extensions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("script.py"), "def foo(): pass").unwrap();
        std::fs::write(dir.path().join("data.json"), "{}").unwrap();
        std::fs::write(dir.path().join("lib.rs"), "fn foo() {}").unwrap();
        let found = walk_indexable_files(dir.path()).unwrap();
        assert_eq!(found.len(), 1, "only .rs should be found");
    }

    // --- hash_file ---

    #[test]
    fn hash_file_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, "hello world").unwrap();
        let h1 = hash_file(&path).unwrap();
        let h2 = hash_file(&path).unwrap();
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn hash_file_differs_for_different_content() {
        let dir = tempfile::tempdir().unwrap();
        let p1 = dir.path().join("a.md");
        let p2 = dir.path().join("b.md");
        std::fs::write(&p1, "content A").unwrap();
        std::fs::write(&p2, "content B").unwrap();
        assert_ne!(hash_file(&p1).unwrap(), hash_file(&p2).unwrap());
    }
}
