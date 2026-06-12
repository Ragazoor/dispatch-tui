//! Per-repo semantic search (RAG): indexing source files into a per-repo
//! embedding store and querying it by similarity.
//!
//! The work is split across focused submodules:
//! - [`scan`]: file discovery, hashing, and the incremental scan/delete diff.
//! - [`chunking`]: language-aware splitting of file content into chunks.
//! - [`embed`]: reading + chunking files, embedding them, committing to the store.
//! - [`search`]: scanning stored chunks and ranking them against a query.
//!
//! See `docs/specs/repo-rag.allium` for the behavioural specification.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::dispatch::DISPATCH_DIR;
use crate::service::embeddings::EmbeddingService;

mod chunking;
mod embed;
mod scan;
mod search;

/// Number of files embedded per `index_repo` call.
///
/// Keeps each MCP call well within client timeouts. Callers loop until
/// `files_remaining` is zero.
pub const BATCH_SIZE: usize = 50;

pub struct IndexResult {
    pub files_indexed: usize,
    pub files_skipped: usize,
    pub files_remaining: usize,
    pub chunks_total: usize,
    pub duration_ms: u64,
}

pub struct SearchResult {
    pub file_path: String,
    pub chunk_text: String,
    pub score: f32,
}

pub struct RepoIndexService {
    embedding_service: Arc<EmbeddingService>,
}

impl RepoIndexService {
    pub fn new(embedding_service: Arc<EmbeddingService>) -> Self {
        Self { embedding_service }
    }

    pub async fn index_repo(&self, repo_path: &Path, batch_size: usize) -> Result<IndexResult> {
        let start = std::time::Instant::now();
        let repo_path = repo_path.to_owned();

        let scan = tokio::task::spawn_blocking({
            let repo_path = repo_path.clone();
            move || scan::scan_files(&repo_path)
        })
        .await??;

        let files_remaining = scan.to_index.len().saturating_sub(batch_size);
        let mut to_index = scan.to_index;
        to_index.truncate(batch_size);

        let file_chunks = embed::read_and_chunk_files(to_index).await?;
        let embedded = embed::embed_file_chunks(&self.embedding_service, file_chunks).await?;
        let files_indexed = embedded.len();

        let chunks_total = tokio::task::spawn_blocking({
            let repo_path = repo_path.clone();
            let to_delete = scan.to_delete;
            move || embed::commit_index(&repo_path, &to_delete, &embedded)
        })
        .await??;

        Ok(IndexResult {
            files_indexed,
            files_skipped: scan.skipped,
            files_remaining,
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
        let db_path = repo_path.join(DISPATCH_DIR).join("rag.db");
        if !db_path.exists() {
            return Ok(vec![]);
        }

        let query_vec = self.embedding_service.embed(query.to_owned()).await?;

        // Scoring, sorting, and truncation all happen inside spawn_blocking so no
        // CPU-bound work runs on the async runtime thread.
        let scored: Vec<SearchResult> = tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            move || search::query_and_rank(&repo_path, &query_vec, limit)
        })
        .await??;

        Ok(scored)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::scan::open_rag_db;
    use super::*;
    use crate::service::embeddings::RAG_SIMILARITY_THRESHOLD;

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
        let result = svc.index_repo(dir.path(), BATCH_SIZE).await.unwrap();

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
        svc.index_repo(dir.path(), BATCH_SIZE).await.unwrap();

        let result2 = svc.index_repo(dir.path(), BATCH_SIZE).await.unwrap();
        assert_eq!(result2.files_indexed, 0);
        assert_eq!(result2.files_skipped, 1);
    }

    #[tokio::test]
    async fn index_repo_reindexes_changed_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, "# Note\n\nOriginal.").unwrap();

        let svc = RepoIndexService::new(EmbeddingService::new_test());
        svc.index_repo(dir.path(), BATCH_SIZE).await.unwrap();

        std::fs::write(&path, "# Note\n\nModified.").unwrap();
        let result = svc.index_repo(dir.path(), BATCH_SIZE).await.unwrap();
        assert_eq!(result.files_indexed, 1);
        assert_eq!(result.files_skipped, 0);
    }

    #[tokio::test]
    async fn index_repo_removes_deleted_file_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, "# Note\n\nContent.").unwrap();

        let svc = RepoIndexService::new(EmbeddingService::new_test());
        svc.index_repo(dir.path(), BATCH_SIZE).await.unwrap();

        std::fs::remove_file(&path).unwrap();
        svc.index_repo(dir.path(), BATCH_SIZE).await.unwrap();

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
        svc.index_repo(dir.path(), BATCH_SIZE).await.unwrap();
        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains(".dispatch/"));
    }

    #[tokio::test]
    async fn index_repo_indexes_rs_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("lib.rs"),
            "/// Adds two numbers.\npub fn add(a: i32, b: i32) -> i32 { a + b }\n\npub fn sub(a: i32, b: i32) -> i32 { a - b }",
        )
        .unwrap();

        let svc = RepoIndexService::new(EmbeddingService::new_test());
        let result = svc.index_repo(dir.path(), BATCH_SIZE).await.unwrap();

        assert_eq!(result.files_indexed, 1);
        assert_eq!(result.files_skipped, 0);
        assert!(result.chunks_total >= 1);

        let conn = open_rag_db(dir.path()).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM rag_chunks", [], |r| r.get(0))
            .unwrap();
        assert!(count >= 1);
    }

    #[tokio::test]
    async fn index_repo_indexes_allium_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("spec.allium"),
            "entity Task {\n    id: TaskId\n}\n\nrule CreateTask {\n    when: foo\n}",
        )
        .unwrap();

        let svc = RepoIndexService::new(EmbeddingService::new_test());
        let result = svc.index_repo(dir.path(), BATCH_SIZE).await.unwrap();

        assert_eq!(result.files_indexed, 1);
        assert!(result.chunks_total >= 1);
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
        svc.index_repo(dir.path(), BATCH_SIZE).await.unwrap();

        let results = svc.search_docs(dir.path(), "escalation", 5).await.unwrap();
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
        svc.index_repo(dir.path(), BATCH_SIZE).await.unwrap();

        let results = svc.search_docs(dir.path(), "query", 2).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn search_docs_results_include_file_path_and_chunk_text() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("my-note.md"), "# My Note\n\nHello world.").unwrap();

        let svc = RepoIndexService::new(EmbeddingService::new_test());
        svc.index_repo(dir.path(), BATCH_SIZE).await.unwrap();

        let results = svc.search_docs(dir.path(), "hello", 5).await.unwrap();
        assert!(!results.is_empty());
        let r = &results[0];
        assert!(r.file_path.ends_with("my-note.md"));
        assert!(!r.chunk_text.is_empty());
        assert!(r.score >= 0.0 && r.score <= 1.0);
    }

    #[tokio::test]
    async fn search_docs_finds_results_in_rs_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("lib.rs"),
            "/// Computes the sum of two integers.\npub fn add(a: i32, b: i32) -> i32 { a + b }",
        )
        .unwrap();

        let svc = RepoIndexService::new(EmbeddingService::new_test());
        svc.index_repo(dir.path(), BATCH_SIZE).await.unwrap();

        let results = svc
            .search_docs(dir.path(), "add integers", 5)
            .await
            .unwrap();
        assert!(!results.is_empty(), "expected at least one result");
        assert!(results[0].file_path.ends_with("lib.rs"));
    }

    #[tokio::test]
    async fn search_docs_results_are_sorted_by_score_descending() {
        let dir = tempfile::tempdir().unwrap();
        // Two documents with different relevance to the query
        std::fs::write(
            dir.path().join("exact.md"),
            "## Exact Match\n\nThis document is about cosine similarity scoring.",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("unrelated.md"),
            "## Unrelated\n\nThis document is about cooking recipes.",
        )
        .unwrap();

        let svc = RepoIndexService::new(EmbeddingService::new_test());
        svc.index_repo(dir.path(), BATCH_SIZE).await.unwrap();

        let results = svc
            .search_docs(dir.path(), "cosine similarity scoring", 5)
            .await
            .unwrap();

        assert!(results.len() >= 2, "expected at least two results");
        for window in results.windows(2) {
            assert!(
                window[0].score >= window[1].score,
                "results must be sorted descending by score: {} < {}",
                window[0].score,
                window[1].score
            );
        }
    }
}
