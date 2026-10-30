//! Work packages (`docs/specs/work-packages.allium`): this store's
//! reimplementation of `spacetime/module/src/packages.rs`.

use anyhow::Result;

use dispatch_spacetime_module as module;

use crate::models::{WorkPackageId, WorkPackageItem};
use crate::spacetime::bindings;

use super::super::writes::{ReducerOutcome, StoreRefusal};
use super::{MemoryReducerCaller, Tables};

impl MemoryReducerCaller {
    pub(super) fn apply_create_work_package(
        &self,
        row: bindings::WorkPackage,
    ) -> Result<WorkPackageId> {
        module::validate_work_package_title(&row.title)
            .map_err(|why| anyhow::anyhow!("the shared store refused: {why}"))?;
        let scope = self.scope();
        let mut tables = self.lock();
        tables
            .check_new_package(&scope, &module::WorkPackage::from(row.clone()))
            .map_err(|why| anyhow::anyhow!("the shared store refused: {why}"))?;
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

    /// Twin of `create_work_package_with_links`: everything is validated under
    /// the one lock before the first insert, because a `BTreeMap` cannot roll
    /// back.
    pub(super) fn apply_create_work_package_with_links(
        &self,
        row: bindings::WorkPackage,
        links: Vec<WorkPackageItem>,
    ) -> Result<WorkPackageId> {
        module::validate_work_package_title(&row.title).map_err(StoreRefusal)?;
        let scope = self.scope();
        let mut tables = self.lock();
        let candidate = module::WorkPackage::from(row.clone());
        tables
            .check_new_package(&scope, &candidate)
            .map_err(StoreRefusal)?;
        for item in &links {
            Self::require_item(&tables, *item).map_err(StoreRefusal)?;
            tables
                .check_link_allowed(&scope, &candidate, item.kind(), item.raw_id())
                .map_err(StoreRefusal)?;
        }
        let id = Self::assign_id(0, &mut tables.next_work_package_id);
        let row = module::WorkPackage {
            id,
            title: row.title.trim().to_string(),
            ..module::WorkPackage::from(row)
        };
        tables.work_packages.insert(id, row.clone());
        self.rows.upsert_work_package(&row.into());
        for item in links {
            self.insert_link_if_absent(&mut tables, WorkPackageId(id), item);
        }
        Ok(WorkPackageId(id))
    }

    /// Twin of `update_work_package_with_links`, validated up front like
    /// [`Self::apply_create_work_package_with_links`]. A patch that writes
    /// nothing leaves `updated_at` alone.
    pub(super) fn apply_update_work_package_with_links(
        &self,
        id: WorkPackageId,
        patch: bindings::WorkPackagePatch,
        unlink: Vec<WorkPackageItem>,
        link: Vec<WorkPackageItem>,
    ) -> Result<ReducerOutcome> {
        let patch = module::WorkPackagePatch::from(patch);
        let scope = self.scope();
        let mut tables = self.lock();
        let Some(mut row) = tables.visible_package(&scope, id.0) else {
            return Ok(ReducerOutcome::Refused(format!("no work package {id}")));
        };
        if let Some(both) = link.iter().find(|i| unlink.contains(i)) {
            return Ok(ReducerOutcome::Refused(format!(
                "{} {} is in both a link and an unlink list; name it in one",
                both.kind(),
                both.raw_id()
            )));
        }
        for item in &link {
            if let Err(why) = Self::require_item(&tables, *item) {
                return Ok(ReducerOutcome::Refused(why));
            }
            if let Err(why) = tables.check_link_allowed(&scope, &row, item.kind(), item.raw_id()) {
                return Ok(ReducerOutcome::Refused(why));
            }
        }
        if !patch.is_empty() {
            if let Err(why) = module::apply_work_package_patch(&mut row, patch) {
                return Ok(ReducerOutcome::Refused(why));
            }
            row.updated_at = self.now();
            tables.work_packages.insert(id.0, row.clone());
            self.rows.upsert_work_package(&row.into());
        }
        let unlink_ids = unlink
            .iter()
            .map(|i| module::work_package_link_id(id.0, i.kind(), i.raw_id()))
            .collect();
        self.delete_work_package_link_rows(&mut tables, unlink_ids);
        for item in link {
            self.insert_link_if_absent(&mut tables, id, item);
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_patch_work_package(
        &self,
        id: WorkPackageId,
        patch: bindings::WorkPackagePatch,
    ) -> Result<ReducerOutcome> {
        let patch = module::WorkPackagePatch::from(patch);
        let scope = self.scope();
        let mut tables = self.lock();
        let Some(mut row) = tables.visible_package(&scope, id.0) else {
            return Ok(ReducerOutcome::Refused(format!("no work package {id}")));
        };
        if let Err(why) = module::apply_work_package_patch(&mut row, patch) {
            return Ok(ReducerOutcome::Refused(why));
        }
        row.updated_at = self.now();
        tables.work_packages.insert(id.0, row.clone());
        self.rows.upsert_work_package(&row.into());
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_delete_work_package(&self, id: WorkPackageId) -> Result<ReducerOutcome> {
        let scope = self.scope();
        let mut tables = self.lock();
        if tables.visible_package(&scope, id.0).is_none() {
            return Ok(ReducerOutcome::Applied(vec![]));
        }
        self.delete_package_rows(&mut tables, id);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    /// The package row and its links, with no sight check.
    pub(super) fn delete_package_rows(&self, tables: &mut Tables, id: WorkPackageId) {
        let links: Vec<String> = tables
            .work_package_links
            .values()
            .filter(|l| l.work_package_id == id.0)
            .map(|l| l.id.clone())
            .collect();
        self.delete_work_package_link_rows(tables, links);
        if tables.work_packages.remove(&id.0).is_some() {
            self.rows.remove_work_package(id);
        }
    }

    pub(super) fn apply_link_work_package_item(
        &self,
        id: WorkPackageId,
        item: WorkPackageItem,
    ) -> Result<ReducerOutcome> {
        let scope = self.scope();
        let mut tables = self.lock();
        if !tables.work_packages.contains_key(&id.0) {
            return Ok(ReducerOutcome::Refused(format!("no work package {id}")));
        }
        if let Err(why) = Self::require_item(&tables, item) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let Some(package) = tables.visible_package(&scope, id.0) else {
            return Ok(ReducerOutcome::Refused(format!("no work package {id}")));
        };
        if let Err(why) = tables.check_link_allowed(&scope, &package, item.kind(), item.raw_id()) {
            return Ok(ReducerOutcome::Refused(why));
        }
        self.insert_link_if_absent(&mut tables, id, item);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    /// The module's `validate_item_exists`, with its wording.
    fn require_item(tables: &Tables, item: WorkPackageItem) -> Result<(), String> {
        let exists = match item {
            WorkPackageItem::Task(t) => tables.tasks.contains_key(&t.0),
            WorkPackageItem::Epic(e) => tables.epics.contains_key(&e.0),
        };
        if exists {
            Ok(())
        } else {
            Err(format!("no {} {}", item.kind(), item.raw_id()))
        }
    }

    /// The module's `insert_link_if_absent`; the caller has validated.
    fn insert_link_if_absent(&self, tables: &mut Tables, id: WorkPackageId, item: WorkPackageItem) {
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
    }

    pub(super) fn apply_unlink_work_package_item(
        &self,
        id: WorkPackageId,
        item: WorkPackageItem,
    ) -> Result<ReducerOutcome> {
        let scope = self.scope();
        let mut tables = self.lock();
        if tables.visible_package(&scope, id.0).is_none() {
            return Ok(ReducerOutcome::Applied(vec![]));
        }
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
