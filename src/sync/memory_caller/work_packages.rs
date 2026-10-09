//! Work packages (`docs/specs/work-packages.allium`): this store's
//! reimplementation of `spacetime/module/src/packages.rs`.

use anyhow::Result;

use dispatch_spacetime_module as module;

use crate::models::{WorkPackageId, WorkPackageItem};
use crate::spacetime::bindings;

use super::super::writes::ReducerOutcome;
use super::{MemoryReducerCaller, Tables};

/// Mirrors the module's private `validate_work_package_title`, wording
/// included, so a caller cannot tell the backends apart from the refusal.
fn validate_title(title: &str) -> Result<(), String> {
    if title.trim().is_empty() {
        return Err("a work package needs a non-empty title".into());
    }
    Ok(())
}

impl MemoryReducerCaller {
    pub(super) fn apply_create_work_package(
        &self,
        row: bindings::WorkPackage,
    ) -> Result<WorkPackageId> {
        validate_title(&row.title)
            .map_err(|why| anyhow::anyhow!("the shared store refused: {why}"))?;
        let mut tables = self.lock();
        let id = Self::assign_id(0, &mut tables.next_work_package_id);
        let row = module::WorkPackage {
            id,
            title: row.title.trim().to_string(),
            ..module::WorkPackage::from(row)
        };
        tables.work_packages.insert(id, row.clone());
        self.rows.upsert_work_package(&row.into());
        Ok(WorkPackageId(id))
    }

    pub(super) fn apply_patch_work_package(
        &self,
        id: WorkPackageId,
        patch: bindings::WorkPackagePatch,
    ) -> Result<ReducerOutcome> {
        let patch = module::WorkPackagePatch::from(patch);
        let mut tables = self.lock();
        let Some(mut row) = tables.work_packages.get(&id.0).cloned() else {
            return Ok(ReducerOutcome::Refused(format!("no work package {id}")));
        };
        if let Some(title) = patch.title {
            if let Err(why) = validate_title(&title) {
                return Ok(ReducerOutcome::Refused(why));
            }
            row.title = title.trim().to_string();
        }
        if let Some(description) = patch.description {
            row.description = description;
        }
        row.updated_at = self.now();
        tables.work_packages.insert(id.0, row.clone());
        self.rows.upsert_work_package(&row.into());
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_delete_work_package(&self, id: WorkPackageId) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let links: Vec<String> = tables
            .work_package_links
            .values()
            .filter(|l| l.work_package_id == id.0)
            .map(|l| l.id.clone())
            .collect();
        self.delete_work_package_link_rows(&mut tables, links);
        if tables.work_packages.remove(&id.0).is_some() {
            self.rows.remove_work_package(id);
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_link_work_package_item(
        &self,
        id: WorkPackageId,
        item: WorkPackageItem,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        if !tables.work_packages.contains_key(&id.0) {
            return Ok(ReducerOutcome::Refused(format!("no work package {id}")));
        }
        let exists = match item {
            WorkPackageItem::Task(t) => tables.tasks.contains_key(&t.0),
            WorkPackageItem::Epic(e) => tables.epics.contains_key(&e.0),
        };
        if !exists {
            return Ok(ReducerOutcome::Refused(format!(
                "no {} {}",
                item.kind(),
                item.raw_id()
            )));
        }
        let link_id = module::work_package_link_id(id.0, item.kind(), item.raw_id());
        let created_at = self.now();
        if let std::collections::btree_map::Entry::Vacant(slot) =
            tables.work_package_links.entry(link_id.clone())
        {
            let row = module::WorkPackageLink {
                id: link_id,
                work_package_id: id.0,
                kind: item.kind().to_string(),
                item_id: item.raw_id(),
                created_at,
            };
            slot.insert(row.clone());
            self.rows.upsert_work_package_link(&row.into());
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_unlink_work_package_item(
        &self,
        id: WorkPackageId,
        item: WorkPackageItem,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let link_id = module::work_package_link_id(id.0, item.kind(), item.raw_id());
        self.delete_work_package_link_rows(&mut tables, vec![link_id]);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    /// Mirrors the module's `delete_work_package_links_to`: every link naming
    /// this item, in every package. Called wherever a task or epic row goes.
    pub(super) fn delete_work_package_links_to(
        &self,
        tables: &mut Tables,
        kind: &str,
        item_id: i64,
    ) {
        let links: Vec<String> = tables
            .work_package_links
            .values()
            .filter(|l| l.kind == kind && l.item_id == item_id)
            .map(|l| l.id.clone())
            .collect();
        self.delete_work_package_link_rows(tables, links);
    }

    fn delete_work_package_link_rows(&self, tables: &mut Tables, ids: Vec<String>) {
        for id in ids {
            if tables.work_package_links.remove(&id).is_some() {
                self.rows.remove_work_package_link(id);
            }
        }
    }
}
