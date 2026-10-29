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
        tables.learning_verdicts.retain(|_, v| v.task_id != task_id);
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
        if !self.delete_learning_cascade(&mut tables, id) {
            return Ok(ReducerOutcome::Refused(format!("learning {id} not found")));
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    /// The learning row and every retrieval and verdict that names it, as the
    /// module's `delete_learning` removes them. False when there was no such
    /// learning.
    pub(super) fn delete_learning_cascade(&self, tables: &mut Tables, id: i64) -> bool {
        if !self.delete_learning_row(tables, id) {
            return false;
        }
        let retrievals: Vec<i64> = tables
            .learning_retrievals
            .values()
            .filter(|r| r.learning_id == id)
            .map(|r| r.id)
            .collect();
        self.delete_learning_retrieval_rows(tables, retrievals);
        tables.learning_verdicts.retain(|_, v| v.learning_id != id);
        true
    }

    /// Mirrors the module's `rescope_learnings`, callable from another
    /// reducer's transaction.
    pub(super) fn rescope_learnings(&self, tables: &mut Tables, from: i64, to: i64) {
        let from_ref = from.to_string();
        let to_ref = to.to_string();
        self.update_matching_learnings(
            tables,
            |l| l.scope == "epic" && l.scope_ref.as_deref() == Some(from_ref.as_str()),
            |row| module::Learning {
                scope_ref: Some(to_ref.clone()),
                ..row
            },
        );
    }

    pub(super) fn apply_rescope_epic_learnings(
        &self,
        from: EpicId,
        to: EpicId,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        self.rescope_learnings(&mut tables, from.0, to.0);
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

    /// Mirrors the module's `apply_learning_verdicts`, which owns the rule: the
    /// weights and the score change are the module's own `verdict_weight` and
    /// `verdict_score_change`, and the refusal text is its
    /// `not_retrieved_message`.
    ///
    /// A real reducer's `Err` rolls back every mutation the transaction already
    /// made, so this validates before it writes: every entry needs a retrieval
    /// for `task_id` first (a refusal for the batch), and the second pass
    /// works on a copy of the verdict table and the learnings it touches,
    /// installed only once every entry has applied. Entries naming one learning
    /// twice apply in order against each other's result, as the module's
    /// re-reads do.
    pub(super) fn apply_apply_learning_verdicts(
        &self,
        task_id: TaskId,
        verdicts: Vec<(LearningId, LearningVerdict)>,
    ) -> Result<ReducerOutcome> {
        let task_id = task_id.0;
        let mut tables = self.lock();
        for (learning_id, _) in &verdicts {
            let retrieved = tables
                .learning_retrievals
                .values()
                .any(|r| r.task_id == task_id && r.learning_id == learning_id.0);
            if !retrieved {
                return Ok(ReducerOutcome::Refused(module::not_retrieved_message(
                    learning_id.0,
                    task_id,
                )));
            }
        }
        let now = self.now();
        let mut standing = tables.learning_verdicts.clone();
        let mut learnings = tables.learnings.clone();
        let mut next_id = tables.next_learning_verdict_id;
        let mut touched = Vec::new();
        for (learning_id, verdict) in &verdicts {
            let learning_id = learning_id.0;
            let Some(row) = learnings.get(&learning_id).cloned() else {
                continue;
            };
            let verdict = verdict.as_str();
            let previous = standing
                .values()
                .find(|s| s.task_id == task_id && s.learning_id == learning_id)
                .cloned();
            let delta = module::verdict_score_change(
                previous.as_ref().map(|s| s.verdict.as_str()),
                verdict,
            )
            .map_err(anyhow::Error::msg)?;
            match previous {
                Some(s) => {
                    standing.insert(
                        s.id,
                        module::LearningVerdict {
                            verdict: verdict.to_string(),
                            ..s
                        },
                    );
                }
                None => {
                    let id = next_id;
                    next_id += 1;
                    standing.insert(
                        id,
                        module::LearningVerdict {
                            id,
                            task_id,
                            learning_id,
                            verdict: verdict.to_string(),
                        },
                    );
                }
            }
            if delta == 0 {
                continue;
            }
            let last_upvoted_at = if verdict == "helped" {
                Some(now.clone())
            } else {
                row.last_upvoted_at.clone()
            };
            learnings.insert(
                learning_id,
                module::Learning {
                    upvote_count: row.upvote_count + delta,
                    last_upvoted_at,
                    updated_at: now.clone(),
                    ..row
                },
            );
            touched.push(learning_id);
        }
        tables.learning_verdicts = standing;
        tables.next_learning_verdict_id = next_id;
        for learning_id in touched {
            if let Some(updated) = learnings.get(&learning_id).cloned() {
                tables.learnings.insert(learning_id, updated.clone());
                self.rows.upsert_learning(&updated.into());
            }
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
