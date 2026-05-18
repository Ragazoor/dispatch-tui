use std::sync::Arc;

use anyhow::Result;
use tokio::sync::oneshot;

use crate::models::learnings::{Learning, LearningKind, LearningScope};

// ---------------------------------------------------------------------------
// EmbeddingService — dedicated OS thread owns the fastembed model
// ---------------------------------------------------------------------------

struct EmbedSingle {
    // In test mode the stub thread ignores text, but the field is populated by callers.
    #[allow(dead_code)]
    text: String,
    reply: oneshot::Sender<Result<Vec<f32>>>,
}

struct EmbedBatch {
    texts: Vec<String>,
    reply: oneshot::Sender<Result<Vec<Vec<f32>>>>,
}

enum EmbedMsg {
    Single(EmbedSingle),
    Batch(EmbedBatch),
}

#[derive(Clone)]
pub struct EmbeddingService {
    tx: std::sync::mpsc::Sender<EmbedMsg>,
}

impl EmbeddingService {
    /// Initialise with the real fastembed model. Blocks until model is loaded.
    /// Call at startup before the TUI opens.
    #[cfg(not(test))]
    pub fn new() -> Result<Arc<Self>> {
        use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
        let mut model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::BGESmallENV15).with_show_download_progress(true),
        )?;
        let (tx, rx) = std::sync::mpsc::channel::<EmbedMsg>();
        std::thread::spawn(move || {
            while let Ok(msg) = rx.recv() {
                match msg {
                    EmbedMsg::Single(EmbedSingle { text, reply }) => {
                        let result = model
                            .embed(vec![text.as_str()], None)
                            .map(|mut vecs| vecs.remove(0))
                            .map_err(anyhow::Error::from);
                        let _ = reply.send(result);
                    }
                    EmbedMsg::Batch(EmbedBatch { texts, reply }) => {
                        let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
                        let result = model.embed(refs, None).map_err(anyhow::Error::from);
                        let _ = reply.send(result);
                    }
                }
            }
        });
        Ok(Arc::new(Self { tx }))
    }

    /// Test stub — returns deterministic vec![0.1; 384] without loading the model.
    /// Integration tests (not `#[cfg(test)]`) use `new_noop` directly.
    #[cfg(test)]
    pub fn new_test() -> Arc<Self> {
        Self::new_noop()
    }

    /// Test-only stub that returns fixed 0.1 vectors. All production call sites use the real
    /// EmbeddingService. Must be `pub` so integration tests can access it.
    pub fn new_noop() -> Arc<Self> {
        let (tx, rx) = std::sync::mpsc::channel::<EmbedMsg>();
        std::thread::spawn(move || {
            while let Ok(msg) = rx.recv() {
                match msg {
                    EmbedMsg::Single(EmbedSingle { reply, .. }) => {
                        let _ = reply.send(Ok(vec![0.1f32; 384]));
                    }
                    EmbedMsg::Batch(EmbedBatch { texts, reply }) => {
                        let result = texts.iter().map(|_| vec![0.1f32; 384]).collect();
                        let _ = reply.send(Ok(result));
                    }
                }
            }
        });
        Arc::new(Self { tx })
    }

    pub async fn embed(&self, text: impl Into<String>) -> Result<Vec<f32>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(EmbedMsg::Single(EmbedSingle {
                text: text.into(),
                reply: reply_tx,
            }))
            .map_err(|e| anyhow::anyhow!("EmbeddingService channel closed: {e}"))?;
        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("EmbeddingService reply channel dropped"))?
    }

    pub async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(EmbedMsg::Batch(EmbedBatch {
                texts,
                reply: reply_tx,
            }))
            .map_err(|e| anyhow::anyhow!("EmbeddingService channel closed: {e}"))?;
        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("EmbeddingService reply channel dropped"))?
    }
}

pub fn embed_text_for_learning(
    kind: LearningKind,
    summary: &str,
    tags: &[String],
    detail: Option<&str>,
) -> String {
    let mut parts = vec![format!("{kind}: {summary}")];
    if !tags.is_empty() {
        parts.push(tags.join(", "));
    }
    if let Some(d) = detail {
        parts.push(d.to_string());
    }
    parts.join("\n")
}

pub fn embed_text_for_query(title: &str, description: &str) -> String {
    format!("{title}\n{description}")
}

pub fn serialize_embedding(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    out.extend(v.iter().flat_map(|f| f.to_le_bytes()));
    out
}

pub fn deserialize_embedding(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

/// Returns scope multiplier for a learning given current task context.
/// task_epic_id / task_repo / task_project are the current task's values.
pub fn scope_multiplier_for(
    scope: LearningScope,
    scope_ref: Option<&str>,
    task_epic_id: Option<&str>,
    task_repo: Option<&str>,
    task_project: Option<&str>,
) -> f32 {
    match scope {
        LearningScope::Epic => {
            if matches!((scope_ref, task_epic_id), (Some(a), Some(b)) if a == b) {
                0.30
            } else {
                0.0
            }
        }
        LearningScope::Repo => {
            if matches!((scope_ref, task_repo), (Some(a), Some(b)) if a == b) {
                0.20
            } else {
                0.0
            }
        }
        LearningScope::Project => {
            if matches!((scope_ref, task_project), (Some(a), Some(b)) if a == b) {
                0.10
            } else {
                0.0
            }
        }
        LearningScope::User => 0.10,
        LearningScope::Task => 0.0,
    }
}

pub fn upvote_boost(upvote_count: i64) -> f32 {
    (upvote_count.max(0).min(10) as f32) * 0.005
}

/// Minimum cosine similarity for a learning to be a RAG candidate.
/// Used by both dispatch injection and the `query_learnings` MCP tool.
pub const RAG_SIMILARITY_THRESHOLD: f32 = 0.25;

/// Decode raw embedding bytes returned by the DB into f32 vectors.
pub fn deserialize_candidate_rows(rows: Vec<(Learning, Vec<u8>)>) -> Vec<(Learning, Vec<f32>)> {
    rows.into_iter()
        .map(|(l, b)| (l, deserialize_embedding(&b)))
        .collect()
}

struct ScoredLearning<'a> {
    learning: &'a Learning,
    score: f32,
}

/// Rank candidate learnings by RAG score.
///
/// `candidates` must contain only approved learnings (status filtering is the caller's responsibility).
/// Returns sorted vec (highest score first), filtered by threshold and limited to `limit`.
pub fn rag_rank_learnings<'a>(
    candidates: &'a [(Learning, Vec<f32>)],
    query_vec: &[f32],
    task_epic_id: Option<&str>,
    task_repo: Option<&str>,
    task_project: Option<&str>,
    threshold: f32,
    tag_filter: &[String],
    limit: usize,
) -> Vec<&'a Learning> {
    let tag_set: std::collections::HashSet<&str> =
        tag_filter.iter().map(|s| s.as_str()).collect();
    let mut scored: Vec<ScoredLearning<'_>> = candidates
        .iter()
        .filter_map(|(learning, emb)| {
            let cosine = cosine_similarity(query_vec, emb);
            if cosine < threshold {
                return None;
            }
            let scope_mul = scope_multiplier_for(
                learning.scope,
                learning.scope_ref.as_deref(),
                task_epic_id,
                task_repo,
                task_project,
            );
            let tag_boost = if tag_set.is_empty() {
                0.0
            } else {
                let matches = learning
                    .tags
                    .iter()
                    .filter(|t| tag_set.contains(t.as_str()))
                    .count();
                matches as f32 * 0.05
            };
            let score =
                cosine * (1.0 + scope_mul) + upvote_boost(learning.upvote_count) + tag_boost;
            Some(ScoredLearning { learning, score })
        })
        .collect();

    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().take(limit).map(|s| s.learning).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn embedding_service_returns_384_dims() {
        let svc = EmbeddingService::new_test();
        let result = svc.embed("hello world").await.unwrap();
        assert_eq!(result.len(), 384);
    }

    #[tokio::test]
    async fn embedding_service_batch_returns_correct_count() {
        let svc = EmbeddingService::new_test();
        let texts = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let result = svc.embed_batch(texts).await.unwrap();
        assert_eq!(result.len(), 3);
        assert!(result.iter().all(|v| v.len() == 384));
    }

    #[tokio::test]
    async fn embedding_service_concurrent_calls() {
        let svc = EmbeddingService::new_test();
        let svc2 = svc.clone();
        let (r1, r2) = tokio::join!(svc.embed("first"), svc2.embed("second"),);
        assert_eq!(r1.unwrap().len(), 384);
        assert_eq!(r2.unwrap().len(), 384);
    }
    use crate::models::{LearningId, LearningKind, LearningScope, LearningStatus};
    use chrono::{TimeZone, Utc};

    fn make_test_learning(id: i64, scope: LearningScope, scope_ref: Option<&str>) -> Learning {
        Learning {
            id: LearningId(id),
            kind: LearningKind::Pitfall,
            summary: format!("learning {id}"),
            detail: None,
            scope,
            scope_ref: scope_ref.map(|s| s.to_string()),
            tags: vec![],
            status: LearningStatus::Approved,
            source_task_id: None,
            upvote_count: 0,
            last_upvoted_at: None,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        }
    }

    #[test]
    fn cosine_identical_vectors() {
        let v = vec![1.0f32, 0.0, 0.0];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal_vectors() {
        let a = vec![1.0f32, 0.0];
        let b = vec![0.0f32, 1.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn serialize_roundtrip() {
        let v: Vec<f32> = vec![1.0, -0.5, 0.25];
        let bytes = serialize_embedding(&v);
        let back = deserialize_embedding(&bytes);
        for (a, b) in v.iter().zip(back.iter()) {
            assert!((a - b).abs() < 1e-7);
        }
    }

    #[test]
    fn embed_text_for_learning_includes_kind_summary_tags_detail() {
        use crate::models::learnings::LearningKind;
        let text = embed_text_for_learning(
            LearningKind::Convention,
            "prefer snake_case",
            &["rust".to_string(), "style".to_string()],
            Some("always use snake_case for identifiers"),
        );
        assert!(text.contains("convention"));
        assert!(text.contains("prefer snake_case"));
        assert!(text.contains("rust"));
        assert!(text.contains("style"));
        assert!(text.contains("always use snake_case"));
    }

    #[test]
    fn embed_text_for_learning_omits_empty_tags_and_none_detail() {
        use crate::models::learnings::LearningKind;
        let text = embed_text_for_learning(LearningKind::Pitfall, "avoid X", &[], None);
        assert!(text.contains("pitfall"));
        assert!(text.contains("avoid X"));
    }

    #[test]
    fn scope_multiplier_does_not_let_low_similarity_beat_high() {
        // cosine=0.26 * (1+0.30) < cosine=0.55 * (1+0.10)
        let low_cos = 0.26f32;
        let low_boost = 0.30f32;
        let high_cos = 0.55f32;
        let high_boost = 0.10f32;
        assert!(low_cos * (1.0 + low_boost) < high_cos * (1.0 + high_boost));
    }

    #[test]
    fn rag_rank_learnings_orders_by_score() {
        // high_sim: User scope, cosine≈1.0 (query nearly identical to embedding)
        let high_sim_learning = make_test_learning(1, LearningScope::User, None);
        // low_sim: Repo scope with matching repo, cosine≈0.26 (scope boost won't overcome gap)
        let low_sim_learning = make_test_learning(2, LearningScope::Repo, Some("my-repo"));

        // query vec
        let query = vec![1.0f32, 0.0, 0.0];
        // high_sim_emb has cosine=1.0 with query (same direction)
        let high_sim_emb = vec![1.0f32, 0.0, 0.0];
        // low_sim_emb has cosine≈0.26 with query (mostly orthogonal, small component along query)
        // [1, 0, 0] · [0.26, 0.97, 0] = 0.26; norms = 1.0 * 1.0; cosine ≈ 0.26
        let low_sim_emb = vec![0.26f32, 0.97f32, 0.0f32];

        let candidates = vec![
            (high_sim_learning, high_sim_emb),
            (low_sim_learning, low_sim_emb),
        ];
        let results = rag_rank_learnings(
            &candidates,
            &query,
            None,
            Some("my-repo"),
            None,
            0.0,
            &[],
            10,
        );
        // high_sim: cosine=1.0, User scope_mul=0.10 → score=1.0*(1.10)=1.10
        // low_sim: cosine≈0.26, Repo+match scope_mul=0.20 → score≈0.26*(1.20)≈0.31
        // Verify high-cosine candidate ranks first
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, LearningId(1), "high cosine should rank first");
        assert_eq!(results[1].id, LearningId(2), "low cosine should rank second");
    }
}
