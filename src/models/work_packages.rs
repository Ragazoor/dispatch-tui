//! Work packages (`docs/specs/work-packages.allium`): a named grouping that
//! links to existing tasks and epics across the epic tree.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{EpicId, TaskId};
use crate::define_id_newtype;

define_id_newtype!(WorkPackageId, work_package_id_tests);

/// One work package. It has no status of its own: an item's done-ness is read
/// from the item (`work-packages.allium`, the decisions in its header).
#[derive(Debug, Clone, PartialEq)]
pub struct WorkPackage {
    pub id: WorkPackageId,
    pub title: String,
    pub description: String,
    /// Manual card order; see [`Self::sort_key`].
    pub sort_order: Option<i64>,
    /// Who created it, as shown (`workspaces.allium: CreatorIsShownByName`):
    /// the creator's display name, or their identity's prefix when they have
    /// set none. `None` when the store holds no creator.
    pub created_by: Option<String>,
    /// The workspace this package lives in, as the store's workspace id.
    pub workspace_id: i64,
    pub visibility: crate::models::Visibility,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl WorkPackage {
    /// The key package cards order by in every column, ascending: the manual
    /// `sort_order`, falling back to the id (`work-packages.allium:
    /// ReorderPackage`). Same shape as `Epic::sort_key`.
    pub fn sort_key(&self) -> i64 {
        self.sort_order.unwrap_or(self.id.0)
    }
}

/// What a link points at: one task or one epic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WorkPackageItem {
    Task(TaskId),
    Epic(EpicId),
}

impl WorkPackageItem {
    /// The store's spelling of the item kind (`work_package_links.kind`).
    pub fn kind(self) -> &'static str {
        match self {
            WorkPackageItem::Task(_) => "task",
            WorkPackageItem::Epic(_) => "epic",
        }
    }

    /// The raw id of the item, whatever its kind.
    pub fn raw_id(self) -> i64 {
        match self {
            WorkPackageItem::Task(id) => id.0,
            WorkPackageItem::Epic(id) => id.0,
        }
    }

    /// The inverse of [`Self::kind`] plus [`Self::raw_id`]; `None` for a kind
    /// this binary does not know.
    pub fn from_parts(kind: &str, id: i64) -> Option<Self> {
        match kind {
            "task" => Some(WorkPackageItem::Task(TaskId(id))),
            "epic" => Some(WorkPackageItem::Epic(EpicId(id))),
            _ => None,
        }
    }
}

/// One link from a package to one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorkPackageLink {
    pub work_package_id: WorkPackageId,
    pub item: WorkPackageItem,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_round_trips_through_its_store_spelling() {
        for item in [
            WorkPackageItem::Task(TaskId(3)),
            WorkPackageItem::Epic(EpicId(3)),
        ] {
            assert_eq!(
                WorkPackageItem::from_parts(item.kind(), item.raw_id()),
                Some(item)
            );
        }
    }

    #[test]
    fn an_unknown_kind_does_not_decode() {
        assert_eq!(WorkPackageItem::from_parts("board", 1), None);
    }
}
