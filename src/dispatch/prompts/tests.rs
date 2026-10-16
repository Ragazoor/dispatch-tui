use super::*;
use crate::models::{LearningKind, LearningScope};

fn seed(id: i64, scope: LearningScope, count: i64) -> Learning {
    use crate::models::{LearningId, LearningStatus};
    use chrono::{TimeZone, Utc};
    Learning {
        id: LearningId(id),
        kind: LearningKind::Pitfall,
        summary: format!("learning {id}"),
        detail: None,
        scope,
        scope_ref: match scope {
            LearningScope::User => None,
            _ => Some("ref".into()),
        },
        tags: vec![],
        status: LearningStatus::Approved,
        source_task_id: None,
        upvote_count: count,
        last_upvoted_at: None,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        updated_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    }
}

const AUTHOR_CLAIM: &str = "Do not re-check the PR author";

mod cve;
mod design_step;
mod knowledge;
mod preamble;
mod release_please;
mod review;
