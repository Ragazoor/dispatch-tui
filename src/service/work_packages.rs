//! Work packages (`docs/specs/work-packages.allium`): the service every
//! package write goes through, from the board and from MCP alike.
//!
//! A package only links to tasks and epics that already exist, so the rules
//! here are about the links: every id a call names is checked before the first
//! write, and a call that would both link and unlink the same item is refused
//! outright rather than left to an order the caller cannot see.

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
            link: Vec::new(),
            unlink: Vec::new(),
        }
    }

    fn changes_anything(&self) -> bool {
        self.title.is_some()
            || self.description.is_some()
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

    /// `CreateWorkPackage` then `LinkItem` for each of `params.link`. Every
    /// item is checked to exist before the package is created, so a bad id
    /// leaves no package behind.
    pub async fn create_work_package(
        &self,
        params: CreateWorkPackageParams,
    ) -> Result<WorkPackageId, ServiceError> {
        let title = require_title(&params.title)?;
        self.require_items(&params.link).await?;
        let id = self
            .db
            .create_work_package(title, &params.description)
            .await?;
        for item in dedup(&params.link) {
            self.db.link_work_package_item(id, item).await?;
        }
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

    /// `UpdateWorkPackage`, then every unlink, then every link. Everything is
    /// validated before the first write: the package, a non-blank title, no
    /// item in both lists, and every item to link.
    pub async fn update_work_package(
        &self,
        params: UpdateWorkPackageParams,
    ) -> Result<(), ServiceError> {
        if !params.changes_anything() {
            return Err(ServiceError::Validation(
                "update_work_package needs at least one of title, description, or a link or \
                 unlink list"
                    .into(),
            ));
        }
        let title = params.title.as_deref().map(require_title).transpose()?;
        let unlink: HashSet<WorkPackageItem> = params.unlink.iter().copied().collect();
        if let Some(both) = params.link.iter().find(|i| unlink.contains(i)) {
            return Err(ServiceError::Validation(format!(
                "{} {} is in both a link and an unlink list; name it in one",
                both.kind(),
                both.raw_id()
            )));
        }
        self.require_package(params.id).await?;
        self.require_items(&params.link).await?;

        if title.is_some() || params.description.is_some() {
            self.db
                .patch_work_package(params.id, title, params.description.as_deref())
                .await?;
        }
        for item in dedup(&params.unlink) {
            self.db.unlink_work_package_item(params.id, item).await?;
        }
        for item in dedup(&params.link) {
            self.db.link_work_package_item(params.id, item).await?;
        }
        Ok(())
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

    async fn require_items(&self, items: &[WorkPackageItem]) -> Result<(), ServiceError> {
        for &item in items {
            if self.resolve(item).await.is_none() {
                return Err(ServiceError::NotFound(format!(
                    "{} {} not found",
                    capitalised(item.kind()),
                    item.raw_id()
                )));
            }
        }
        Ok(())
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

fn require_title(title: &str) -> Result<&str, ServiceError> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(ServiceError::Validation(
            "a work package needs a non-empty title".into(),
        ));
    }
    Ok(trimmed)
}

fn not_found(id: WorkPackageId) -> ServiceError {
    ServiceError::NotFound(format!("Work package {} not found", id.0))
}

/// `kind` with its first letter upper-cased ("task" -> "Task"), so an error
/// names the item kind from `WorkPackageItem::kind`'s one table.
fn capitalised(kind: &str) -> String {
    let mut chars = kind.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// `items` in first-seen order with repeats dropped: a repeated id is one
/// write, not two.
fn dedup(items: &[WorkPackageItem]) -> Vec<WorkPackageItem> {
    let mut seen = HashSet::new();
    items.iter().copied().filter(|i| seen.insert(*i)).collect()
}

#[cfg(test)]
mod tests;
