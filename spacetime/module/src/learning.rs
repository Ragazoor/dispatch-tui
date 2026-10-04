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
pub fn apply_learning_patch(row: &mut Learning, patch: LearningPatch) {
    apply_patch!(row, patch, status, summary, embedding);
}

/// Create a learning. The row arrives with `id = 0`; the caller learns the
/// generated id by watching it arrive on its subscription, the same as
/// [`create_task`] — see that reducer's doc comment for why a reducer cannot
/// answer with it directly.
///
/// Enforces `docs/specs/learnings.allium`'s `ApprovedLearningsHaveScopeRef`
/// server-side, on the same reasoning `write_task` enforces
/// `OwnerTracksUserBoardTask`: a client-side check is a courtesy to a
/// well-behaved caller, and a shared table has more than one.
#[spacetimedb::reducer]
pub fn create_learning(ctx: &ReducerContext, row: Learning) -> Result<(), String> {
    validate_learning_scope(&row.scope, &row.scope_ref)?;
    ctx.db.learnings().insert(Learning {
        // Never trust an incoming id on a create — see `create_task`.
        id: 0,
        ..row
    });
    Ok(())
}

/// `pub`, like `validate_task_ownership`: `MemoryReducerCaller`'s
/// `create_learning` calls this directly rather than re-deriving it, per
/// `spacetime-memory-store.allium`'s `ReducerConformance` `@guidance`.
pub fn validate_learning_scope(scope: &str, scope_ref: &Option<String>) -> Result<(), String> {
    match (scope, scope_ref) {
        ("user", Some(_)) => Err("a user-scoped learning must not carry a scope_ref".to_string()),
        ("user", None) => Ok(()),
        (_, None) => Err(format!("a {scope}-scoped learning needs a scope_ref")),
        (_, Some(_)) => Ok(()),
    }
}

/// Change some fields of a learning. A missing learning is a silent no-op,
/// the same bargain [`patch_task`] makes.
#[spacetimedb::reducer]
pub fn patch_learning(ctx: &ReducerContext, id: i64, patch: LearningPatch) -> Result<(), String> {
    let Some(mut row) = ctx.db.learnings().id().find(id) else {
        return Ok(());
    };
    apply_learning_patch(&mut row, patch);
    row.updated_at = now(ctx);
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
    ctx.db.learnings().id().delete(id);
    let retrievals: Vec<i64> = ctx
        .db
        .learning_retrievals()
        .learning_id()
        .filter(&id)
        .map(|r| r.id)
        .collect();
    delete_learning_retrievals(ctx, retrievals);
    Ok(())
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
    Ok(())
}

/// Record that `learning_id` was surfaced to `task_id` via `source`.
#[spacetimedb::reducer]
pub fn record_learning_retrieval(
    ctx: &ReducerContext,
    task_id: i64,
    learning_id: i64,
    source: String,
) -> Result<(), String> {
    ctx.db.learning_retrievals().insert(LearningRetrieval {
        task_id,
        learning_id,
        source,
        retrieved_at: now(ctx),
        ..blank_learning_retrieval()
    });
    Ok(())
}

/// One verdict in a batch — mirrors `db::LearningVerdict` (`helped` | `wrong`)
/// as an opaque string, the same treatment every other enum-typed column in
/// this module gets.
#[derive(spacetimedb::SpacetimeType, Clone, Debug)]
pub struct LearningVerdictInput {
    pub learning_id: i64,
    pub verdict: String,
}

/// Apply a batch of verdicts' in-flight score effects.
///
/// The verdict itself is not persisted — migration v74 dropped
/// `learning_verdicts`, and a verdict has never been more than this effect on
/// `Learning.upvote_count` (`docs/specs/learnings.allium`'s Retrievals &
/// Verdicts). A missing learning is skipped rather than failing the whole
/// batch: the retrieval precondition that makes a verdict valid is enforced
/// client-side by `LearningServiceApi::apply_verdicts` before this is ever
/// called, so a batch here has already passed that check.
#[spacetimedb::reducer]
pub fn apply_learning_verdicts(
    ctx: &ReducerContext,
    verdicts: Vec<LearningVerdictInput>,
) -> Result<(), String> {
    for v in verdicts {
        let Some(row) = ctx.db.learnings().id().find(v.learning_id) else {
            continue;
        };
        let delta: i64 = match v.verdict.as_str() {
            "helped" => 1,
            "wrong" => -1,
            other => return Err(format!("unknown verdict {other}")),
        };
        let now = now(ctx);
        let last_upvoted_at = if delta > 0 {
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
