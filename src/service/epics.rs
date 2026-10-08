use std::sync::Arc;

use crate::models::{completed_at_for_status_transition, Epic, EpicId, Task, TaskStatus};
use crate::store::{self, EpicPatch};

use super::{validate_feed_interval, FieldUpdate, ServiceError};

/// Load the epic a caller NAMED as a target for new work, or fail with
/// `NotFound`.
///
/// Extracted because the existence check had been written inline three times
/// and had begun to drift: the same operator mistake, naming an epic that does
/// not exist, answered `NotFound` on the reassign path and something else on
/// the create path. One helper means one answer. Never a routed substitute —
/// see `resolve_routed_epic`.
pub async fn require_epic_accepting_work(
    db: &dyn store::EpicRead,
    epic_id: EpicId,
) -> Result<Epic, ServiceError> {
    db.get_epic(epic_id)
        .await?
        .ok_or_else(|| ServiceError::NotFound(format!("Epic {} not found", epic_id.0)))
}

// ---------------------------------------------------------------------------
// UpdateEpicParams
// ---------------------------------------------------------------------------

pub struct UpdateEpicParams {
    pub epic_id: EpicId,
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub plan_path: Option<String>,
    pub sort_order: Option<i64>,
    /// The Done column's ordering key. `None` = leave untouched; `Some(v)` =
    /// write `v`. Normally derived by the service from the status transition
    /// (`completed_at_for_status_transition`) rather than set by a caller —
    /// the exception is a manual reorder of this epic's card in the Done
    /// column, which persists an override here.
    pub completed_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    pub auto_dispatch: Option<bool>,
    pub feed_command: Option<FieldUpdate>,
    pub feed_interval_secs: Option<Option<i64>>,
    pub group_by_repo: Option<bool>,
    /// When set, flips whether this epic's feed removes tasks the emission
    /// omits. See `docs/specs/feeds.allium`: `AppendOnlyFeed`.
    pub feed_append_only: Option<bool>,
    /// Triple-state: None = no change, Some(Some(id)) = reparent, Some(None) = make root.
    pub parent_epic_id: Option<Option<EpicId>>,
}

impl UpdateEpicParams {
    pub(in crate::service) fn has_any_field(&self) -> bool {
        !self.updated_field_names().is_empty()
    }

    /// Names of the fields this params value actually sets. Mirrors
    /// [`UpdateTaskParams::updated_field_names`](crate::service::UpdateTaskParams::updated_field_names)
    /// — same compiler-enforced parity, same reason.
    pub fn updated_field_names(&self) -> Vec<&str> {
        let Self {
            epic_id: _,
            title,
            description,
            status,
            plan_path,
            sort_order,
            completed_at,
            auto_dispatch,
            feed_command,
            feed_interval_secs,
            group_by_repo,
            feed_append_only,
            parent_epic_id,
        } = self;

        [
            ("title", title.is_some()),
            ("description", description.is_some()),
            ("status", status.is_some()),
            ("plan_path", plan_path.is_some()),
            ("sort_order", sort_order.is_some()),
            ("completed_at", completed_at.is_some()),
            ("auto_dispatch", auto_dispatch.is_some()),
            ("feed_command", feed_command.is_some()),
            ("feed_interval_secs", feed_interval_secs.is_some()),
            ("group_by_repo", group_by_repo.is_some()),
            ("feed_append_only", feed_append_only.is_some()),
            ("parent_epic_id", parent_epic_id.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, is_set)| is_set.then_some(name))
        .collect()
    }
}

/// Result of [`EpicService::update_epic`]. Mirrors
/// [`UpdateTaskResult`](crate::service::UpdateTaskResult) — same
/// capture-before-write shape, same reason: the service (not the caller)
/// computes `completed_at` on a Done-transition, so a caller holding its own
/// in-memory copy of the epic (the TUI's `App.board.epics`) needs a way to
/// learn that value without a second DB round-trip.
#[derive(Debug, Clone)]
pub struct UpdateEpicResult {
    pub epic_id: EpicId,
    /// `None` = this call's patch didn't touch `completed_at`. `Some(v)` = it
    /// did, where `v` is exactly what was written (`Some(None)` for a clear,
    /// `Some(Some(t))` for a set to `t`).
    pub completed_at_after_write: Option<Option<chrono::DateTime<chrono::Utc>>>,
}

// ---------------------------------------------------------------------------
// CreateEpicParams
// ---------------------------------------------------------------------------

pub struct CreateEpicParams {
    pub title: String,
    pub description: String,
    pub sort_order: Option<i64>,
    pub parent_epic_id: Option<EpicId>,
    pub feed_command: Option<String>,
    pub feed_interval_secs: Option<i64>,
}

// ---------------------------------------------------------------------------
// Progress-rollup helper types
// ---------------------------------------------------------------------------

/// Tasks grouped by their epic id, for the progress rollup.
type TasksByEpic<'a> = std::collections::HashMap<EpicId, Vec<&'a Task>>;

/// (done, total) over a task slice, counting `TaskStatus::Done` as done.
fn count_progress(tasks: &[&Task]) -> (usize, usize) {
    (
        tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Done)
            .count(),
        tasks.len(),
    )
}

// ---------------------------------------------------------------------------
// EpicService
// ---------------------------------------------------------------------------

pub struct EpicService {
    pub db: Arc<dyn store::TaskAndEpicStore>,
    /// The local half, held only for the repo-group cleanup rule: deleting an
    /// empty `RepoGroup` sub-epic re-scopes its learnings onto the parent
    /// first. Separate from `db` because the two sit on opposite sides of the
    /// store seam — see "The store seam" in `docs/conventions.md`.
    learnings: Arc<dyn store::LearningStore>,
    clock: Arc<dyn crate::service::Clock>,
}

impl EpicService {
    pub fn new(
        db: Arc<dyn store::TaskAndEpicStore>,
        learnings: Arc<dyn store::LearningStore>,
    ) -> Self {
        Self {
            db,
            learnings,
            clock: Arc::new(crate::service::SystemClock),
        }
    }

    /// Override the clock used for the Done-transition completion stamp.
    /// Tests inject a `FixedClock` for determinism; mirrors
    /// `TaskService::with_clock`.
    pub fn with_clock(mut self, clock: Arc<dyn crate::service::Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Materialise the managed feed-epic tree from already-read settings.
    /// The epic writes go through the service's `EpicCrud` handle.
    pub async fn provision_managed_feeds(
        &self,
        settings: crate::service::ManagedFeedSettings,
    ) -> Result<(), ServiceError> {
        crate::service::ensure_managed_epics(
            &*self.db,
            settings.reviews_command.as_deref(),
            settings.reviews_interval_secs,
            settings.cve_command.as_deref(),
            settings.cve_interval_secs,
        )
        .await
        .map_err(ServiceError::from)
    }

    pub async fn create_epic(&self, params: CreateEpicParams) -> Result<Epic, ServiceError> {
        // Before the insert, not after: creation is bound as tightly as update,
        // so an epic must not be able to be *born* below the floor and then be
        // rejected on the follow-up patch, leaving a half-created epic behind.
        validate_feed_interval("feed_interval_secs", params.feed_interval_secs)?;

        if let Some(parent_id) = params.parent_epic_id {
            self.db.get_epic(parent_id).await?.ok_or_else(|| {
                ServiceError::NotFound(format!("Parent epic {} not found", parent_id.0))
            })?;
        }

        let epic = self
            .db
            .create_epic(&params.title, &params.description, params.parent_epic_id)
            .await?;

        // A new sub-epic changes the parent's active_sub_epics set, so the
        // parent must be recalculated immediately (e.g. a Done parent with a
        // freshly-attached Backlog child regresses right away).
        if let Some(parent_id) = params.parent_epic_id {
            self.recalculate_epic(parent_id).await;
        }

        // The insert above only carries title/description/parent, so anything
        // else the caller supplied needs a follow-up write.
        let patch = EpicPatch {
            sort_order: params.sort_order.map(Some),
            feed_command: params.feed_command.as_deref().map(Some),
            feed_interval_secs: params.feed_interval_secs.map(Some),
            ..EpicPatch::new()
        };
        if !patch.has_changes() {
            return Ok(epic);
        }

        self.db.patch_epic(epic.id, &patch).await?;
        // Re-read so the returned Epic reflects the follow-up write rather
        // than the pre-patch insert result.
        self.get_epic(epic.id).await
    }

    pub async fn get_epic(&self, epic_id: EpicId) -> Result<Epic, ServiceError> {
        self.db
            .get_epic(epic_id)
            .await?
            .ok_or_else(|| ServiceError::NotFound(format!("Epic {} not found", epic_id.0)))
    }

    pub async fn get_epic_with_subtasks(
        &self,
        epic_id: EpicId,
    ) -> Result<(Epic, Vec<Task>), ServiceError> {
        let epic = self.get_epic(epic_id).await?;
        let subtasks = self
            .db
            .list_tasks_for_epic(epic.id)
            .await
            .unwrap_or_default();
        Ok((epic, subtasks))
    }

    /// Progress for a single epic, rolled up the same way as
    /// [`list_epics_with_progress`](Self::list_epics_with_progress): a
    /// `group_by_repo` epic's counts include its descendant sub-epics, not
    /// just its direct subtasks. Used by `get_epic` so it agrees with the
    /// board/`list_epics` view instead of undercounting to 0/0.
    ///
    /// Only `group_by_repo` epics need the whole-board rollup — every other
    /// epic still counts its own direct subtasks, so the common case stays as
    /// cheap as the old direct-subtasks-only lookup.
    pub async fn get_epic_with_progress(
        &self,
        epic_id: EpicId,
    ) -> Result<(Epic, usize, usize), ServiceError> {
        let epic = self.get_epic(epic_id).await?;
        if !epic.group_by_repo {
            let subtasks = self
                .db
                .list_tasks_for_epic(epic.id)
                .await
                .unwrap_or_default();
            let (done, total) = count_progress(&subtasks.iter().collect::<Vec<_>>());
            return Ok((epic, done, total));
        }

        let all_epics = self.db.list_epics().await?;
        let all_tasks = self.db.list_all_tasks_with_epic_id().await?;
        let tasks_by_epic = Self::group_tasks_by_epic(&all_tasks);
        let children = crate::models::build_children_map(&all_epics);
        let (done, total) = Self::epic_progress(&epic, &tasks_by_epic, &children);
        Ok((epic, done, total))
    }

    /// Group tasks by epic id, for the progress rollup.
    fn group_tasks_by_epic(tasks: &[Task]) -> TasksByEpic<'_> {
        let mut tasks_by_epic: TasksByEpic = std::collections::HashMap::new();
        for task in tasks {
            if let Some(eid) = task.epic_id {
                tasks_by_epic.entry(eid).or_default().push(task);
            }
        }
        tasks_by_epic
    }

    /// (done, total) for one epic: direct subtasks, plus — when the epic is
    /// `group_by_repo` — the rollup of all descendant sub-epics' tasks, via
    /// the shared [`crate::models::descendant_epic_ids_with_map`] traversal
    /// (the same one `SubtaskStats::for_epic` uses in the TUI).
    fn epic_progress(
        epic: &Epic,
        tasks_by_epic: &TasksByEpic<'_>,
        children: &std::collections::HashMap<EpicId, Vec<EpicId>>,
    ) -> (usize, usize) {
        if epic.group_by_repo {
            let ids = crate::models::descendant_epic_ids_with_map(epic.id, children);
            let tasks: Vec<&Task> = ids
                .iter()
                .filter_map(|id| tasks_by_epic.get(id))
                .flatten()
                .copied()
                .collect();
            count_progress(&tasks)
        } else {
            let empty = Vec::new();
            let tasks = tasks_by_epic.get(&epic.id).unwrap_or(&empty);
            count_progress(tasks)
        }
    }

    pub async fn list_epics(&self) -> Result<Vec<Epic>, ServiceError> {
        Ok(self.db.list_epics().await?)
    }

    pub async fn list_root_epics(&self) -> Result<Vec<Epic>, ServiceError> {
        Ok(self.db.list_root_epics().await?)
    }

    pub async fn list_sub_epics(&self, parent_id: EpicId) -> Result<Vec<Epic>, ServiceError> {
        Ok(self.db.list_sub_epics(parent_id).await?)
    }

    pub async fn list_epics_with_progress(
        &self,
    ) -> Result<Vec<(Epic, usize, usize)>, ServiceError> {
        self.list_epics_with_progress_under(None, false).await
    }

    /// [`list_epics_with_progress`](Self::list_epics_with_progress) narrowed
    /// to `parent`'s direct children, or with `recursive` to its whole subtree
    /// (never `parent` itself). `parent = None` lists every epic, and
    /// `recursive` without a parent is a validation error: the full list is
    /// already every epic. See `ListEpicsViaMcp` in `docs/specs/epics.allium`.
    pub async fn list_epics_with_progress_under(
        &self,
        parent: Option<EpicId>,
        recursive: bool,
    ) -> Result<Vec<(Epic, usize, usize)>, ServiceError> {
        if recursive && parent.is_none() {
            return Err(ServiceError::Validation(
                "recursive needs parent_epic_id: without it every epic is already listed".into(),
            ));
        }
        let epics = self.list_epics().await?;
        let children = crate::models::build_children_map(&epics);
        let keep: Option<std::collections::HashSet<EpicId>> = match parent {
            None => None,
            Some(p) if !epics.iter().any(|e| e.id == p) => {
                return Err(ServiceError::NotFound(format!("Epic {p} not found")));
            }
            Some(p) if recursive => {
                let mut subtree = crate::models::descendant_epic_ids_with_map(p, &children);
                subtree.remove(&p);
                Some(subtree)
            }
            Some(p) => Some(children.get(&p).into_iter().flatten().copied().collect()),
        };
        let all_subtasks = self.db.list_all_tasks_with_epic_id().await?;
        let tasks_by_epic = Self::group_tasks_by_epic(&all_subtasks);

        let result = epics
            .into_iter()
            .filter(|e| keep.as_ref().is_none_or(|k| k.contains(&e.id)))
            .map(|e| {
                let (done, total) = Self::epic_progress(&e, &tasks_by_epic, &children);
                (e, done, total)
            })
            .collect();
        Ok(result)
    }

    pub async fn update_epic(
        &self,
        params: UpdateEpicParams,
    ) -> Result<UpdateEpicResult, ServiceError> {
        if !params.has_any_field() {
            return Err(ServiceError::Validation(
                "At least one field must be provided".into(),
            ));
        }

        // Ahead of every write, so a refused cadence takes the whole update
        // with it. The epic editor sends title and interval in one call, and a
        // partial apply would save the title against a cadence the service
        // refused (epics.allium: EditEpic).
        //
        // `Some(None)` — clear the field — is allowed through: it means
        // "inherit config.default_feed_interval", which itself clears the
        // floor, so clearing can never sink below it.
        validate_feed_interval("feed_interval_secs", params.feed_interval_secs.flatten())?;

        let epic_id = params.epic_id;
        let existing = self.db.get_epic(epic_id).await?;

        Self::check_append_only_compatible(&params, existing.as_ref())?;

        let mut patch = Self::base_patch(&params);

        // Fetch the prior epic whenever status changes, to detect a
        // transition INTO Done for the completion-stamp rule. This method has
        // no other prior-fetch to reuse (the RepoGroup-reparent guard below
        // does its own, gated on a different condition).
        if let Some(new_status) = params.status {
            if let Some(prior_epic) = self.db.get_epic(params.epic_id).await? {
                if let Some(at) = completed_at_for_status_transition(
                    prior_epic.status,
                    new_status,
                    self.clock.now(),
                ) {
                    patch = patch.completed_at(Some(at));
                }
            }
        }

        // Prevent reparenting or detaching a RepoGroup sub-epic: both
        // Some(Some(_)) (reparent) and Some(None) (detach to root) would
        // orphan an auto-created sub-epic outside its grouping root.
        if matches!(params.parent_epic_id, Some(Some(_)) | Some(None)) {
            if let Some(ref epic) = existing {
                if epic.origin == crate::models::EpicOrigin::RepoGroup {
                    return Err(ServiceError::Validation(
                        "Cannot reparent an auto-created repo-group sub-epic".into(),
                    ));
                }
            }
        }

        match params.parent_epic_id {
            Some(Some(new_parent_id)) => {
                let parent = self.get_epic(new_parent_id).await?;
                self.check_no_cycle(epic_id, &parent).await?;
                patch = patch.parent_epic_id(Some(new_parent_id));
            }
            Some(None) => {
                patch = patch.parent_epic_id(None);
            }
            None => {}
        }

        // Captured before the write so the caller can learn what this call
        // wrote to completed_at (including the Done-transition stamp above)
        // without a second DB round-trip. See `UpdateEpicResult`.
        let completed_at_after_write = patch.completed_at;
        self.db.patch_epic(epic_id, &patch).await?;

        self.recalculate_parents_after_update(existing, &params)
            .await;
        Ok(UpdateEpicResult {
            epic_id,
            completed_at_after_write,
        })
    }

    /// Refuse `feed_append_only` together with `group_by_repo` or a feed role.
    fn check_append_only_compatible(
        params: &UpdateEpicParams,
        existing: Option<&Epic>,
    ) -> Result<(), ServiceError> {
        // Repo is a MIRRORING feed's key: a PR, a CVE, a Dependabot alert each
        // belongs to exactly one repo and carries that repo's URL as its own,
        // so group_by_repo partitions the emission along an axis its items
        // already have. An APPEND-ONLY feed's items are events keyed by where
        // in the code they fired (a log record's level, module and message
        // head), and the one url they carry is a configured repo root that
        // exists only so dispatch can resolve a local clone. Grouping such an
        // epic would put every item in a single sub-epic — grouping by a
        // constant. Several repos' events are covered by one flat append-only
        // epic per repo under a common parent, which FeedRunner polls exactly
        // as it polls root epics.
        //
        // This restriction is PERMANENT (task #4640 decided it), not the
        // conservative holding position it started as. In particular it does
        // not hinge on the grouped path's migration deadlock — see
        // feeds.allium: AppendOnlyFeed for why that was a symptom of a defect
        // in GroupedFeedUpsert's own migration rather than the reason here.
        //
        // Evaluated against the POST-update values, so the pair is refused
        // whichever flag arrives second and whether they arrive together or
        // apart. feed_role is not settable here, so it is read as-is.
        let appends_only = params
            .feed_append_only
            .or_else(|| existing.as_ref().map(|e| e.feed_append_only))
            .unwrap_or(false);
        if appends_only {
            let grouped = params
                .group_by_repo
                .or_else(|| existing.as_ref().map(|e| e.group_by_repo))
                .unwrap_or(false);
            let routed = existing
                .as_ref()
                .is_some_and(|e| e.feed_role != crate::models::FeedRole::None);
            if grouped || routed {
                return Err(ServiceError::Validation(
                    "feed_append_only cannot be combined with group_by_repo or a feed role: \
                     grouping keys on the repo an item belongs to, but an append-only feed's \
                     items are events keyed by where in the code they fired, and the one URL \
                     they carry is a configured repo root — so every item would land in the \
                     same sub-epic. To cover several repos, use one flat append-only epic per \
                     repo under a common parent."
                        .to_string(),
                ));
            }
        }
        Ok(())
    }

    /// The patch for the plain fields `params` carries.
    fn base_patch(params: &UpdateEpicParams) -> EpicPatch<'_> {
        let mut patch = EpicPatch::new();
        if let Some(ref t) = params.title {
            patch = patch.title(t);
        }
        if let Some(ref d) = params.description {
            patch = patch.description(d);
        }
        if let Some(status) = params.status {
            patch = patch.status(status);
        }
        if let Some(ref p) = params.plan_path {
            patch = patch.plan_path(Some(p.as_str()));
        }
        if let Some(so) = params.sort_order {
            patch = patch.sort_order(Some(so));
        }
        if let Some(at) = params.completed_at {
            patch = patch.completed_at(at);
        }
        if let Some(ad) = params.auto_dispatch {
            patch = patch.auto_dispatch(ad);
        }
        if let Some(ref fc) = params.feed_command {
            patch = patch.feed_command(fc.as_option());
        }
        if let Some(fi) = params.feed_interval_secs {
            patch = patch.feed_interval_secs(fi);
        }
        if let Some(append_only) = params.feed_append_only {
            patch = patch.feed_append_only(append_only);
        }
        if let Some(gbr) = params.group_by_repo {
            patch = patch.group_by_repo(gbr);
        }
        patch
    }

    /// Recalculate the parents whose rollup this update changed.
    async fn recalculate_parents_after_update(
        &self,
        existing: Option<Epic>,
        params: &UpdateEpicParams,
    ) {
        // recalculate_epic_status must run whenever a sub-epic's status
        // changes or its parent membership changes, since either mutates a
        // parent's active_sub_epics rollup. Recalculate the *parent*, not
        // this epic itself — self-recalc would fight an explicit status
        // write with the children-derived target.
        if let Some(existing) = existing {
            if let Some(new_parent) = params.parent_epic_id {
                if let Some(old_parent) = existing.parent_epic_id {
                    self.recalculate_epic(old_parent).await;
                }
                if let Some(new_parent) = new_parent {
                    self.recalculate_epic(new_parent).await;
                }
            } else if params.status.is_some() {
                if let Some(parent) = existing.parent_epic_id {
                    self.recalculate_epic(parent).await;
                }
            }
        }
    }

    /// Recalculate the given epic, logging any database error.
    async fn recalculate_epic(&self, epic_id: EpicId) {
        if let Err(err) = self.db.recalculate_epic_status(epic_id).await {
            tracing::warn!(
                "failed to recalculate epic status for epic {}: {err}",
                epic_id.0
            );
        }
    }

    /// Walk the ancestor chain of `proposed_parent` and return a Validation error
    /// if `epic_id` appears in it (which would create a cycle).
    /// Takes a pre-fetched `&Epic` to avoid an extra DB round-trip.
    async fn check_no_cycle(
        &self,
        epic_id: EpicId,
        proposed_parent: &Epic,
    ) -> Result<(), ServiceError> {
        if proposed_parent.id == epic_id {
            return Err(ServiceError::Validation(
                "Setting this parent would create a cycle in the epic hierarchy".into(),
            ));
        }
        let mut current_opt = proposed_parent.parent_epic_id;
        loop {
            let current = match current_opt {
                None => return Ok(()),
                Some(c) => c,
            };
            if current == epic_id {
                return Err(ServiceError::Validation(
                    "Setting this parent would create a cycle in the epic hierarchy".into(),
                ));
            }
            match self.db.get_epic(current).await? {
                Some(e) => current_opt = e.parent_epic_id,
                None => return Ok(()),
            }
        }
    }

    pub async fn regroup_epic(&self, root: EpicId) -> Result<(), ServiceError> {
        crate::service::regroup_epic(&*self.db, root).await
    }

    pub async fn flatten_epic(&self, root: EpicId) -> Result<(), ServiceError> {
        crate::service::flatten_epic(&*self.db, &*self.learnings, root).await
    }

    pub async fn reroute_on_repo_change(
        &self,
        task: crate::models::TaskId,
        new_repo: &str,
    ) -> Result<(), ServiceError> {
        crate::service::reroute_on_repo_change(&*self.db, &*self.learnings, task, new_repo).await
    }

    /// The delete pre-check (`DeleteEpic`, epics.allium): every task in the
    /// subtree must be done in the store's true rows.
    ///
    /// The refusal names what blocks the delete (`DeleteEpicRefused`): the
    /// lowest-id task that is not done, and any subtree task whose row the
    /// client could not decode — its status is unknown, so it blocks too.
    pub async fn ensure_deletable(&self, epic_id: EpicId) -> Result<(), ServiceError> {
        self.get_epic(epic_id).await?;

        let mut blocking = SubtreeBlockers::default();
        collect_subtree_blockers(&*self.db, epic_id, &mut blocking).await?;
        match blocking.message() {
            Some(msg) => Err(ServiceError::Validation(msg)),
            None => Ok(()),
        }
    }

    /// epics.allium: `ConfirmDeleteEpic`/`DeleteEpic`. Refuses unless every
    /// task anywhere in `epic_id`'s subtree, at any depth, is `done` — an
    /// empty subtree qualifies. Permanent: there is no archived fallback for
    /// an epic to land in instead. The DB-layer delete itself (`self.db`'s
    /// `delete_epic`) writes the `RetiredFeedItem` retirement records and
    /// performs the actual recursive removal; this guard is the one thing it
    /// does not check, since `FlattenEpic` and the feed's empty repo-group
    /// cleanup call the DB method directly and apply their own, narrower
    /// guarantee (no tasks left at all) rather than this one.
    pub async fn delete_epic(&self, epic_id: EpicId) -> Result<(), ServiceError> {
        self.ensure_deletable(epic_id).await?;

        self.db
            .delete_epic(epic_id)
            .await
            .map_err(ServiceError::from)
    }
}

/// What stops an epic delete (epics.allium: `DeleteEpicRefused`), gathered
/// over the whole subtree.
#[derive(Default)]
struct SubtreeBlockers {
    /// The lowest-id task that is not done: id, title, status.
    first_undone: Option<(crate::models::TaskId, String, TaskStatus)>,
    /// Tasks whose row the client could not decode. Their status is unknown,
    /// so they block the delete.
    unreadable: Vec<crate::models::TaskId>,
}

impl SubtreeBlockers {
    fn message(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some((id, title, status)) = &self.first_undone {
            parts.push(format!(
                "task #{} \"{}\" (status: {}) in its subtree is not done",
                id.0,
                title,
                status.as_str()
            ));
        }
        if !self.unreadable.is_empty() {
            let mut ids = self.unreadable.clone();
            ids.sort();
            let ids = ids
                .iter()
                .map(|id| format!("#{}", id.0))
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!(
                "{} task(s) in its subtree could not be read (ids {ids})",
                self.unreadable.len()
            ));
        }
        if parts.is_empty() {
            None
        } else {
            Some(format!("cannot delete epic: {}", parts.join("; ")))
        }
    }
}

/// Walk `epic_id`'s subtree (its own direct tasks, plus those of every
/// descendant epic, at any depth) and record what blocks a delete. An epic
/// with no tasks has no blockers. Boxed for recursion — async fns cannot
/// recurse unboxed, since the compiler would need an infinitely-sized future
/// type.
fn collect_subtree_blockers<'a>(
    db: &'a dyn store::TaskAndEpicStore,
    epic_id: EpicId,
    out: &'a mut SubtreeBlockers,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ServiceError>> + Send + 'a>> {
    Box::pin(async move {
        for t in db.list_tasks_for_epic(epic_id).await? {
            let lower = out
                .first_undone
                .as_ref()
                .is_none_or(|(id, _, _)| t.id < *id);
            if t.status != TaskStatus::Done && lower {
                out.first_undone = Some((t.id, t.title.clone(), t.status));
            }
        }
        out.unreadable
            .extend(db.list_undecodable_task_ids_for_epic(epic_id).await?);
        for sub in db.list_sub_epics(epic_id).await? {
            collect_subtree_blockers(db, sub.id, out).await?;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests;
