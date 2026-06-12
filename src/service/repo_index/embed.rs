//! Reading + chunking files, embedding their chunks, and committing the
//! resulting vectors to the per-repo store.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

use crate::service::embeddings::{serialize_embedding, EmbeddingService};

use super::chunking::chunk_for_extension;
use super::scan::{open_rag_db, FileEntry};

pub(crate) struct FileChunks {
    pub(crate) path: String,
    pub(crate) hash: String,
    pub(crate) chunks: Vec<String>,
}

pub(crate) struct EmbeddedFile {
    pub(crate) path: String,
    pub(crate) hash: String,
    pub(crate) chunks: Vec<String>,
    pub(crate) embeddings: Vec<Vec<f32>>,
}

pub(crate) async fn read_and_chunk_files(to_index: Vec<FileEntry>) -> Result<Vec<FileChunks>> {
    let mut file_chunks = Vec::new();
    for entry in to_index {
        let content = tokio::fs::read_to_string(&entry.path).await?;
        let ext = entry
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let chunks = chunk_for_extension(&content, ext);
        let path = entry
            .path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-UTF-8 path: {:?}", entry.path))?
            .to_owned();
        file_chunks.push(FileChunks {
            path,
            hash: entry.hash,
            chunks,
        });
    }
    Ok(file_chunks)
}

pub(crate) async fn embed_file_chunks(
    svc: &EmbeddingService,
    file_chunks: Vec<FileChunks>,
) -> Result<Vec<EmbeddedFile>> {
    let all_chunks: Vec<String> = file_chunks
        .iter()
        .flat_map(|f| f.chunks.iter().cloned())
        .collect();
    let all_chunks_len = all_chunks.len();

    let all_vecs = if all_chunks.is_empty() {
        vec![]
    } else {
        svc.embed_batch(all_chunks).await?
    };
    debug_assert_eq!(
        all_vecs.len(),
        all_chunks_len,
        "embed_batch must return exactly one vector per input"
    );

    let mut embedded = Vec::new();
    let mut offset = 0;
    for fc in file_chunks {
        let n = fc.chunks.len();
        let embeddings = all_vecs[offset..offset + n].to_vec();
        offset += n;
        embedded.push(EmbeddedFile {
            path: fc.path,
            hash: fc.hash,
            chunks: fc.chunks,
            embeddings,
        });
    }
    Ok(embedded)
}

pub(crate) fn commit_index(
    repo_path: &Path,
    to_delete: &[String],
    embedded: &[EmbeddedFile],
) -> Result<usize> {
    let mut conn = open_rag_db(repo_path)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let tx = conn.transaction()?;
    for path in to_delete {
        tx.execute("DELETE FROM rag_files WHERE file_path = ?1", [path])?;
    }

    for file in embedded {
        tx.execute("DELETE FROM rag_files WHERE file_path = ?1", [&file.path])?;
        tx.execute(
            "INSERT INTO rag_files (file_path, content_hash, indexed_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![file.path, file.hash, now],
        )?;
        for (idx, (text, emb)) in file.chunks.iter().zip(file.embeddings.iter()).enumerate() {
            let blob = serialize_embedding(emb);
            tx.execute(
                "INSERT INTO rag_chunks \
                 (file_path, chunk_index, chunk_text, embedding) \
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![file.path, idx as i64, text, blob],
            )?;
        }
    }

    let existing_count: i64 = tx.query_row("SELECT COUNT(*) FROM rag_chunks", [], |r| r.get(0))?;
    tx.commit()?;

    Ok(existing_count as usize)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    // --- read_and_chunk_files ---

    #[tokio::test]
    async fn read_and_chunk_files_returns_chunks_for_each_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.md");
        std::fs::write(
            &path,
            "## Section A\n\nContent A.\n\n## Section B\n\nContent B.",
        )
        .unwrap();
        let entries = vec![FileEntry {
            path: path.clone(),
            hash: "abc".to_string(),
        }];
        let result = read_and_chunk_files(entries).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].chunks.len(), 2);
        assert_eq!(result[0].hash, "abc");
        assert!(result[0].path.ends_with("doc.md"));
    }

    #[tokio::test]
    async fn read_and_chunk_files_empty_input_returns_empty() {
        let result = read_and_chunk_files(vec![]).await.unwrap();
        assert!(result.is_empty());
    }

    // --- embed_file_chunks ---

    #[tokio::test]
    async fn embed_file_chunks_returns_one_embedded_file_per_input() {
        let svc = EmbeddingService::new_test();
        let file_chunks = vec![
            FileChunks {
                path: "a.md".into(),
                hash: "h1".into(),
                chunks: vec!["chunk one".into(), "chunk two".into()],
            },
            FileChunks {
                path: "b.md".into(),
                hash: "h2".into(),
                chunks: vec!["chunk three".into()],
            },
        ];
        let result = embed_file_chunks(&svc, file_chunks).await.unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].embeddings.len(), 2);
        assert_eq!(result[1].embeddings.len(), 1);
    }

    #[tokio::test]
    async fn embed_file_chunks_empty_input_returns_empty() {
        let svc = EmbeddingService::new_test();
        let result = embed_file_chunks(&svc, vec![]).await.unwrap();
        assert!(result.is_empty());
    }

    // --- commit_index ---

    #[test]
    fn commit_index_inserts_files_and_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let embedded = vec![EmbeddedFile {
            path: "a.md".into(),
            hash: "h1".into(),
            chunks: vec!["chunk one".into()],
            embeddings: vec![vec![0.1f32; 384]],
        }];
        let chunks_total = commit_index(dir.path(), &[], &embedded).unwrap();
        assert_eq!(chunks_total, 1);
        let conn = open_rag_db(dir.path()).unwrap();
        let file_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM rag_files", [], |r| r.get(0))
            .unwrap();
        let chunk_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM rag_chunks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(file_count, 1);
        assert_eq!(chunk_count, 1);
    }

    #[test]
    fn commit_index_deletes_removed_files() {
        let dir = tempfile::tempdir().unwrap();
        // Pre-populate
        let embedded = vec![EmbeddedFile {
            path: "keep.md".into(),
            hash: "h1".into(),
            chunks: vec!["text".into()],
            embeddings: vec![vec![0.1f32; 384]],
        }];
        commit_index(dir.path(), &[], &embedded).unwrap();
        // Second call: delete keep.md, insert nothing
        commit_index(dir.path(), &["keep.md".to_string()], &[]).unwrap();
        let conn = open_rag_db(dir.path()).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM rag_files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
