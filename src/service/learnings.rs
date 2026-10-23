use std::sync::{Arc, LazyLock};

use regex::Regex;

use crate::embeddings::{
    deserialize_candidate_rows, embed_text_for_learning, embed_text_for_query, rag_rank_learnings,
    serialize_embedding, EmbeddingService, RagRankParams, RAG_SIMILARITY_THRESHOLD,
};
use crate::models::{
    Learning, LearningId, LearningKind, LearningScope, LearningVerdict, RetrievalSource, TaskId,
};
use crate::store::{self, CreateLearningRow, LearningFilter};

use super::ServiceError;

// ---------------------------------------------------------------------------
// Internal-code citation detection
// ---------------------------------------------------------------------------

// Mirrors three of check-doc-symbols.sh's four candidate shapes (pathsym,
// typesym, bare) — see docs/specs/learnings.allium: RecordLearningViaMcp for
// why the fourth shape (bare backticked snake_case) is deliberately excluded.
//
// PATHSYM_RE is redundant as a *gate*: any text it matches also carries a `.rs`
// token, which the file-reference check rejects on its own. It is kept because
// it reports the whole citation as the offending substring, and an agent that
// is told `src/feed/cycle.rs::run_feed_cycle` fixes the right thing faster than
// one told `src/feed/cycle.rs`.
static PATHSYM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z0-9_./-]+\.rs::[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*")
        .unwrap_or_else(|e| unreachable!("PATHSYM_RE is a hardcoded pattern: {e}"))
});

static TYPESYM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Z][A-Za-z0-9]*(?:::[A-Za-z_][A-Za-z0-9_]*)+")
        .unwrap_or_else(|e| unreachable!("TYPESYM_RE is a hardcoded pattern: {e}"))
});

// At least four underscores (five word segments) — the same threshold
// check-doc-symbols.sh measured as the lowest value with zero false positives
// across the docs/specs/ corpus.
static BARE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[a-z][a-z0-9]*(?:_[a-z0-9]+){4,}")
        .unwrap_or_else(|e| unreachable!("BARE_RE is a hardcoded pattern: {e}"))
});

// A call site: an identifier immediately followed by empty parentheses. Prose
// does not produce this shape, so no carve-out is needed — not even for the
// MCP tool names the bare-identifier rule deliberately lets through.
static CALL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z_][A-Za-z0-9_]*\(\)")
        .unwrap_or_else(|e| unreachable!("CALL_RE is a hardcoded pattern: {e}"))
});

// A macro invocation, in two shapes. The underscore in the first is what
// separates `patch_struct!` from an exclamation mark in prose ("Never push to
// main!"); the second needs no underscore because the bracket is unambiguous.
//
// They stay two patterns because the regex crate has no lookaround, so the
// bracket has to be consumed rather than looked ahead at. Folding them into one
// alternation with an optional bracket would reject a bare `main!` in prose.
static MACRO_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[a-z][a-z0-9]*(?:_[a-z0-9]+)+!")
        .unwrap_or_else(|e| unreachable!("MACRO_RE is a hardcoded pattern: {e}"))
});

static MACRO_BRACKET_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[a-z_][a-z0-9_]*![\(\[\{]")
        .unwrap_or_else(|e| unreachable!("MACRO_BRACKET_RE is a hardcoded pattern: {e}"))
});

// A candidate file reference: a token carrying an extension, optionally with
// directory separators. Whether it is *rejected* depends on the two tests in
// `find_file_reference` — this only finds the candidates.
static FILEISH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z0-9_][A-Za-z0-9_./+-]*\.[A-Za-z][A-Za-z0-9]{0,9}")
        .unwrap_or_else(|e| unreachable!("FILEISH_RE is a hardcoded pattern: {e}"))
});

// Extensions that name a programming language. A bare filename with one of
// these is a source file wherever it lives, so it is rejected without needing
// a directory separator. Everything else — `md`, `json`, `toml`, `yaml`,
// `lock`, `allium` — is a document or a root manifest, and naming one does not
// rot the way a path into the source tree does.
const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "pyi", "scala", "sbt", "cs", "go", "java",
    "kt", "kts", "rb", "php", "swift", "cc", "cpp", "hpp", "sql", "sh", "bash", "zsh", "tf",
    "tfvars", "lua", "dart", "ex", "exs", "vue", "svelte",
];

/// Finds a file reference worth rejecting: a token with an extension that
/// either contains a directory separator (a path into the tree) or whose
/// extension names a programming language (a source file). A token reached
/// through a URL scheme, or carrying a `www.` prefix, is exempt — a link does
/// not rot the way a path does. A bare hostname with a path is not exempt, and
/// is rejected like any other path.
fn find_file_reference(text: &str) -> Option<&str> {
    FILEISH_RE.find_iter(text).find_map(|m| {
        let token = m.as_str();
        if token.starts_with("www.") || text[..m.start()].ends_with("//") {
            return None;
        }
        let is_path = token.contains('/');
        let is_source = token
            .rsplit_once('.')
            .is_some_and(|(_, ext)| SOURCE_EXTENSIONS.contains(&ext));
        (is_path || is_source).then_some(token)
    })
}

/// Detects an internal-code-shaped citation in learning text: a
// allow-phantom-symbol: describes the citation shape itself, not a real reference
/// `path.rs::symbol` reference, a `Type::method` reference, a long (5+
/// segment) bare snake_case identifier, a call with empty parentheses, a macro
/// invocation, or a source-file reference. Returns the offending substring on
/// a match. See docs/specs/learnings.allium: RecordLearningViaMcp for the
/// rationale, including why short bare identifiers (MCP tool names, and
/// PascalCase type names a regex cannot tell from a product name) are
/// deliberately left to the /learnings skill rather than flagged here.
fn find_code_citation(text: &str) -> Option<&str> {
    [&*PATHSYM_RE, &*TYPESYM_RE, &*BARE_RE, &*CALL_RE, &*MACRO_RE]
        .into_iter()
        .find_map(|re| re.find(text))
        .map(|m| m.as_str())
        // The bracket is matched so the pattern can find the macro without
        // lookahead, but it is not part of the offending name. It is always a
        // single ASCII character, so trimming it cannot split a code point.
        .or_else(|| {
            MACRO_BRACKET_RE
                .find(text)
                .map(|m| &text[m.start()..m.end() - 1])
        })
        .or_else(|| find_file_reference(text))
}

/// Rejects `field` (the "summary" or "detail" text of a learning) if it
/// contains an internal-code citation. See [`find_code_citation`].
fn reject_code_citation(field: &str, text: &str) -> Result<(), ServiceError> {
    if let Some(hit) = find_code_citation(text) {
        return Err(ServiceError::Validation(format!(
            "learning {field} names implementation detail (`{hit}`) — this rots silently \
             since nothing re-checks the knowledge base against the codebase. Describe the \
             durable behavior in prose instead, or add the citation to the relevant \
             docs/specs/*.allium file or a Rust doc comment, both of which \
             check-doc-symbols.sh keeps accurate on every push."
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// QueryLearningsParams
// ---------------------------------------------------------------------------

pub struct QueryLearningsParams {
    pub task_id: TaskId,
    /// Semantic query text. `None` derives it from the task's title + description.
    pub query: Option<String>,
    pub tag_filter: Vec<String>,
    pub limit: usize,
}

// ---------------------------------------------------------------------------
// CreateLearningParams
// ---------------------------------------------------------------------------

pub struct CreateLearningParams {
    pub kind: LearningKind,
    pub summary: String,
    pub detail: Option<String>,
    pub scope: LearningScope,
    pub scope_ref: Option<String>,
    pub tags: Vec<String>,
    pub source_task_id: Option<TaskId>,
}

/// One guard per `requires:` clause on `RecordLearningViaMcp`
/// (docs/specs/learnings.allium), in the order the spec states them. Keeping
/// them as peers rather than nesting the kind-specific rule inside the
/// citation check is what lets the next such rule land somewhere obvious.
fn validate_create_params(params: &CreateLearningParams) -> Result<(), ServiceError> {
    if params.summary.trim().is_empty() {
        return Err(ServiceError::Validation("summary must not be empty".into()));
    }

    match params.scope {
        LearningScope::User if params.scope_ref.is_some() => {
            return Err(ServiceError::Validation(
                "scope_ref must be null for user-scoped learnings".into(),
            ));
        }
        LearningScope::Repo | LearningScope::Epic | LearningScope::Task
            if params.scope_ref.is_none() =>
        {
            return Err(ServiceError::Validation(
                "scope_ref is required for non-user-scoped learnings".into(),
            ));
        }
        _ => {}
    }

    reject_code_citation("summary", &params.summary)?;
    if let Some(detail) = &params.detail {
        reject_code_citation("detail", detail)?;
    }

    // A procedural entry steers other agents, so it has to say where it stops
    // applying. Only the detail's presence is enforced; that it actually names
    // a boundary is a convention the /learnings skill carries.
    let has_detail = params.detail.as_ref().is_some_and(|d| !d.trim().is_empty());
    if params.kind == LearningKind::Procedural && !has_detail {
        return Err(ServiceError::Validation(
            "a procedural learning must carry a detail that says when to stop following \
             it and ask a human — an instruction with no boundary is not a guardrail"
                .into(),
        ));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// LearningService
// ---------------------------------------------------------------------------

pub struct LearningService {
    pub db: Arc<dyn store::TaskStore>,
    embedding_service: Arc<EmbeddingService>,
}

impl LearningService {
    pub fn new(db: Arc<dyn store::TaskStore>, embedding_service: Arc<EmbeddingService>) -> Self {
        Self {
            db,
            embedding_service,
        }
    }

    pub async fn create_learning(
        &self,
        params: CreateLearningParams,
    ) -> Result<LearningId, ServiceError> {
        validate_create_params(&params)?;
        let text = embed_text_for_learning(
            params.kind,
            &params.summary,
            &params.tags,
            params.detail.as_deref(),
        );
        let emb_vec = self
            .embedding_service
            .embed(text)
            .await
            .map_err(ServiceError::from)?;
        let emb_bytes = serialize_embedding(&emb_vec);
        self.db
            .create_learning(CreateLearningRow {
                kind: params.kind,
                summary: &params.summary,
                detail: params.detail.as_deref(),
                scope: params.scope,
                scope_ref: params.scope_ref.as_deref(),
                tags: &params.tags,
                source_task_id: params.source_task_id,
                embedding: Some(&emb_bytes),
            })
            .await
            .map_err(ServiceError::from)
    }

    pub async fn get_learning(&self, id: LearningId) -> Result<Learning, ServiceError> {
        self.db
            .get_learning(id)
            .await?
            .ok_or_else(|| ServiceError::NotFound(format!("learning {id} not found")))
    }

    pub async fn list_learnings(
        &self,
        filter: LearningFilter,
    ) -> Result<Vec<Learning>, ServiceError> {
        self.db
            .list_learnings(filter)
            .await
            .map_err(ServiceError::from)
    }

    pub async fn record_retrieval(
        &self,
        task_id: TaskId,
        learning_id: LearningId,
        source: RetrievalSource,
    ) -> Result<(), ServiceError> {
        self.db
            .record_retrieval(task_id, learning_id, source)
            .await
            .map_err(ServiceError::from)
    }

    /// Rate learnings for `task_id`. The rules (a retrieval for the task, one
    /// standing verdict per task and learning) live in the store's
    /// `apply_learning_verdicts` reducer; a refusal arrives as
    /// `ServiceError::Validation`. See docs/specs/learnings.allium:
    /// RateLearningViaMcp.
    pub async fn apply_verdicts(
        &self,
        task_id: TaskId,
        verdicts: Vec<(LearningId, LearningVerdict)>,
    ) -> Result<(), ServiceError> {
        if verdicts.is_empty() {
            return Ok(());
        }
        self.db
            .apply_verdicts_tx(task_id, &verdicts)
            .await
            .map_err(ServiceError::from)
    }

    /// Archive approved learnings that have gone stale with a non-positive score
    /// (upvote_count <= 0 and updated_at <= cutoff). Returns the number archived.
    /// See docs/specs/learnings.allium: ArchiveStaleLearning.
    pub async fn archive_stale_learnings(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64, ServiceError> {
        self.db
            .archive_stale_learnings(cutoff)
            .await
            .map_err(ServiceError::from)
    }

    pub async fn delete_learning(&self, id: LearningId) -> Result<(), ServiceError> {
        let deleted = self.db.delete_learning(id).await?;
        if !deleted {
            return Err(ServiceError::NotFound(format!("learning {id} not found")));
        }
        Ok(())
    }

    /// Rank approved learnings against a task's context and record a retrieval
    /// for each one returned, so `rate_learning` can later validate a verdict
    /// against what was actually surfaced. Owns the fetch→embed→rank→record
    /// orchestration so it is testable without going through JSON-RPC and is
    /// reachable from non-MCP callers (e.g. the TUI) if ever needed.
    pub async fn query_learnings(
        &self,
        params: QueryLearningsParams,
    ) -> Result<Vec<Learning>, ServiceError> {
        let task =
            self.db.get_task(params.task_id).await.ok_or_else(|| {
                ServiceError::NotFound(format!("task {} not found", params.task_id))
            })?;

        let query_text = params
            .query
            .unwrap_or_else(|| embed_text_for_query(&task.title, &task.description));
        let query_vec = self
            .embedding_service
            .embed(query_text)
            .await
            .map_err(ServiceError::from)?;

        let candidates_raw = self.db.list_all_approved_non_task_learnings().await?;
        let candidates = deserialize_candidate_rows(candidates_raw);

        let epic_id_str = task.epic_id.map(|e| e.0.to_string());
        let ranked = rag_rank_learnings(
            &candidates,
            &RagRankParams {
                query_vec: &query_vec,
                task_epic_id: epic_id_str.as_deref(),
                task_repo: Some(task.repo_path.as_str()),
                threshold: RAG_SIMILARITY_THRESHOLD,
                tag_filter: &params.tag_filter,
                limit: params.limit,
            },
        );

        for l in &ranked {
            if let Err(e) = self
                .db
                .record_retrieval(params.task_id, l.id, RetrievalSource::QueryLearnings)
                .await
            {
                tracing::warn!(
                    task_id = params.task_id.0,
                    learning_id = l.id.0,
                    error = ?e,
                    "failed to record learning retrieval"
                );
            }
        }

        Ok(ranked.into_iter().cloned().collect())
    }
}

// ---------------------------------------------------------------------------
// LearningService tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod learning_tests;
