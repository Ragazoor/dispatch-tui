//! Learnings, retrievals and verdicts.

use super::*;

/// `source_task_id` is not indexed — nothing else ever looks a learning up by
/// it, and a full scan on a task delete is the same cost class `rescope_epic_learnings`
/// already accepts for `scope_ref`.
pub(crate) fn detach_learnings_from_task(ctx: &ReducerContext, task_id: i64) {
    update_matching_learnings(
        ctx,
        |l| l.source_task_id == Some(task_id),
        |row| Learning {
            source_task_id: None,
            ..row
        },
    );
    let retrievals: Vec<i64> = ctx
        .db
        .learning_retrievals()
        .task_id()
        .filter(&task_id)
        .map(|r| r.id)
        .collect();
    delete_learning_retrievals(ctx, retrievals);
    let verdicts: Vec<i64> = ctx
        .db
        .learning_verdicts()
        .task_id()
        .filter(&task_id)
        .map(|v| v.id)
        .collect();
    delete_learning_verdicts(ctx, verdicts);
}

/// Collect every learning matching `pred`, apply `f`, and write each back.
/// Shared by every reducer that scans the whole table for a bulk update —
/// `detach_learnings_from_task`, `rescope_epic_learnings` and
/// `archive_stale_learnings` all had this collect-then-loop shape separately.
pub(crate) fn update_matching_learnings(
    ctx: &ReducerContext,
    pred: impl Fn(&Learning) -> bool,
    f: impl Fn(Learning) -> Learning,
) {
    let matching: Vec<Learning> = ctx.db.learnings().iter().filter(|l| pred(l)).collect();
    for row in matching {
        ctx.db.learnings().id().update(f(row));
    }
}

/// Delete each `learning_retrievals` row by id. Shared by
/// `detach_learnings_from_task` and `delete_learning`, whose retrieval
/// cascades otherwise repeated the identical loop.
pub(crate) fn delete_learning_retrievals(ctx: &ReducerContext, ids: impl IntoIterator<Item = i64>) {
    for id in ids {
        ctx.db.learning_retrievals().id().delete(id);
    }
}

/// Delete each `learning_verdicts` row by id, the verdict twin of
/// [`delete_learning_retrievals`].
pub(crate) fn delete_learning_verdicts(ctx: &ReducerContext, ids: impl IntoIterator<Item = i64>) {
    for id in ids {
        ctx.db.learning_verdicts().id().delete(id);
    }
}

// -- Learnings (Phase 10, task #4914) ---------------------------------------

/// The fields a learning patch may change. Deliberately narrow, mirroring
/// `db::LearningPatch<'a>`'s own doc comment: `embedding` is the only field a
/// production caller writes (the startup backfill), and `status` exists for
/// `ArchiveStaleLearning`'s bulk sweep below and for tests seeding
/// archived/rejected rows directly. There is no field-editing path for a
/// learning's content.
///
/// `embedding` is doubly optional (`Patch<Option<Vec<u8>>>`), the same shape
/// `TaskPatch::sort_order` uses: the row's own `embedding` column is itself
/// `Option<Vec<u8>>`, so "do not touch" and "clear the stored embedding" are
/// two different `None`s. In practice only `Some(Some(bytes))` is ever sent —
/// nothing clears an embedding back to absent.
#[derive(spacetimedb::SpacetimeType, Clone, Debug, Default)]
pub struct LearningPatch {
    pub status: Patch<String>,
    pub summary: Patch<String>,
    pub embedding: Patch<Option<Vec<u8>>>,
}

/// Apply a learning patch. See [`apply_task_patch`].
///
/// `pub`, like `apply_task_patch`/`apply_epic_patch`: `MemoryReducerCaller`
/// calls this directly rather than re-deriving it, per
/// `spacetime-memory-store.allium`'s `ReducerConformance` `@guidance`.
///
/// Answers whether the patch touched content. An embedding is derived, not
/// content: the startup repair pass writes it and must change nothing else
/// (`learnings.allium: RecordLearningViaMcp`), or it would postpone
/// `ArchiveStaleLearning`; so `updated_at` moves only when this is true.
pub fn apply_learning_patch(row: &mut Learning, patch: LearningPatch) -> bool {
    let content = patch.status.is_some() || patch.summary.is_some();
    apply_patch!(row, patch, status, summary, embedding);
    content
}

/// Create a learning. The row arrives with `id = 0`; the caller learns the
/// generated id by watching it arrive on its subscription, the same as
/// [`create_task`] — see that reducer's doc comment for why a reducer cannot
/// answer with it directly.
///
/// Enforces `docs/specs/core.allium`'s `Learning.ScopeRefConsistency`
/// server-side, on the same reasoning `write_task` enforces
/// `OwnerTracksUserBoardTask`: a client-side check is a courtesy to a
/// well-behaved caller, and a shared table has more than one.
#[spacetimedb::reducer]
pub fn create_learning(ctx: &ReducerContext, row: Learning) -> Result<(), String> {
    validate_learning_scope(&row.scope, &row.scope_ref)?;
    // The store stamps the author itself (`store-split.allium:
    // IdentitiesAreTheCallers`); the dead `workspace_id` takes its default.
    let row = Learning {
        workspace_id: 0,
        author: caller_identity(ctx),
        ..row
    };
    // An epic ref that names no epic would store a learning nothing can reach.
    if !epic_scope_ref_names_an_epic(ctx, &row) {
        return Err(format!(
            "{} {} does not exist",
            row.scope,
            row.scope_ref.as_deref().unwrap_or("")
        ));
    }
    ctx.db.learnings().insert(Learning {
        // Never trust an incoming id on a create — see `create_task`.
        id: 0,
        ..row
    });
    Ok(())
}

/// False only for an epic-scoped learning whose `scope_ref` names no epic in
/// `r` (an unknown id, or text that is no id): `learnings.allium:
/// RecordLearningViaMcp`. `pub` so `MemoryReducerCaller` shares it.
pub fn epic_scope_ref_names_an_epic(r: &impl StoreReads, row: &Learning) -> bool {
    row.scope != "epic"
        || row
            .scope_ref
            .as_deref()
            .and_then(|s| s.parse::<i64>().ok())
            .is_some_and(|id| r.epic(id).is_some())
}

/// `pub`, like `validate_task_ownership`: `MemoryReducerCaller`'s
/// `create_learning` calls this directly rather than re-deriving it, per
/// `spacetime-memory-store.allium`'s `ReducerConformance` `@guidance`.
pub fn validate_learning_scope(scope: &str, scope_ref: &Option<String>) -> Result<(), String> {
    match (scope, scope_ref) {
        // `learnings.allium: RecordLearningViaMcp`. The scope is gone; every
        // write path comes through here, so none stores it.
        ("task", _) => Err(
            "scope=task is not supported; record the learning with scope=repo or scope=epic"
                .to_string(),
        ),
        ("user", Some(_)) => Err("a user-scoped learning must not carry a scope_ref".to_string()),
        ("user", None) => Ok(()),
        (_, None) => Err(format!("a {scope}-scoped learning needs a scope_ref")),
        // A blank repo ref covers every repo (`repo_ref_covers`).
        ("repo", Some(r)) if r.trim().is_empty() => {
            Err("a repo-scoped learning needs a non-blank scope_ref".to_string())
        }
        (_, Some(_)) => Ok(()),
    }
}

/// What the one-time retire step makes of a learning recorded under the removed
/// task scope: archived, so no retrieval returns it, and re-scoped to repo
/// (the recording task's repo, `""` when that task is gone) so the row stays a
/// scope every reader parses. `None` for any other row.
/// Pure, so the tests can pin it.
pub fn retired_task_scoped_learning(
    row: &Learning,
    source_repo: Option<String>,
) -> Option<Learning> {
    (row.scope == "task").then(|| Learning {
        scope: "repo".to_string(),
        scope_ref: Some(source_repo.unwrap_or_default()),
        status: "archived".to_string(),
        ..row.clone()
    })
}

/// `learnings.allium: Scope Model`: archive every learning recorded under the
/// removed task scope. Run once by `stamp_rules_version_on_connect`
/// (`TASK_SCOPE_REMOVED_RULES_VERSION`).
pub fn retire_task_scoped_learnings(ctx: &ReducerContext) {
    let rows: Vec<Learning> = ctx
        .db
        .learnings()
        .iter()
        .filter(|l| l.scope == "task")
        .collect();
    for row in rows {
        if let Some(retired) = retire_task_scoped_learning(ctx, &row) {
            ctx.db.learnings().id().update(retired);
        }
    }
}

/// The retire step for one row, read against the tasks and epics stored now:
/// the migration above and `seed_learnings` both call it, so a restored or
/// imported task-scoped learning lands as a migrated one does
/// (`spacetime-seed.allium: SeedsNormaliseLikeMigrations`). `None` for any
/// other scope.
pub fn retire_task_scoped_learning(ctx: &ReducerContext, row: &Learning) -> Option<Learning> {
    if row.scope != "task" {
        return None;
    }
    let repo = row
        .scope_ref
        .as_deref()
        .and_then(|r| r.parse::<i64>().ok())
        .and_then(|id| ctx.db.tasks().id().find(id))
        .map(|t| t.repo_path);
    retired_task_scoped_learning(row, repo)
}

/// Change some fields of a learning. A missing learning is a silent no-op,
/// the same bargain [`patch_task`] makes.
#[spacetimedb::reducer]
pub fn patch_learning(ctx: &ReducerContext, id: i64, patch: LearningPatch) -> Result<(), String> {
    let Some(mut row) = ctx.db.learnings().id().find(id) else {
        return Ok(());
    };
    if apply_learning_patch(&mut row, patch) {
        row.updated_at = now(ctx);
    }
    ctx.db.learnings().id().update(row);
    Ok(())
}

/// Delete a learning and the retrieval rows that only referred to it.
///
/// Unlike [`delete_task`], a missing id is refused rather than a silent
/// no-op: `DeleteLearningViaMcp` (`docs/specs/learnings.allium`) returns an
/// error for an id that was never created or was already deleted, and the
/// service layer's not-found mapping depends on that refusal reaching it.
/// The retrieval cascade runs in the same transaction as the delete,
/// reproduced as explicit reducer logic rather than a SQLite
/// `ON DELETE CASCADE` clause.
#[spacetimedb::reducer]
pub fn delete_learning(ctx: &ReducerContext, id: i64) -> Result<(), String> {
    if ctx.db.learnings().id().find(id).is_none() {
        return Err(format!("learning {id} not found"));
    }
    delete_learning_rows(ctx, id);
    Ok(())
}

/// The learning row and what only referred to it.
pub(crate) fn delete_learning_rows(ctx: &ReducerContext, id: i64) {
    ctx.db.learnings().id().delete(id);
    let retrievals: Vec<i64> = ctx
        .db
        .learning_retrievals()
        .learning_id()
        .filter(&id)
        .map(|r| r.id)
        .collect();
    delete_learning_retrievals(ctx, retrievals);
    let verdicts: Vec<i64> = ctx
        .db
        .learning_verdicts()
        .learning_id()
        .filter(&id)
        .map(|v| v.id)
        .collect();
    delete_learning_verdicts(ctx, verdicts);
}

/// Re-scope every epic-scoped learning pointing at `from` to `to` instead.
///
/// Epic-shaped arguments, a `learnings` write — see `docs/conventions.md`'s
/// store seam section for why this sits here rather than being folded into
/// an epic reducer, and `ReScopeLearningsOnRepoGroupDelete` in
/// `docs/specs/learnings.allium` for the rule this implements. `scope_ref` is
/// not an embedding input, so no row's `embedding` is touched.
#[spacetimedb::reducer]
pub fn rescope_epic_learnings(ctx: &ReducerContext, from: i64, to: i64) -> Result<(), String> {
    rescope_learnings(ctx, from, to);
    Ok(())
}

/// [`rescope_epic_learnings`]'s body, callable from another reducer's
/// transaction (a repo-group sub-epic's deletion re-scopes its learnings).
pub(crate) fn rescope_learnings(ctx: &ReducerContext, from: i64, to: i64) {
    let from_ref = from.to_string();
    let to_ref = to.to_string();
    update_matching_learnings(
        ctx,
        |l| l.scope == "epic" && l.scope_ref.as_deref() == Some(from_ref.as_str()),
        |row| Learning {
            scope_ref: Some(to_ref.clone()),
            ..row
        },
    );
}

/// Record that `learning_id` was surfaced to `task_id` via `source`.
#[spacetimedb::reducer]
pub fn record_learning_retrieval(
    ctx: &ReducerContext,
    task_id: i64,
    learning_id: i64,
    source: String,
) -> Result<(), String> {
    if ctx.db.tasks().id().find(task_id).is_none()
        || ctx.db.learnings().id().find(learning_id).is_none()
    {
        return Err(format!(
            "task {task_id} or learning {learning_id} does not exist"
        ));
    }
    ctx.db.learning_retrievals().insert(LearningRetrieval {
        task_id,
        learning_id,
        source,
        retrieved_at: now(ctx),
        ..blank_learning_retrieval()
    });
    Ok(())
}

/// One verdict in a batch — mirrors `db::LearningVerdict` (`helped` | `wrong` |
/// `not_applicable`)
/// as an opaque string, the same treatment every other enum-typed column in
/// this module gets.
#[derive(spacetimedb::SpacetimeType, Clone, Debug)]
pub struct LearningVerdictInput {
    pub learning_id: i64,
    pub verdict: String,
}

/// What one verdict is worth to `Learning.upvote_count`: `helped` +1, `wrong`
/// -1, `not_applicable` 0 (a measurement signal, not a ranking input). `pub`, like [`validate_learning_scope`], so `MemoryReducerCaller` shares
/// it rather than re-deriving it.
pub fn verdict_weight(verdict: &str) -> Result<i64, String> {
    match verdict {
        "helped" => Ok(1),
        "wrong" => Ok(-1),
        "not_applicable" => Ok(0),
        other => Err(format!("unknown verdict {other}")),
    }
}

/// How far a task's new verdict moves a learning's score: the new verdict's
/// weight less the weight of the verdict that task already stood behind, if
/// any. The same verdict again is 0.
pub fn verdict_score_change(previous: Option<&str>, verdict: &str) -> Result<i64, String> {
    let before = match previous {
        Some(prev) => verdict_weight(prev)?,
        None => 0,
    };
    Ok(verdict_weight(verdict)? - before)
}

/// The refusal for a verdict on a learning the task was never shown.
///
/// `pub` for the same reason as [`verdict_weight`]: the service maps it to a
/// validation error by this text.
pub fn not_retrieved_message(learning_id: i64, task_id: i64) -> String {
    format!("learning {learning_id} was not retrieved during task {task_id}")
}

/// Apply a batch of verdicts from `task_id`.
///
/// One verdict stands per (task, learning) — `learnings.allium:
/// RateLearningViaMcp`. A repeat replaces the stored verdict and moves
/// `Learning.upvote_count` by the difference, so a task cannot raise a score
/// by rating twice; entries naming one learning twice in a batch apply in
/// order. A verdict on a learning the task was never shown (no
/// `learning_retrievals` row) refuses the whole batch, as does an unknown
/// verdict string. A learning that does not exist is refused as "learning N not found" before the retrieval is checked.
/// A verdict that leaves the score where it was (a repeat, or a first
/// `not_applicable`) stores the verdict and does not write the learning, so it
/// cannot keep a stale entry alive.
#[spacetimedb::reducer]
pub fn apply_learning_verdicts(
    ctx: &ReducerContext,
    task_id: i64,
    verdicts: Vec<LearningVerdictInput>,
) -> Result<(), String> {
    if ctx.db.tasks().id().find(task_id).is_none() {
        return Err(format!("task {task_id} not found"));
    }
    for v in &verdicts {
        // A missing learning first, whether or not it was ever retrieved.
        if ctx.db.learnings().id().find(v.learning_id).is_none() {
            return Err(format!("learning {} not found", v.learning_id));
        }
        let retrieved = ctx
            .db
            .learning_retrievals()
            .task_id()
            .filter(&task_id)
            .any(|r| r.learning_id == v.learning_id);
        if !retrieved {
            return Err(not_retrieved_message(v.learning_id, task_id));
        }
    }
    for v in verdicts {
        let Some(row) = ctx.db.learnings().id().find(v.learning_id) else {
            continue;
        };
        let standing = ctx
            .db
            .learning_verdicts()
            .task_id()
            .filter(&task_id)
            .find(|s| s.learning_id == v.learning_id);
        let delta =
            verdict_score_change(standing.as_ref().map(|s| s.verdict.as_str()), &v.verdict)?;
        match standing {
            Some(s) => {
                ctx.db.learning_verdicts().id().update(LearningVerdict {
                    verdict: v.verdict.clone(),
                    ..s
                });
            }
            None => {
                ctx.db.learning_verdicts().insert(LearningVerdict {
                    task_id,
                    learning_id: v.learning_id,
                    verdict: v.verdict.clone(),
                    ..blank_learning_verdict()
                });
            }
        }
        if delta == 0 {
            continue;
        }
        let now = now(ctx);
        let last_upvoted_at = if v.verdict == "helped" {
            Some(now.clone())
        } else {
            row.last_upvoted_at.clone()
        };
        ctx.db.learnings().id().update(Learning {
            upvote_count: row.upvote_count + delta,
            last_upvoted_at,
            updated_at: now,
            ..row
        });
    }
    Ok(())
}

/// Archive every approved learning with a non-positive score that has gone
/// untouched since before `cutoff`. Mirrors the SQLite bulk `UPDATE`
/// `ArchiveStaleLearning` (`docs/specs/learnings.allium`) ran directly;
/// returns the number of rows archived.
#[spacetimedb::reducer]
pub fn archive_stale_learnings(ctx: &ReducerContext, cutoff: String) -> Result<(), String> {
    let now = now(ctx);
    update_matching_learnings(
        ctx,
        |l| l.status == "approved" && l.upvote_count <= 0 && l.updated_at <= cutoff,
        |row| Learning {
            status: "archived".to_string(),
            updated_at: now.clone(),
            ..row
        },
    );
    Ok(())
}
