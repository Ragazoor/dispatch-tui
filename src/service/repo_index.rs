use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::dispatch::{ensure_dispatch_dir_and_gitignore, DISPATCH_DIR};
use crate::service::embeddings::{
    cosine_similarity, deserialize_embedding, serialize_embedding, EmbeddingService,
    RAG_SIMILARITY_THRESHOLD,
};

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

struct EmbeddedFile {
    path: String,
    hash: String,
    chunks: Vec<String>,
    embeddings: Vec<Vec<f32>>,
}

/// Strips leading visibility modifiers from a line.
fn strip_vis(s: &str) -> &str {
    if let Some(rest) = s.strip_prefix("pub(crate) ") {
        return rest;
    }
    if let Some(rest) = s.strip_prefix("pub(super) ") {
        return rest;
    }
    if let Some(rest) = s.strip_prefix("pub ") {
        return rest;
    }
    s
}

/// Returns true if `line` (at column 0) starts a declaration boundary.
///
/// Visibility (`pub`, `pub(crate)`, `pub(super)`) and async/unsafe modifiers
/// are stripped before checking against `keywords`.
fn is_decl_boundary(line: &str, keywords: &[&str]) -> bool {
    let s = strip_vis(line);
    let s = s.strip_prefix("async ").unwrap_or(s);
    let s = s.strip_prefix("unsafe ").unwrap_or(s);
    // Handle `unsafe async fn` — after stripping `unsafe `, `async ` may still be present.
    let s = s.strip_prefix("async ").unwrap_or(s);
    keywords.iter().any(|kw| s.starts_with(kw))
}

/// Split `content` into chunks at declaration boundaries.
///
/// Uses a two-buffer algorithm:
/// - `main_current`: non-attribute body lines for the current chunk.
/// - `attr_buffer`: pending attribute/doc lines at column 0 that belong to
///   the NEXT chunk when a declaration boundary arrives.
fn chunk_by_declarations(
    content: &str,
    decl_keywords: &[&str],
    is_attr_line: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut main_current = String::new();
    let mut attr_buffer = String::new();
    let mut has_seen_decl = false;

    for line in content.lines() {
        if is_decl_boundary(line, decl_keywords) {
            // main_current is non-empty whenever has_seen_decl is true, but guard is kept for clarity.
            if has_seen_decl && !main_current.is_empty() {
                chunks.push(main_current.trim_end().to_string());
                main_current = format!("{attr_buffer}{line}\n");
            } else {
                // First declaration: absorb preamble + pending attrs
                main_current.push_str(&attr_buffer);
                main_current.push_str(line);
                main_current.push('\n');
            }
            attr_buffer.clear();
            has_seen_decl = true;
        } else if is_attr_line(line) {
            attr_buffer.push_str(line);
            attr_buffer.push('\n');
        } else if line.trim().is_empty() && !attr_buffer.is_empty() {
            // Blank line inside an attr run: keep it in the buffer so it
            // travels with the attribute to the next declaration.
            attr_buffer.push_str(line);
            attr_buffer.push('\n');
        } else {
            // Flush attr_buffer into body (attr was orphaned — not followed by a decl)
            main_current.push_str(&attr_buffer);
            main_current.push_str(line);
            main_current.push('\n');
            attr_buffer.clear();
        }
    }

    // Append any remaining attr_buffer to the last chunk
    let remaining = format!("{main_current}{attr_buffer}");
    if !remaining.trim().is_empty() {
        chunks.push(remaining.trim_end().to_string());
    }

    chunks
}

pub(crate) fn chunk_rust(content: &str) -> Vec<String> {
    const RUST_KEYWORDS: &[&str] = &[
        "fn ", "impl ", "impl<", "struct ", "enum ", "trait ",
        "type ", "mod ", "const ", "static ",
    ];
    chunk_by_declarations(content, RUST_KEYWORDS, |line| {
        line.starts_with("#[") || line.starts_with("///")
    })
}

pub(crate) fn chunk_allium(content: &str) -> Vec<String> {
    const ALLIUM_KEYWORDS: &[&str] = &[
        "entity ", "rule ", "surface ", "config ", "enum ",
        "concept ", "external ", "invariant ", "value ",
    ];
    chunk_by_declarations(content, ALLIUM_KEYWORDS, |line| line.starts_with("-- "))
}

/// Returns `(None, content)` if no valid frontmatter fence is found.
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
pub(crate) fn chunk_markdown(content: &str) -> Vec<String> {
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

pub(crate) fn chunk_for_extension(content: &str, ext: &str) -> Vec<String> {
    match ext {
        "md" => chunk_markdown(content),
        "rs" => chunk_rust(content),
        "allium" => chunk_allium(content),
        _ => {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                vec![]
            } else {
                vec![trimmed.to_string()]
            }
        }
    }
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

fn open_rag_db(repo_path: &Path) -> Result<rusqlite::Connection> {
    let dispatch_dir = repo_path.join(DISPATCH_DIR);
    std::fs::create_dir_all(&dispatch_dir)?;
    let conn = rusqlite::Connection::open(dispatch_dir.join("rag.db"))?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

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

type DiffResult = (Vec<(std::path::PathBuf, String)>, Vec<String>, usize);
type ChunkRows = Vec<(String, String, Vec<f32>)>;

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

        // Phase 1 (blocking): walk files, compute hashes, diff against DB.
        let (to_index, to_delete, skipped_count) = tokio::task::spawn_blocking({
            let repo_path = repo_path.clone();
            move || -> Result<DiffResult> {
                ensure_dispatch_dir_and_gitignore(&repo_path)?;
                let conn = open_rag_db(&repo_path)?;
                let on_disk = walk_indexable_files(&repo_path)?;

                let in_db: std::collections::HashMap<String, String> = {
                    let mut stmt = conn.prepare("SELECT file_path, content_hash FROM rag_files")?;
                    let rows = stmt
                        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
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

        let files_remaining = to_index.len().saturating_sub(batch_size);
        let mut to_index = to_index;
        to_index.truncate(batch_size);

        // Phase 2 (async): read all changed files and embed their chunks in one batch.
        let mut file_chunks: Vec<(String, String, Vec<String>)> = Vec::new();
        for (path, hash) in to_index {
            let content = tokio::fs::read_to_string(&path).await?;
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let chunks = chunk_for_extension(&content, ext);
            let path_str = path
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("non-UTF-8 path: {:?}", path))?
                .to_owned();
            file_chunks.push((path_str, hash, chunks));
        }

        let all_chunks: Vec<String> = file_chunks
            .iter()
            .flat_map(|f| f.2.iter().cloned())
            .collect();
        let all_chunks_len = all_chunks.len();

        let all_vecs = if all_chunks.is_empty() {
            vec![]
        } else {
            self.embedding_service.embed_batch(all_chunks).await?
        };
        debug_assert_eq!(
            all_vecs.len(),
            all_chunks_len,
            "embed_batch must return exactly one vector per input"
        );

        let mut embedded: Vec<EmbeddedFile> = Vec::new();
        let mut offset = 0;
        for fc in file_chunks {
            let n = fc.2.len();
            let embeddings = all_vecs[offset..offset + n].to_vec();
            offset += n;
            embedded.push(EmbeddedFile {
                path: fc.0,
                hash: fc.1,
                chunks: fc.2,
                embeddings,
            });
        }

        let files_indexed = embedded.len();

        // Phase 3 (blocking): write to DB.
        let chunks_total = tokio::task::spawn_blocking({
            let repo_path = repo_path.clone();
            move || -> Result<usize> {
                let mut conn = open_rag_db(&repo_path)?;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;

                let tx = conn.transaction()?;
                for path in &to_delete {
                    tx.execute("DELETE FROM rag_files WHERE file_path = ?1", [path])?;
                }

                for file in &embedded {
                    tx.execute("DELETE FROM rag_files WHERE file_path = ?1", [&file.path])?;
                    tx.execute(
                        "INSERT INTO rag_files (file_path, content_hash, indexed_at) \
                         VALUES (?1, ?2, ?3)",
                        rusqlite::params![file.path, file.hash, now],
                    )?;
                    for (idx, (text, emb)) in
                        file.chunks.iter().zip(file.embeddings.iter()).enumerate()
                    {
                        let blob = serialize_embedding(emb);
                        tx.execute(
                            "INSERT INTO rag_chunks \
                             (file_path, chunk_index, chunk_text, embedding) \
                             VALUES (?1, ?2, ?3, ?4)",
                            rusqlite::params![file.path, idx as i64, text, blob],
                        )?;
                    }
                }

                let existing_count: i64 =
                    tx.query_row("SELECT COUNT(*) FROM rag_chunks", [], |r| r.get(0))?;
                tx.commit()?;

                Ok(existing_count as usize)
            }
        })
        .await??;

        Ok(IndexResult {
            files_indexed,
            files_skipped: skipped_count,
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

        let candidates: ChunkRows = tokio::task::spawn_blocking({
            let repo_path = repo_path.to_owned();
            move || -> Result<ChunkRows> {
                let conn = open_rag_db(&repo_path)?;
                // Limit scan to MAX_SCAN_CHUNKS rows; repos with more chunks will return incomplete results.
                let mut stmt = conn.prepare(
                    "SELECT file_path, chunk_text, embedding FROM rag_chunks LIMIT 1000",
                )?;
                let rows = stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Vec<u8>>(2)?,
                    ))
                })?;
                rows.map(|row| {
                    row.map_err(anyhow::Error::from)
                        .map(|(path, text, blob)| (path, text, deserialize_embedding(&blob)))
                })
                .collect()
            }
        })
        .await??;

        let mut scored: Vec<SearchResult> = candidates
            .into_iter()
            .filter_map(|(path, text, emb)| {
                let score = cosine_similarity(&query_vec, &emb);
                if score < RAG_SIMILARITY_THRESHOLD {
                    return None;
                }
                Some(SearchResult {
                    file_path: path,
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    // --- chunker ---

    #[test]
    fn chunk_markdown_no_h2_returns_single_chunk() {
        let content = "# Title\n\nSome body text.";
        let chunks = chunk_markdown(content);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("Some body text."));
    }

    #[test]
    fn chunk_markdown_two_h2s_returns_two_chunks() {
        let content = "# Title\n\n## Section A\n\nText A.\n\n## Section B\n\nText B.";
        let chunks = chunk_markdown(content);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].contains("Section A"));
        assert!(chunks[0].contains("Text A."));
        assert!(chunks[1].contains("Section B"));
        assert!(chunks[1].contains("Text B."));
    }

    #[test]
    fn chunk_markdown_with_frontmatter_prepends_to_each_chunk() {
        let content =
            "---\ntags: [foo, bar]\n---\n\n## Section A\n\nText A.\n\n## Section B\n\nText B.";
        let chunks = chunk_markdown(content);
        assert_eq!(chunks.len(), 2);
        for chunk in &chunks {
            assert!(
                chunk.contains("tags: [foo, bar]"),
                "missing frontmatter in: {chunk}"
            );
        }
    }

    #[test]
    fn chunk_markdown_no_h2_with_frontmatter_is_one_chunk_with_prefix() {
        let content = "---\ninterviewee: Gustaf\n---\n\n# Title\n\nBody text.";
        let chunks = chunk_markdown(content);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("interviewee: Gustaf"));
        assert!(chunks[0].contains("Body text."));
    }

    #[test]
    fn chunk_markdown_empty_body_returns_no_chunks() {
        let content = "---\ntags: [foo]\n---\n";
        let chunks = chunk_markdown(content);
        assert!(chunks.is_empty());
    }

    #[test]
    fn chunk_markdown_empty_string_returns_no_chunks() {
        let chunks = chunk_markdown("");
        assert!(chunks.is_empty());
    }

    #[test]
    fn chunk_markdown_h2_at_start_of_body() {
        let content = "## Only Section\n\nContent here.";
        let chunks = chunk_markdown(content);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("Only Section"));
    }

    // --- chunk_by_declarations ---

    #[test]
    fn chunk_by_decls_empty_returns_no_chunks() {
        let result = chunk_by_declarations("", &["fn "], |_| false);
        assert!(result.is_empty());
    }

    #[test]
    fn chunk_by_decls_whitespace_only_returns_no_chunks() {
        let result = chunk_by_declarations("   \n\n  ", &["fn "], |_| false);
        assert!(result.is_empty());
    }

    #[test]
    fn chunk_by_decls_no_keyword_match_returns_single_chunk() {
        let content = "use std::fmt;\n\nsome text";
        let result = chunk_by_declarations(content, &["fn "], |_| false);
        assert_eq!(result.len(), 1);
        assert!(result[0].contains("some text"));
    }

    #[test]
    fn chunk_by_decls_two_declarations_produce_two_chunks() {
        let content = "fn foo() {}\n\nfn bar() {}";
        let result = chunk_by_declarations(content, &["fn "], |_| false);
        assert_eq!(result.len(), 2);
        assert!(result[0].contains("fn foo()"), "got: {}", result[0]);
        assert!(result[1].contains("fn bar()"), "got: {}", result[1]);
    }

    #[test]
    fn chunk_by_decls_preamble_merges_into_first_chunk() {
        let content = "use std::fmt;\n\nfn foo() {}";
        let result = chunk_by_declarations(content, &["fn "], |_| false);
        assert_eq!(result.len(), 1);
        assert!(result[0].contains("use std::fmt;"), "got: {}", result[0]);
        assert!(result[0].contains("fn foo()"), "got: {}", result[0]);
    }

    #[test]
    fn chunk_by_decls_attr_before_second_decl_lands_in_new_chunk() {
        let content = "fn foo() {}\n\n#[attr]\nfn bar() {}";
        let result = chunk_by_declarations(content, &["fn "], |line| line.starts_with("#["));
        assert_eq!(result.len(), 2);
        assert!(
            !result[0].contains("#[attr]"),
            "attr should NOT be in foo chunk: {}",
            result[0]
        );
        assert!(
            result[1].contains("#[attr]"),
            "attr should be in bar chunk: {}",
            result[1]
        );
        assert!(result[1].contains("fn bar()"));
    }

    #[test]
    fn chunk_by_decls_non_attr_line_flushes_attr_buffer_into_body() {
        // An attr line followed by a non-attr non-boundary line gets absorbed into the body.
        let content = "fn foo() {}\n#[orphan]\nbody line\nfn bar() {}";
        let result = chunk_by_declarations(content, &["fn "], |line| line.starts_with("#["));
        assert_eq!(result.len(), 2);
        assert!(
            result[0].contains("#[orphan]"),
            "orphaned attr flushed into foo chunk: {}",
            result[0]
        );
        assert!(result[1].contains("fn bar()"));
    }

    #[test]
    fn chunk_by_decls_pub_prefix_triggers_split() {
        let result = chunk_by_declarations(
            "pub fn foo() {}\n\npub fn bar() {}",
            &["fn "],
            |_| false,
        );
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn chunk_by_decls_pub_crate_prefix_triggers_split() {
        let result = chunk_by_declarations(
            "pub(crate) fn foo() {}\n\npub(crate) fn bar() {}",
            &["fn "],
            |_| false,
        );
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn chunk_by_decls_async_prefix_triggers_split() {
        let result = chunk_by_declarations(
            "async fn foo() {}\n\nasync fn bar() {}",
            &["fn "],
            |_| false,
        );
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn chunk_by_decls_unsafe_async_prefix_triggers_split() {
        let result = chunk_by_declarations(
            "unsafe async fn foo() {}\n\nunsafe async fn bar() {}",
            &["fn "],
            |_| false,
        );
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn chunk_by_decls_attr_before_first_decl_stays_in_first_chunk() {
        // attr_buffer is flushed into the first chunk (not lost) when the first declaration arrives.
        let content = "#[derive(Debug)]\nstruct Foo {}";
        let result = chunk_by_declarations(content, &["struct "], |l| l.starts_with("#["));
        assert_eq!(result.len(), 1);
        assert!(result[0].contains("#[derive(Debug)]"), "got: {}", result[0]);
        assert!(result[0].contains("struct Foo"), "got: {}", result[0]);
    }

    #[test]
    fn chunk_by_decls_trailing_attr_absorbed_into_last_chunk() {
        // An attr at the very end of a file (after the last declaration body) stays in that chunk.
        let content = "fn foo() {}\n#[orphan_at_eof]";
        let result = chunk_by_declarations(content, &["fn "], |l| l.starts_with("#["));
        assert_eq!(result.len(), 1);
        assert!(result[0].contains("#[orphan_at_eof]"), "got: {}", result[0]);
    }

    #[test]
    fn chunk_by_decls_adjacent_decls_no_blank_line_produce_two_chunks() {
        let result = chunk_by_declarations("fn foo() {}\nfn bar() {}", &["fn "], |_| false);
        assert_eq!(result.len(), 2, "adjacent decls should each be their own chunk: {:?}", result);
    }

    #[test]
    fn chunk_by_decls_single_fn_returns_one_chunk() {
        let result = chunk_by_declarations("fn foo() {}", &["fn "], |_| false);
        assert_eq!(result.len(), 1);
        assert!(result[0].contains("fn foo()"));
    }

    #[test]
    fn chunk_by_decls_doc_comment_before_decl_stays_with_it() {
        let content = "fn foo() {}\n\n/// Does bar.\nfn bar() {}";
        let result = chunk_by_declarations(content, &["fn "], |l| l.starts_with("///"));
        assert_eq!(result.len(), 2);
        assert!(
            !result[0].contains("/// Does bar."),
            "doc comment must not be in foo chunk: {}",
            result[0]
        );
        assert!(
            result[1].contains("/// Does bar."),
            "doc comment must be in bar chunk: {}",
            result[1]
        );
    }

    #[test]
    fn chunk_by_decls_indented_fn_is_not_a_boundary() {
        let content = "fn outer() {\n    fn inner() {}\n}";
        let result = chunk_by_declarations(content, &["fn "], |_| false);
        assert_eq!(result.len(), 1, "indented fn must not split: {:?}", result);
        assert!(result[0].contains("fn inner()"));
    }

    #[test]
    fn chunk_by_decls_only_attr_lines_no_decls_returns_single_chunk() {
        let content = "#[derive(Debug)]\n#[allow(dead_code)]";
        let result = chunk_by_declarations(content, &["fn "], |l| l.starts_with("#["));
        assert_eq!(result.len(), 1, "attr-only content must not be empty: {:?}", result);
        assert!(result[0].contains("#[derive(Debug)]"));
    }

    // --- is_decl_boundary ---

    #[test]
    fn is_decl_boundary_fn_at_col0_returns_true() {
        assert!(is_decl_boundary("fn foo()", &["fn "]));
    }

    #[test]
    fn is_decl_boundary_async_fn_returns_true() {
        assert!(is_decl_boundary("async fn foo()", &["fn "]));
    }

    #[test]
    fn is_decl_boundary_pub_fn_returns_true() {
        assert!(is_decl_boundary("pub fn foo()", &["fn "]));
    }

    #[test]
    fn is_decl_boundary_pub_async_fn_returns_true() {
        assert!(is_decl_boundary("pub async fn foo()", &["fn "]));
    }

    #[test]
    fn is_decl_boundary_pub_crate_async_fn_returns_true() {
        assert!(is_decl_boundary("pub(crate) async fn foo()", &["fn "]));
    }

    #[test]
    fn is_decl_boundary_pub_super_fn_returns_true() {
        assert!(is_decl_boundary("pub(super) fn foo()", &["fn "]));
    }

    #[test]
    fn is_decl_boundary_unsafe_fn_returns_true() {
        assert!(is_decl_boundary("unsafe fn foo()", &["fn "]));
    }

    #[test]
    fn is_decl_boundary_unsafe_async_fn_returns_true() {
        assert!(is_decl_boundary("unsafe async fn foo()", &["fn "]));
    }

    #[test]
    fn is_decl_boundary_struct_returns_true() {
        assert!(is_decl_boundary("struct Foo", &["fn ", "struct "]));
    }

    #[test]
    fn is_decl_boundary_enum_returns_true() {
        assert!(is_decl_boundary("enum Bar", &["fn ", "enum "]));
    }

    #[test]
    fn is_decl_boundary_impl_returns_true() {
        assert!(is_decl_boundary("impl Foo", &["fn ", "impl "]));
    }

    #[test]
    fn is_decl_boundary_indented_fn_returns_false() {
        assert!(!is_decl_boundary("    fn foo()", &["fn "]));
    }

    #[test]
    fn is_decl_boundary_comment_returns_false() {
        assert!(!is_decl_boundary("// fn foo()", &["fn "]));
    }

    #[test]
    fn is_decl_boundary_let_stmt_returns_false() {
        assert!(!is_decl_boundary("let x = fn_call()", &["fn "]));
    }

    // --- chunk_rust ---

    #[test]
    fn chunk_rust_fn_at_col0_splits() {
        let chunks = chunk_rust("fn foo() {}\n\nfn bar() {}");
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].contains("fn foo()"));
        assert!(chunks[1].contains("fn bar()"));
    }

    #[test]
    fn chunk_rust_pub_fn_splits() {
        let chunks = chunk_rust("pub fn foo() {}\n\npub fn bar() {}");
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn chunk_rust_async_fn_splits() {
        let chunks = chunk_rust("async fn foo() {}\n\nfn bar() {}");
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn chunk_rust_pub_async_fn_splits() {
        let chunks = chunk_rust("pub async fn foo() {}\n\nfn bar() {}");
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn chunk_rust_indented_fn_does_not_split() {
        let content = "impl Foo {\n    fn method(&self) {}\n    fn other(&self) {}\n}";
        let chunks = chunk_rust(content);
        assert_eq!(chunks.len(), 1, "indented fn should not split: {:?}", chunks);
    }

    #[test]
    fn chunk_rust_impl_splits() {
        let content = "struct Foo {}\n\nimpl Foo {\n    fn foo(&self) {}\n}";
        let chunks = chunk_rust(content);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].contains("struct Foo"));
        assert!(chunks[1].contains("impl Foo"));
    }

    #[test]
    fn chunk_rust_impl_generic_splits() {
        let content = "struct Foo<T>(T);\n\nimpl<T: std::fmt::Debug> Foo<T> {\n    fn foo(&self) {}\n}";
        let chunks = chunk_rust(content);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[1].contains("impl<T:"), "got: {}", chunks[1]);
    }

    #[test]
    fn chunk_rust_derive_attr_stays_with_item() {
        let content = "struct Foo { x: i32 }\n\n#[derive(Debug)]\npub enum Bar { A }";
        let chunks = chunk_rust(content);
        assert_eq!(chunks.len(), 2);
        assert!(
            !chunks[0].contains("#[derive"),
            "derive must not be in Foo chunk: {}",
            chunks[0]
        );
        assert!(
            chunks[1].contains("#[derive"),
            "derive must be in Bar chunk: {}",
            chunks[1]
        );
        assert!(chunks[1].contains("pub enum Bar"));
    }

    #[test]
    fn chunk_rust_doc_comment_stays_with_fn() {
        let content = "fn foo() {}\n\n/// Does bar.\nfn bar() {}";
        let chunks = chunk_rust(content);
        assert_eq!(chunks.len(), 2);
        assert!(
            !chunks[0].contains("/// Does bar."),
            "doc must not be in foo chunk: {}",
            chunks[0]
        );
        assert!(
            chunks[1].contains("/// Does bar."),
            "doc must be in bar chunk: {}",
            chunks[1]
        );
    }

    #[test]
    fn chunk_rust_struct_splits() {
        let chunks = chunk_rust("struct Foo { x: i32 }\n\nstruct Bar { y: i32 }");
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].contains("struct Foo"));
        assert!(chunks[1].contains("struct Bar"));
    }

    #[test]
    fn chunk_rust_trait_splits() {
        let chunks = chunk_rust("struct Foo {}\n\ntrait Greet {\n    fn hello(&self);\n}");
        assert_eq!(chunks.len(), 2);
        assert!(chunks[1].contains("trait Greet"));
    }

    #[test]
    fn chunk_rust_const_splits() {
        let chunks = chunk_rust("fn foo() {}\n\nconst MAX: usize = 100;");
        assert_eq!(chunks.len(), 2);
        assert!(chunks[1].contains("const MAX"));
    }

    #[test]
    fn chunk_rust_mod_splits() {
        let chunks = chunk_rust("fn foo() {}\n\nmod utils {\n    pub fn helper() {}\n}");
        assert_eq!(chunks.len(), 2);
        assert!(chunks[1].contains("mod utils"));
    }

    // --- chunk_allium ---

    #[test]
    fn chunk_allium_entity_and_rule_produce_two_chunks() {
        let content = "entity Task {\n    id: TaskId\n}\n\nrule CreateTask {\n    when: foo\n}";
        let chunks = chunk_allium(content);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].contains("entity Task"));
        assert!(chunks[1].contains("rule CreateTask"));
    }

    #[test]
    fn chunk_allium_section_comment_stays_with_rule() {
        let content = "entity Task {}\n\n-- == Creation ==\n\nrule CreateTask {}";
        let chunks = chunk_allium(content);
        assert_eq!(chunks.len(), 2);
        assert!(
            !chunks[0].contains("-- == Creation =="),
            "comment should not be in entity chunk: {}",
            chunks[0]
        );
        assert!(
            chunks[1].contains("-- == Creation =="),
            "comment should be in rule chunk: {}",
            chunks[1]
        );
    }

    #[test]
    fn chunk_allium_config_block_splits() {
        let content = "entity Task {}\n\nconfig {\n    key: value\n}";
        let chunks = chunk_allium(content);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[1].contains("config {"));
    }

    #[test]
    fn chunk_allium_enum_splits() {
        let content = "entity Task {}\n\nenum Status { active | done }";
        let chunks = chunk_allium(content);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[1].contains("enum Status"));
    }

    #[test]
    fn chunk_allium_surface_splits() {
        let content = "entity Task {}\n\nsurface KanbanBoard {\n    shows: Task\n}";
        let chunks = chunk_allium(content);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[1].contains("surface KanbanBoard"));
    }

    #[test]
    fn chunk_allium_value_splits() {
        let content = "entity Task {}\n\nvalue TrajectoryEntry {\n    id: u64\n}";
        let chunks = chunk_allium(content);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[1].contains("value TrajectoryEntry"));
    }

    // --- chunk_for_extension ---

    #[test]
    fn chunk_for_extension_md_uses_h2_splitting() {
        let content = "## Section A\n\nText A.\n\n## Section B\n\nText B.";
        let chunks = chunk_for_extension(content, "md");
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].contains("Section A"));
        assert!(chunks[1].contains("Section B"));
    }

    #[test]
    fn chunk_for_extension_rs_uses_declaration_splitting() {
        let chunks = chunk_for_extension("fn foo() {}\n\nfn bar() {}", "rs");
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn chunk_for_extension_allium_uses_allium_splitting() {
        let chunks = chunk_for_extension("entity Task {}\n\nrule CreateTask {}", "allium");
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn chunk_for_extension_unknown_ext_returns_single_chunk() {
        let chunks = chunk_for_extension("some content", "txt");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "some content");
    }

    #[test]
    fn chunk_for_extension_unknown_ext_empty_returns_no_chunks() {
        let chunks = chunk_for_extension("", "txt");
        assert!(chunks.is_empty());
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

        let results = svc.search_docs(dir.path(), "add integers", 5).await.unwrap();
        assert!(!results.is_empty(), "expected at least one result");
        assert!(results[0].file_path.ends_with("lib.rs"));
    }
}
