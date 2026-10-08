//! Folding raw reducer answers into outcomes and generated ids, and matching
//! the rows a create just sent.

use anyhow::anyhow;

use crate::models::TaskStatus;
use crate::spacetime::bindings;
use crate::spacetime::bindings::TasksTableAccess as _;
use crate::sync::writes::{DrainReadBack, ReducerOutcome};

/// `[live, task_is_now_in_review]` off the task the drain acted on, shared by
/// every reducer whose answer is that shape (`subagent_stop`/`subagent_clear`).
/// `live` is `live_subagents`; giving both the identical slot layout is what
/// lets one function serve them rather than two near-duplicates.
pub(super) fn subagent_drain_read_back(
    ctx: &bindings::ReducerEventContext,
    task_id: i64,
) -> DrainReadBack {
    match ctx.db.tasks().id().find(&task_id) {
        Some(t) => DrainReadBack {
            live: t.live_subagents,
            is_review: is_review(&t.status),
        },
        None => DrainReadBack::default(),
    }
}

/// Whether a task row's `status` parses as [`TaskStatus::Review`]. One
/// predicate for the two read-backs that need it.
pub(super) fn is_review(status: &str) -> bool {
    TaskStatus::parse(status) == Some(TaskStatus::Review)
}

/// Fold a raw reducer answer that has NO application-level refusal into its
/// read-back value, or a genuine error. Unlike [`outcome_of`], an
/// `Err(InternalError)` here is a real anomaly rather than an ordinary "the
/// store said no" — none of the methods this serves has anything to fold it
/// into, since SQL never refuses them either.
pub(super) fn value_or_bail<T>(
    result: std::result::Result<
        std::result::Result<(), String>,
        spacetimedb_sdk::__codegen::InternalError,
    >,
    what: &str,
    value: impl FnOnce() -> T,
) -> anyhow::Result<T> {
    match result {
        Ok(Ok(())) => Ok(value()),
        Ok(Err(why)) => Err(anyhow!(
            "the shared store refused {what}, which should never happen: {why}"
        )),
        Err(why) => Err(anyhow!("the shared store could not answer {what}: {why}")),
    }
}

/// Fold a raw reducer answer into `Some`/`None` — `None` for the ordinary
/// refusal `try_record_stop` uses as its `NoOp`, and ALSO for a transport
/// `InternalError`, mirroring [`outcome_of`]'s identical fold for every other
/// reducer that has a real refusal to answer with.
pub(super) fn flag_or_refused<T>(
    result: std::result::Result<
        std::result::Result<(), String>,
        spacetimedb_sdk::__codegen::InternalError,
    >,
    value: impl FnOnce() -> T,
) -> Option<T> {
    match result {
        Ok(Ok(())) => Some(value()),
        Ok(Err(_)) | Err(_) => None,
    }
}

/// A create's answer: the ids the callback found, or the store's refusal.
///
/// The two refusal arms are [`outcome_of`]'s rather than a third copy. They are
/// the arms no CI test reaches — they need a live store — so a fourth create
/// written by copy-paste with a dropped `Ok(Err(why))` arm would look correct
/// and silently turn a refusal into "created, but outside this board's
/// subscriptions".
///
/// `ids` is a closure so the scan only happens on the arm that uses it.
pub(super) fn outcome_with_ids(
    result: std::result::Result<
        std::result::Result<(), String>,
        spacetimedb_sdk::__codegen::InternalError,
    >,
    ids: impl FnOnce() -> Vec<i64>,
) -> ReducerOutcome {
    match outcome_of(result) {
        ReducerOutcome::Applied(_) => ReducerOutcome::Applied(ids()),
        refused => refused,
    }
}

/// The answer of a call whose only answer is "did it work?".
pub(super) fn outcome_of(
    result: std::result::Result<
        std::result::Result<(), String>,
        spacetimedb_sdk::__codegen::InternalError,
    >,
) -> ReducerOutcome {
    match result {
        Ok(Ok(())) => ReducerOutcome::Applied(Vec::new()),
        Ok(Err(why)) => ReducerOutcome::Refused(why),
        Err(why) => ReducerOutcome::Refused(why.to_string()),
    }
}

/// Pull the generated id out of a create's answer.
///
/// # Why this can work at all
///
/// The view the callback reads is the client's subscription cache, so a
/// created row is findable only where a subscription already covers it
/// (`subscription_queries` above is the list). Task #4911 is the reason
/// `own_creations` is on that list unconditionally, for exactly this: an
/// epic-less task the operator owns arrives on `WHERE owner = …`, a task in a
/// followed epic on `WHERE epic_id = …`, and EVERYTHING ELSE — a brand-new
/// epic nobody follows yet, a task landing in an epic this board does not
/// follow — arrives on `WHERE created_by = …` instead, because
/// `SubscribeOnceIdentityIsSettled` asserts it the moment identity settles,
/// before any create this board makes could exist.
///
/// An applied create with NO matching row is now a real anomaly rather than
/// the ordinary case it used to be — own_creations covers every create this
/// identity can make — and the message still says what it would mean: the
/// store made the row, and this board's subscriptions do not cover where it
/// landed.
pub(super) fn generated_id(answer: ReducerOutcome, what: &str) -> anyhow::Result<i64> {
    match answer {
        ReducerOutcome::Applied(ids) => ids.into_iter().max().ok_or_else(|| {
            anyhow!(
                "the shared store created the {what} but it is outside this board's \
                 subscriptions, so its id could not be read back"
            )
        }),
        ReducerOutcome::Refused(why) => Err(anyhow!("the shared store refused: {why}")),
    }
}

/// Whether `candidate` is a row this board's epic create could have produced.
///
/// `created_by` narrows to this identity's own epics — the field
/// `own_creations` subscribes by — which a coincidence with a colleague's epic
/// of the same title, parent and millisecond cannot satisfy.
pub(super) fn matches_created_epic(candidate: &bindings::Epic, sent: &bindings::Epic) -> bool {
    candidate.title == sent.title
        && candidate.parent_epic_id == sent.parent_epic_id
        && candidate.created_at == sent.created_at
        && candidate.created_by == sent.created_by
}

/// Whether `candidate` is a row this board's create could have produced.
///
/// Every field here is one the CLIENT chose, so a match cannot be a coincidence
/// with somebody else's work. `owner` is blank for every task in an epic, so it
/// narrows nothing there; `created_by` is what actually pins a match to THIS
/// identity's own task when the candidate set includes an epic's other tasks
/// (from the `epic_id`-followed subscription) or nothing at all epic-scoped
/// (from `own_creations`).
pub(super) fn matches_create(candidate: &bindings::Task, sent: &bindings::Task) -> bool {
    candidate.title == sent.title
        && candidate.repo_path == sent.repo_path
        && candidate.owner == sent.owner
        && candidate.epic_id == sent.epic_id
        && candidate.created_at == sent.created_at
        && candidate.created_by == sent.created_by
}

/// Whether `candidate` is a row this board's learning create could have
/// produced.
///
/// Every field here is one the CLIENT chose. Unlike `matches_create`, there
/// is no identity field to pin the match to THIS board's own call —
/// `docs/specs/learnings.allium` allows genuine duplicates (same kind,
/// summary, scope and scope_ref recorded twice), so two boards creating an
/// identical learning in the same millisecond tie benignly: both rows are
/// real, both are somebody's, and returning either returns a learning that call made.
pub(super) fn matches_created_learning(
    candidate: &bindings::Learning,
    sent: &bindings::Learning,
) -> bool {
    candidate.kind == sent.kind
        && candidate.summary == sent.summary
        && candidate.scope == sent.scope
        && candidate.scope_ref == sent.scope_ref
        && candidate.source_task_id == sent.source_task_id
        && candidate.created_at == sent.created_at
}
