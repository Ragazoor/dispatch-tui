use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::dispatch::ensure_dispatch_dir_and_gitignore;
use crate::service::embeddings::{
    cosine_similarity, deserialize_embedding, serialize_embedding, EmbeddingService,
    RAG_SIMILARITY_THRESHOLD,
};

pub struct IndexResult {
    pub files_indexed: usize,
    pub files_skipped: usize,
    pub chunks_total: usize,
    pub duration_ms: u64,
}

pub struct SearchResult {
    pub file_path: String,
    pub chunk_index: usize,
    pub chunk_text: String,
    pub score: f32,
}

struct EmbeddedFile {
    path: String,
    hash: String,
    chunks: Vec<String>,
    embeddings: Vec<Vec<f32>>,
}

// ---------------------------------------------------------------------------
// Chunker
// ---------------------------------------------------------------------------

/// Parse the YAML frontmatter fence and return `(frontmatter_text, body)`.
/// If no valid fence is found, returns `(None, content)`.
fn split_frontmatter(content: &str) -> (Option<&str>, &str) {
    let s = content.trim_start();
    if !s.starts_with("---") {
        return (None, content);
    }
    let Some(after_open) = s.get(3..) else {
        return (None, content);
    };
    let Some(inner) = after_open.strip_prefix('\n') else {
        return (None, content);
    };
    let Some(close) = inner.find("\n---") else {
        return (None, content);
    };
    let fm = inner[..close].trim();
    let body = inner[close + 4..].trim_start_matches('\n');
    (Some(fm).filter(|s| !s.is_empty()), body)
}

/// Split `content` into embedding-ready text chunks.
///
/// Rules:
/// - Each H2 (`## `) header starts a new chunk.
/// - The frontmatter block (if present) is prepended to every chunk.
/// - Files with no H2 headers are one chunk (the whole body).
/// - Files that are empty or contain only frontmatter produce no chunks.
pub fn chunk_file(content: &str) -> Vec<String> {
    let (fm, body) = split_frontmatter(content);

    if body.trim().is_empty() {
        return vec![];
    }

    let prefix: Option<String> = fm.map(|fm| format!("{fm}\n---\n"));

    // Split at H2 boundaries. Content before the first H2 (preamble, H1
    // title, etc.) is merged into the first H2 chunk rather than treated as
    // a standalone chunk — it rarely has standalone retrieval value.
    let mut raw_chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut seen_h2 = false;

    for line in body.lines() {
        if line.starts_with("## ") {
            if seen_h2 && !current.trim().is_empty() {
                raw_chunks.push(current.trim_end().to_string());
                current = String::new();
            }
            seen_h2 = true;
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.trim().is_empty() {
        raw_chunks.push(current.trim_end().to_string());
    }

    if raw_chunks.is_empty() {
        raw_chunks.push(body.trim().to_string());
    }

    match prefix {
        Some(pfx) => raw_chunks.iter().map(|c| format!("{pfx}{c}")).collect(),
        None => raw_chunks,
    }
}

// ---------------------------------------------------------------------------
// Per-repo SQLite DB
// ---------------------------------------------------------------------------

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

fn open_rag_db(repo_path: &Path) -> Result<rusqlite::Connection> {
    let dispatch_dir = repo_path.join(".dispatch");
    std::fs::create_dir_all(&dispatch_dir)?;
    let conn = rusqlite::Connection::open(dispatch_dir.join("rag.db"))?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

// ---------------------------------------------------------------------------
// File utilities
// ---------------------------------------------------------------------------

fn walk_md_files(repo_path: &Path) -> Result<Vec<std::path::PathBuf>> {
    let dispatch_dir = repo_path.join(".dispatch");
    let mut files = Vec::new();
    for entry in ignore::WalkBuilder::new(repo_path).hidden(false).build() {
        let entry = entry?;
        let path = entry.path();
        if path.starts_with(&dispatch_dir) {
            continue;
        }
        if entry.file_type().is_some_and(|ft| ft.is_file())
            && path.extension().and_then(|e| e.to_str()) == Some("md")
        {
            files.push(path.to_owned());
        }
    }
    Ok(files)
}

fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

// ---------------------------------------------------------------------------
// Private type aliases (reduce complexity warnings on spawn_blocking closures)
// ---------------------------------------------------------------------------

type DiffResult = (Vec<(std::path::PathBuf, String)>, Vec<String>, usize);
type ChunkRows = Vec<(String, usize, String, Vec<f32>)>;

// ---------------------------------------------------------------------------
// RepoIndexService
// ---------------------------------------------------------------------------

pub struct RepoIndexService {
    embedding_service: Arc<EmbeddingService>,
}

impl RepoIndexService {
    pub fn new(embedding_service: Arc<EmbeddingService>) -> Self {
        Self { embedding_service }
    }

    pub async fn index_repo(&self, repo_path: &Path) -> Result<IndexResult> {
        let start = std::time::Instant::now();
        let repo_path = repo_path.to_owned();

        // Phase 1 (blocking): walk files, compute hashes, diff against DB.
        let (to_index, to_delete, skipped_count) = tokio::task::spawn_blocking({
            let repo_path = repo_path.clone();
            move || -> Result<DiffResult> {
                let conn = open_rag_db(&repo_path)?;
                let on_disk = walk_md_files(&repo_path)?;

                let in_db: std::collections::HashMap<String, String> = {
                    let mut stmt =
                        conn.prepare("SELECT file_path, content_hash FROM rag_files")?;
                    let rows = stmt.query_map([], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })?;
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
                        to_index.push((path, hash));
                    } else {
                        skipped += 1;
                    }
                }

                let to_delete: Vec<String> = in_db
                    .keys()
                    .filter(|k| !seen_paths.contains(*k))
                    .cloned()
                    .collect();

                Ok((to_index, to_delete, skipped))
            }
        })
        .await??;

        // Phase 2 (async): embed chunks for changed files.
        let mut embedded: Vec<EmbeddedFile> = Vec::new();

        for (path, hash) in to_index {
            let content = tokio::fs::read_to_string(&path).await?;
            let chunks = chunk_file(&content);
            if chunks.is_empty() {
                embedded.push(EmbeddedFile {
                    path: path.to_string_lossy().into_owned(),
                    hash,
                    chunks: vec![],
                    embeddings: vec![],
                });
                continue;
            }
            let vecs = self.embedding_service.embed_batch(chunks.clone()).await?;
            embedded.push(EmbeddedFile {
                path: path.to_string_lossy().into_owned(),
                hash,
                chunks,
                embeddings: vecs,
            });
        }

        let files_indexed = embedded.len();

        // Phase 3 (blocking): write to DB and update .gitignore.
        let chunks_total = tokio::task::spawn_blocking({
            let repo_path = repo_path.clone();
            move || -> Result<usize> {
                ensure_dispatch_dir_and_gitignore(&repo_path)?;
                let conn = open_rag_db(&repo_path)?;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;

                for path in &to_delete {
                    conn.execute("DELETE FROM rag_files WHERE file_path = ?1", [path])?;
                }

                for file in &embedded {
                    conn.execute(
                        "DELETE FROM rag_files WHERE file_path = ?1",
                        [&file.path],
                    )?;
                    conn.execute(
                        "INSERT INTO rag_files (file_path, content_hash, indexed_at) \
                         VALUES (?1, ?2, ?3)",
                        rusqlite::params![file.path, file.hash, now],
                    )?;
                    for (idx, (text, emb)) in
                        file.chunks.iter().zip(file.embeddings.iter()).enumerate()
                    {
                        let blob = serialize_embedding(emb);
                        conn.execute(
                            "INSERT INTO rag_chunks \
                             (file_path, chunk_index, chunk_text, embedding) \
                             VALUES (?1, ?2, ?3, ?4)",
                            rusqlite::params![file.path, idx as i64, text, blob],
                        )?;
                    }
                }

                let existing_count: i64 =
                    conn.query_row("SELECT COUNT(*) FROM rag_chunks", [], |r| r.get(0))?;

                Ok(existing_count as usize)
            }
        })
        .await??;

        Ok(IndexResult {
            files_indexed,
            files_skipped: skipped_count,
            chunks_total,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    pub async fn search_docs(
        &self,
        repo_path: &Path,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let db_path = repo_path.join(".dispatch").join("rag.db");
        if !db_path.exists() {
            return Ok(vec![]);
        }

        let query_vec = self.embedding_service.embed(query.to_owned()).await?;

        let candidates: ChunkRows = tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            move || -> Result<ChunkRows> {
                let conn = open_rag_db(&repo_path)?;
                let mut stmt = conn.prepare(
                    "SELECT file_path, chunk_index, chunk_text, embedding \
                     FROM rag_chunks",
                )?;
                let rows = stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, usize>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Vec<u8>>(3)?,
                    ))
                })?;
                rows.map(|row| {
                    row.map_err(anyhow::Error::from)
                        .map(|(path, idx, text, blob)| {
                            (path, idx, text, deserialize_embedding(&blob))
                        })
                })
                .collect()
            }
        })
        .await??;

        let mut scored: Vec<SearchResult> = candidates
            .into_iter()
            .filter_map(|(path, idx, text, emb)| {
                let score = cosine_similarity(&query_vec, &emb);
                if score < RAG_SIMILARITY_THRESHOLD {
                    return None;
                }
                Some(SearchResult {
                    file_path: path,
                    chunk_index: idx,
                    chunk_text: text,
                    score,
                })
            })
            .collect();

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit);
        Ok(scored)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    // --- chunker ---

    #[test]
    fn chunk_file_no_h2_returns_single_chunk() {
        let content = "# Title\n\nSome body text.";
        let chunks = chunk_file(content);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("Some body text."));
    }

    #[test]
    fn chunk_file_two_h2s_returns_two_chunks() {
        let content = "# Title\n\n## Section A\n\nText A.\n\n## Section B\n\nText B.";
        let chunks = chunk_file(content);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].contains("Section A"));
        assert!(chunks[0].contains("Text A."));
        assert!(chunks[1].contains("Section B"));
        assert!(chunks[1].contains("Text B."));
    }

    #[test]
    fn chunk_file_with_frontmatter_prepends_to_each_chunk() {
        let content =
            "---\ntags: [foo, bar]\n---\n\n## Section A\n\nText A.\n\n## Section B\n\nText B.";
        let chunks = chunk_file(content);
        assert_eq!(chunks.len(), 2);
        for chunk in &chunks {
            assert!(
                chunk.contains("tags: [foo, bar]"),
                "missing frontmatter in: {chunk}"
            );
        }
    }

    #[test]
    fn chunk_file_no_h2_with_frontmatter_is_one_chunk_with_prefix() {
        let content = "---\ninterviewee: Gustaf\n---\n\n# Title\n\nBody text.";
        let chunks = chunk_file(content);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("interviewee: Gustaf"));
        assert!(chunks[0].contains("Body text."));
    }

    #[test]
    fn chunk_file_empty_body_returns_no_chunks() {
        let content = "---\ntags: [foo]\n---\n";
        let chunks = chunk_file(content);
        assert!(chunks.is_empty());
    }

    #[test]
    fn chunk_file_empty_string_returns_no_chunks() {
        let chunks = chunk_file("");
        assert!(chunks.is_empty());
    }

    #[test]
    fn chunk_file_h2_at_start_of_body() {
        let content = "## Only Section\n\nContent here.";
        let chunks = chunk_file(content);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("Only Section"));
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

    // --- walk_md_files ---

    #[test]
    fn walk_md_finds_markdown_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "# A").unwrap();
        std::fs::write(dir.path().join("b.md"), "# B").unwrap();
        std::fs::write(dir.path().join("c.txt"), "text").unwrap();
        let found = walk_md_files(dir.path()).unwrap();
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn walk_md_skips_dispatch_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".dispatch")).unwrap();
        std::fs::write(dir.path().join(".dispatch").join("something.md"), "# X").unwrap();
        std::fs::write(dir.path().join("real.md"), "# Real").unwrap();
        let found = walk_md_files(dir.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with("real.md"));
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

    // --- index_repo ---

    #[tokio::test]
    async fn index_repo_indexes_md_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("note.md"),
            "---\ntags: [foo]\n---\n\n## Section\n\nContent here.",
        )
        .unwrap();

        let svc = RepoIndexService::new(EmbeddingService::new_test());
        let result = svc.index_repo(dir.path()).await.unwrap();

        assert_eq!(result.files_indexed, 1);
        assert_eq!(result.files_skipped, 0);
        assert!(result.chunks_total >= 1);

        let conn = open_rag_db(dir.path()).unwrap();
        let chunk_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM rag_chunks", [], |r| r.get(0))
            .unwrap();
        assert!(chunk_count >= 1);
    }

    #[tokio::test]
    async fn index_repo_second_run_skips_unchanged_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.md"), "# Note\n\nContent.").unwrap();

        let svc = RepoIndexService::new(EmbeddingService::new_test());
        svc.index_repo(dir.path()).await.unwrap();

        let result2 = svc.index_repo(dir.path()).await.unwrap();
        assert_eq!(result2.files_indexed, 0);
        assert_eq!(result2.files_skipped, 1);
    }

    #[tokio::test]
    async fn index_repo_reindexes_changed_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, "# Note\n\nOriginal.").unwrap();

        let svc = RepoIndexService::new(EmbeddingService::new_test());
        svc.index_repo(dir.path()).await.unwrap();

        std::fs::write(&path, "# Note\n\nModified.").unwrap();
        let result = svc.index_repo(dir.path()).await.unwrap();
        assert_eq!(result.files_indexed, 1);
        assert_eq!(result.files_skipped, 0);
    }

    #[tokio::test]
    async fn index_repo_removes_deleted_file_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, "# Note\n\nContent.").unwrap();

        let svc = RepoIndexService::new(EmbeddingService::new_test());
        svc.index_repo(dir.path()).await.unwrap();

        std::fs::remove_file(&path).unwrap();
        svc.index_repo(dir.path()).await.unwrap();

        let conn = open_rag_db(dir.path()).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM rag_chunks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn index_repo_creates_gitignore_entry() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.md"), "# Note").unwrap();
        let svc = RepoIndexService::new(EmbeddingService::new_test());
        svc.index_repo(dir.path()).await.unwrap();
        let content =
            std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains(".dispatch/"));
    }

    // --- search_docs ---

    #[tokio::test]
    async fn search_docs_returns_empty_when_not_indexed() {
        let dir = tempfile::tempdir().unwrap();
        let svc = RepoIndexService::new(EmbeddingService::new_test());
        let results = svc.search_docs(dir.path(), "anything", 5).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn search_docs_returns_results_after_indexing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("note.md"),
            "# Title\n\nContent about escalation patterns.",
        )
        .unwrap();

        let svc = RepoIndexService::new(EmbeddingService::new_test());
        svc.index_repo(dir.path()).await.unwrap();

        let results = svc
            .search_docs(dir.path(), "escalation", 5)
            .await
            .unwrap();
        assert!(!results.is_empty());
        assert!(results[0].score > RAG_SIMILARITY_THRESHOLD);
    }

    #[tokio::test]
    async fn search_docs_respects_limit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("note.md"),
            "## A\n\nText A.\n\n## B\n\nText B.\n\n## C\n\nText C.",
        )
        .unwrap();

        let svc = RepoIndexService::new(EmbeddingService::new_test());
        svc.index_repo(dir.path()).await.unwrap();

        let results = svc.search_docs(dir.path(), "query", 2).await.unwrap();
        assert!(results.len() <= 2);
    }

    #[tokio::test]
    async fn search_docs_results_include_file_path_and_chunk_text() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("my-note.md"), "# My Note\n\nHello world.").unwrap();

        let svc = RepoIndexService::new(EmbeddingService::new_test());
        svc.index_repo(dir.path()).await.unwrap();

        let results = svc.search_docs(dir.path(), "hello", 5).await.unwrap();
        assert!(!results.is_empty());
        let r = &results[0];
        assert!(r.file_path.ends_with("my-note.md"));
        assert!(!r.chunk_text.is_empty());
        assert!(r.score >= 0.0 && r.score <= 1.0);
    }
}
