//! Work packages (`docs/specs/work-packages.allium`): the service every
//! package write goes through, from the board and from MCP alike.
//!
//! The data rules (a non-blank title, a package that exists, items that exist,
//! no item in both lists) live in the store's reducers, which write a create
//! or an update as one transaction. What stays here is the empty-call check
//! and the report of unlinks that touched nothing.

use std::collections::HashSet;
use std::sync::Arc;

use crate::models::{TaskStatus, WorkPackage, WorkPackageId, WorkPackageItem};
use crate::store;

use super::ServiceError;

/// Input to [`WorkPackageService::create_work_package`].
#[derive(Debug, Clone, Default)]
pub struct CreateWorkPackageParams {
    pub title: String,
    pub description: String,
    /// Items to link once the package exists (`LinkItem`, idempotent).
    pub link: Vec<WorkPackageItem>,
}

/// Input to [`WorkPackageService::update_work_package`]. `None` leaves a field
/// alone; the link lists are applied unlinks first, then links.
#[derive(Debug, Clone)]
pub struct UpdateWorkPackageParams {
    pub id: WorkPackageId,
    pub title: Option<String>,
    pub description: Option<String>,
    pub sort_order: Option<i64>,
    pub link: Vec<WorkPackageItem>,
    pub unlink: Vec<WorkPackageItem>,
}

impl UpdateWorkPackageParams {
    /// Params that change nothing on `id` yet.
    pub fn for_package(id: WorkPackageId) -> Self {
        Self {
            id,
            title: None,
            description: None,
            sort_order: None,
            link: Vec::new(),
            unlink: Vec::new(),
        }
    }

    fn changes_anything(&self) -> bool {
        self.title.is_some()
            || self.description.is_some()
            || self.sort_order.is_some()
            || !self.link.is_empty()
            || !self.unlink.is_empty()
    }
}

/// One linked item as a reader sees it: the item plus the title and status of
/// the task or epic it names.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkedItem {
    pub item: WorkPackageItem,
    pub title: String,
    pub status: TaskStatus,
}

/// A package with its resolved links, in link order (tasks then epics, each by
/// id). A link whose item this board does not hold is left out.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkPackageView {
    pub package: WorkPackage,
    pub items: Vec<LinkedItem>,
}

pub struct WorkPackageService {
    db: Arc<dyn store::WorkPackageStore>,
}

impl WorkPackageService {
    pub fn new(db: Arc<dyn store::WorkPackageStore>) -> Self {
        Self { db }
    }

    /// `CreateWorkPackage` then `LinkItem` for each of `params.link`, as one
    /// store transaction: a bad id leaves no package behind.
    pub async fn create_work_package(
        &self,
        params: CreateWorkPackageParams,
    ) -> Result<WorkPackageId, ServiceError> {
        let id = self
            .db
            .create_work_package(&params.title, &params.description, &params.link)
            .await?;
        Ok(id)
    }

    /// Every package with its linked items, or only `id` (not-found when it
    /// does not exist).
    pub async fn list_work_packages(
        &self,
        id: Option<WorkPackageId>,
    ) -> Result<Vec<WorkPackageView>, ServiceError> {
        let packages = self.db.list_work_packages().await;
        let packages: Vec<WorkPackage> = match id {
            None => packages,
            Some(id) => {
                let one: Vec<_> = packages.into_iter().filter(|p| p.id == id).collect();
                if one.is_empty() {
                    return Err(not_found(id));
                }
                one
            }
        };
        let links = self.db.list_work_package_links().await;
        let mut views = Vec::with_capacity(packages.len());
        for package in packages {
            let mut items = Vec::new();
            for link in links.iter().filter(|l| l.work_package_id == package.id) {
                if let Some(resolved) = self.resolve(link.item).await {
                    items.push(resolved);
                }
            }
            views.push(WorkPackageView { package, items });
        }
        Ok(views)
    }

    /// `UpdateWorkPackage`, then every unlink, then every link, as one store
    /// transaction. Answers the requested unlinks that were not linked, each
    /// once: still a success (`UnlinkItem` is idempotent), but the caller can
    /// tell that nothing changed for it.
    pub async fn update_work_package(
        &self,
        params: UpdateWorkPackageParams,
    ) -> Result<Vec<WorkPackageItem>, ServiceError> {
        if !params.changes_anything() {
            return Err(ServiceError::Validation(
                "update_work_package needs at least one of title, description, sort_order, or \
                 a link or unlink list"
                    .into(),
            ));
        }
        // Read before the write: a link the same call adds is not a link the
        // unlink could have removed. A racing writer can make this stale; the
        // report is advice, the write is the store's.
        let not_linked: Vec<WorkPackageItem> = if params.unlink.is_empty() {
            Vec::new()
        } else {
            let linked: HashSet<WorkPackageItem> = self
                .db
                .list_work_package_links()
                .await
                .into_iter()
                .filter(|l| l.work_package_id == params.id)
                .map(|l| l.item)
                .collect();
            dedup(&params.unlink)
                .into_iter()
                .filter(|i| !linked.contains(i))
                .collect()
        };
        self.db
            .update_work_package(
                params.id,
                params.title.as_deref(),
                params.description.as_deref(),
                params.sort_order,
                &params.unlink,
                &params.link,
            )
            .await?;
        Ok(not_linked)
    }

    /// `DeleteWorkPackage`: the package and its links go; no item is touched.
    pub async fn delete_work_package(&self, id: WorkPackageId) -> Result<(), ServiceError> {
        self.require_package(id).await?;
        self.db.delete_work_package(id).await?;
        Ok(())
    }

    async fn require_package(&self, id: WorkPackageId) -> Result<(), ServiceError> {
        if self
            .db
            .list_work_packages()
            .await
            .iter()
            .any(|p| p.id == id)
        {
            Ok(())
        } else {
            Err(not_found(id))
        }
    }

    async fn resolve(&self, item: WorkPackageItem) -> Option<LinkedItem> {
        let (title, status) = match item {
            WorkPackageItem::Task(id) => {
                let t = self.db.get_task(id).await?;
                (t.title, t.status)
            }
            WorkPackageItem::Epic(id) => {
                let e = self.db.get_epic(id).await?;
                (e.title, e.status)
            }
        };
        Some(LinkedItem {
            item,
            title,
            status,
        })
    }
}

fn not_found(id: WorkPackageId) -> ServiceError {
    ServiceError::NotFound(format!("Work package {} not found", id.0))
}

/// `items` in first-seen order with repeats dropped: a repeated id is one
/// write, not two.
fn dedup(items: &[WorkPackageItem]) -> Vec<WorkPackageItem> {
    let mut seen = HashSet::new();
    items.iter().copied().filter(|i| seen.insert(*i)).collect()
}

#[cfg(test)]
mod tests;
