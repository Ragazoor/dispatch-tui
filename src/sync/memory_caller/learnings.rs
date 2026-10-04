//! Learnings, retrievals and verdicts.

use anyhow::Result;
use chrono::{DateTime, Utc};

use dispatch_spacetime_module as module;

use crate::models::{EpicId, LearningId, LearningVerdict, RetrievalSource, TaskId};
use crate::spacetime::bindings;

use super::super::encode;
use super::super::writes::ReducerOutcome;
use super::{MemoryReducerCaller, Tables};

impl MemoryReducerCaller {
    /// `source_task_id` is not indexed here either — mirrors the module's own
    /// `detach_learnings_from_task`, which accepts the same full-table scan
    /// for the same reason (see that function's doc comment).
    pub(super) fn detach_learnings_from_task(&self, tables: &mut Tables, task_id: i64) {
        self.update_matching_learnings(
            tables,
            |l| l.source_task_id == Some(task_id),
            |row| module::Learning {
                source_task_id: None,
                ..row
            },
        );
        let retrievals: Vec<i64> = tables
            .learning_retrievals
            .values()
            .filter(|r| r.task_id == task_id)
            .map(|r| r.id)
            .collect();
        self.delete_learning_retrieval_rows(tables, retrievals);
    }

    /// Collect every learning matching `pred`, apply `f`, and write each
    /// back — mirrors the module's own `update_matching_learnings`. Shared by
    /// `detach_learnings_from_task`, `rescope_epic_learnings` and
    /// `archive_stale_learnings`.
    pub(super) fn update_matching_learnings(
        &self,
        tables: &mut Tables,
        pred: impl Fn(&module::Learning) -> bool,
        f: impl Fn(module::Learning) -> module::Learning,
    ) {
        let matching: Vec<module::Learning> = tables
            .learnings
            .values()
            .filter(|l| pred(l))
            .cloned()
            .collect();
        for row in matching {
            let updated = f(row);
            tables.learnings.insert(updated.id, updated.clone());
            self.rows.upsert_learning(&updated.into());
        }
    }

    /// Returns whether a row was actually removed, so a caller that needs to
    /// distinguish "removed" from "already absent" (`delete_learning`) can do
    /// so without a separate lookup of its own.
    pub(super) fn delete_learning_row(&self, tables: &mut Tables, id: i64) -> bool {
        let removed = tables.learnings.remove(&id).is_some();
        if removed {
            self.rows.remove_learning(LearningId(id));
        }
        removed
    }

    /// Delete each `learning_retrievals` row by id — mirrors the module's own
    /// `delete_learning_retrievals`. Shared by `detach_learnings_from_task`
    /// and `delete_learning`.
    pub(super) fn delete_learning_retrieval_rows(&self, tables: &mut Tables, ids: Vec<i64>) {
        for id in ids {
            if tables.learning_retrievals.remove(&id).is_some() {
                self.rows.remove_learning_retrieval(id);
            }
        }
    }

    pub(super) fn apply_create_learning(&self, row: bindings::Learning) -> Result<LearningId> {
        let row = module::Learning::from(row);
        module::validate_learning_scope(&row.scope, &row.scope_ref)
            // Same wording `SdkReducerCaller`'s real refusal path surfaces, so
            // a caller cannot tell the two backends apart from the error text
            // alone — see `create_task`'s matching comment.
            .map_err(|why| anyhow::anyhow!("the shared store refused: {why}"))?;
        let mut tables = self.lock();
        let id = Self::assign_id(0, &mut tables.next_learning_id);
        let row = module::Learning { id, ..row };
        tables.learnings.insert(id, row.clone());
        self.rows.upsert_learning(&row.into());
        Ok(LearningId(id))
    }

    pub(super) fn apply_patch_learning(
        &self,
        id: LearningId,
        patch: bindings::LearningPatch,
    ) -> Result<ReducerOutcome> {
        let id = id.0;
        let mut tables = self.lock();
        let Some(mut row) = tables.learnings.get(&id).cloned() else {
            return Ok(ReducerOutcome::Applied(vec![]));
        };
        module::apply_learning_patch(&mut row, module::LearningPatch::from(patch));
        row.updated_at = self.now();
        tables.learnings.insert(id, row.clone());
        self.rows.upsert_learning(&row.into());
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_delete_learning(&self, id: LearningId) -> Result<ReducerOutcome> {
        let id = id.0;
        let mut tables = self.lock();
        if !self.delete_learning_row(&mut tables, id) {
            return Ok(ReducerOutcome::Refused(format!("learning {id} not found")));
        }
        let retrievals: Vec<i64> = tables
            .learning_retrievals
            .values()
            .filter(|r| r.learning_id == id)
            .map(|r| r.id)
            .collect();
        self.delete_learning_retrieval_rows(&mut tables, retrievals);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_rescope_epic_learnings(
        &self,
        from: EpicId,
        to: EpicId,
    ) -> Result<ReducerOutcome> {
        let from = from.0;
        let to = to.0;
        let mut tables = self.lock();
        let from_ref = from.to_string();
        let to_ref = to.to_string();
        self.update_matching_learnings(
            &mut tables,
            |l| l.scope == "epic" && l.scope_ref.as_deref() == Some(from_ref.as_str()),
            |row| module::Learning {
                scope_ref: Some(to_ref.clone()),
                ..row
            },
        );
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_record_learning_retrieval(
        &self,
        task_id: TaskId,
        learning_id: LearningId,
        source: RetrievalSource,
    ) -> Result<ReducerOutcome> {
        let task_id = task_id.0;
        let learning_id = learning_id.0;
        let source = source.as_str().to_string();
        let mut tables = self.lock();
        let id = Self::assign_id(0, &mut tables.next_learning_retrieval_id);
        let row = module::LearningRetrieval {
            id,
            task_id,
            learning_id,
            source,
            retrieved_at: self.now(),
        };
        tables.learning_retrievals.insert(id, row.clone());
        self.rows.upsert_learning_retrieval(&row.into());
        Ok(ReducerOutcome::Applied(vec![]))
    }

    /// Mirrors the module's `apply_learning_verdicts`. Two passes rather than
    /// one, to reproduce a real reducer's atomicity: an unknown verdict for a
    /// learning that EXISTS refuses the whole batch (a real reducer's `Err`
    /// rolls back every mutation the transaction already made), so every
    /// existing learning's delta is validated before any of them is applied.
    /// Existence is checked before the verdict string, per entry, same order
    /// as the module — an unknown verdict tied to a MISSING learning id is a
    /// silent no-op for that entry, same as the module, not a batch refusal:
    /// the module's own `continue` never reaches its `match` for that entry.
    ///
    /// The second pass re-`get`s each row rather than carrying the clone
    /// pass 1 already made — NOT redundant, on purpose: the module re-reads
    /// via `ctx.db.learnings().id().find()` on every loop iteration too, so
    /// two entries naming the same `learning_id` apply in order against each
    /// other's result (read-your-own-writes within the one transaction), not
    /// both against the pre-batch row. Below,
    /// `apply_learning_verdicts_applies_duplicate_entries_for_the_same_id_in_order`
    /// pins this. The `Some` this re-`get` unwraps can never be `None` —
    /// nothing in this function removes a row — so the `else` is a defensive
    /// no-op, not a reachable path.
    pub(super) fn apply_apply_learning_verdicts(
        &self,
        verdicts: Vec<(LearningId, LearningVerdict)>,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let mut deltas = Vec::with_capacity(verdicts.len());
        for (learning_id, verdict) in &verdicts {
            let learning_id = learning_id.0;
            if !tables.learnings.contains_key(&learning_id) {
                continue;
            }
            let delta: i64 = match verdict {
                LearningVerdict::Helped => 1,
                LearningVerdict::Wrong => -1,
            };
            deltas.push((learning_id, delta));
        }
        let now = self.now();
        for (learning_id, delta) in deltas {
            let Some(row) = tables.learnings.get(&learning_id).cloned() else {
                continue;
            };
            let last_upvoted_at = if delta > 0 {
                Some(now.clone())
            } else {
                row.last_upvoted_at.clone()
            };
            let updated = module::Learning {
                upvote_count: row.upvote_count + delta,
                last_upvoted_at,
                updated_at: now.clone(),
                ..row
            };
            tables.learnings.insert(learning_id, updated.clone());
            self.rows.upsert_learning(&updated.into());
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_archive_stale_learnings(
        &self,
        cutoff: DateTime<Utc>,
    ) -> Result<ReducerOutcome> {
        let cutoff = encode::stamp(cutoff);
        let mut tables = self.lock();
        let now = self.now();
        self.update_matching_learnings(
            &mut tables,
            |l| l.status == "approved" && l.upvote_count <= 0 && l.updated_at <= cutoff,
            |row| module::Learning {
                status: "archived".to_string(),
                updated_at: now.clone(),
                ..row
            },
        );
        Ok(ReducerOutcome::Applied(vec![]))
    }
}
