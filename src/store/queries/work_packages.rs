//! Work packages (`docs/specs/work-packages.allium`): reads from the rows,
//! writes as reducer calls.

use anyhow::Result;

use crate::models::{WorkPackage, WorkPackageId, WorkPackageItem, WorkPackageLink};
use crate::spacetime::bindings;
use crate::sync::encode;

use super::super::Store;

#[async_trait::async_trait]
impl super::super::WorkPackageRead for Store {
    async fn list_work_packages(&self) -> Vec<WorkPackage> {
        self.rows.work_packages()
    }

    async fn list_work_package_links(&self) -> Vec<WorkPackageLink> {
        self.rows.work_package_links()
    }
}

#[async_trait::async_trait]
impl super::super::WorkPackageCrud for Store {
    async fn create_work_package(&self, title: &str, description: &str) -> Result<WorkPackageId> {
        // `sync.allium: CreatesRequireASettledIdentity`: the board subscribes
        // to packages by `created_by`, so a package with no name stamped on it
        // would be created and then never seen again.
        let identity = self
            .require_identity("there is no name to stamp on a new work package; it was not created")
            .await?;
        let now = self.now();
        let row = encode::create_work_package_row(title, description, &identity, &now);
        self.caller.create_work_package(row).await
    }

    async fn patch_work_package(
        &self,
        id: WorkPackageId,
        title: Option<&str>,
        description: Option<&str>,
    ) -> Result<()> {
        if title.is_none() && description.is_none() {
            return Ok(());
        }
        let patch = bindings::WorkPackagePatch {
            title: title.map(str::to_string),
            description: description.map(str::to_string),
        };
        self.caller.patch_work_package(id, patch).await?.applied()
    }

    async fn delete_work_package(&self, id: WorkPackageId) -> Result<()> {
        self.caller.delete_work_package(id).await?.applied()
    }

    async fn link_work_package_item(&self, id: WorkPackageId, item: WorkPackageItem) -> Result<()> {
        self.caller
            .link_work_package_item(id, item)
            .await?
            .applied()
    }

    async fn unlink_work_package_item(
        &self,
        id: WorkPackageId,
        item: WorkPackageItem,
    ) -> Result<()> {
        self.caller
            .unlink_work_package_item(id, item)
            .await?
            .applied()
    }
}
