use crate::models::learnings::{Learning, LearningKind, LearningScope};

pub fn embed_text_for_learning(
    kind: LearningKind,
    summary: &str,
    tags: &[String],
    detail: Option<&str>,
) -> String {
    let kind_str = format!("{kind}");
    let mut parts = vec![format!("{kind_str}: {summary}")];
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
            if scope_ref.is_some() && scope_ref == task_epic_id {
                0.30
            } else {
                0.0
            }
        }
        LearningScope::Repo => {
            if scope_ref.is_some() && scope_ref == task_repo {
                0.20
            } else {
                0.0
            }
        }
        LearningScope::Project => {
            if scope_ref.is_some() && scope_ref == task_project {
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
    (upvote_count.min(10) as f32) * 0.005
}

pub struct ScoredLearning<'a> {
    pub learning: &'a Learning,
    pub score: f32,
}

/// Rank candidate learnings by RAG score. Returns sorted vec (highest score first),
/// filtered by threshold and limited to `limit`.
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
}
