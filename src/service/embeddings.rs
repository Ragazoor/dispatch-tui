use std::sync::Arc;

use anyhow::Result;
use tokio::sync::oneshot;

use crate::models::learnings::{Learning, LearningKind, LearningScope};

// ---------------------------------------------------------------------------
// EmbeddingService — dedicated OS thread owns the fastembed model
// ---------------------------------------------------------------------------

struct EmbedRequest {
    // In test mode the stub thread ignores text, but the field is populated by callers.
    #[allow(dead_code)]
    text: String,
    reply: oneshot::Sender<Result<Vec<f32>>>,
}

#[derive(Clone)]
pub struct EmbeddingService {
    tx: std::sync::mpsc::Sender<EmbedRequest>,
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
        let (tx, rx) = std::sync::mpsc::channel::<EmbedRequest>();
        std::thread::spawn(move || {
            while let Ok(req) = rx.recv() {
                let result = model
                    .embed(vec![req.text.as_str()], None)
                    .map(|mut vecs| vecs.remove(0))
                    .map_err(anyhow::Error::from);
                let _ = req.reply.send(result);
            }
        });
        Ok(Arc::new(Self { tx }))
    }

    /// Test stub — returns deterministic vec![0.1; 384] without loading the model.
    #[cfg(test)]
    pub fn new_test() -> Arc<Self> {
        let (tx, rx) = std::sync::mpsc::channel::<EmbedRequest>();
        std::thread::spawn(move || {
            while let Ok(req) = rx.recv() {
                let _ = req.reply.send(Ok(vec![0.1f32; 384]));
            }
        });
        Arc::new(Self { tx })
    }

    /// Placeholder stub for call sites not yet wired in Tasks 8/9.
    ///
    /// In test builds: returns `vec![0.1f32; 384]` so existing tests that
    /// incidentally call `create_learning` or `update_learning` continue to pass.
    ///
    /// In production builds: returns an error, causing the service call to fail
    /// with a clear message prompting the caller to wire the real service.
    pub(crate) fn new_noop() -> Arc<Self> {
        #[cfg(test)]
        let thread_fn = |rx: std::sync::mpsc::Receiver<EmbedRequest>| {
            std::thread::spawn(move || {
                while let Ok(req) = rx.recv() {
                    let _ = req.reply.send(Ok(vec![0.1f32; 384]));
                }
            });
        };
        #[cfg(not(test))]
        let thread_fn = |rx: std::sync::mpsc::Receiver<EmbedRequest>| {
            std::thread::spawn(move || {
                while let Ok(req) = rx.recv() {
                    let _ = req.reply.send(Err(anyhow::anyhow!(
                        "EmbeddingService not yet wired — call site must be updated in Task 8/9"
                    )));
                }
            });
        };
        let (tx, rx) = std::sync::mpsc::channel::<EmbedRequest>();
        thread_fn(rx);
        Arc::new(Self { tx })
    }

    pub async fn embed(&self, text: impl Into<String>) -> Result<Vec<f32>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(EmbedRequest {
                text: text.into(),
                reply: reply_tx,
            })
            .map_err(|e| anyhow::anyhow!("EmbeddingService channel closed: {e}"))?;
        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("EmbeddingService reply channel dropped"))?
    }

    pub async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            results.push(self.embed(text).await?);
        }
        Ok(results)
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
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
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

pub(crate) struct ScoredLearning<'a> {
    pub(crate) learning: &'a Learning,
    pub(crate) score: f32,
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
            let tag_boost = if tag_filter.is_empty() {
                0.0
            } else {
                let matches = learning
                    .tags
                    .iter()
                    .filter(|t| tag_filter.contains(t))
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
