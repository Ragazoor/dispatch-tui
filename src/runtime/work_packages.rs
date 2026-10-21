//! Work-package side effects (`docs/specs/work-packages.allium`): the board's
//! writes go through the work-package service, then the board re-reads its
//! packages so a refused write cannot leave a check mark the store disagrees
//! with.

use super::*;
use crate::tui::commands::WorkPackageCommand;

impl TuiRuntime {
    pub(super) async fn exec_work_package_command(&self, app: &mut App, cmd: WorkPackageCommand) {
        use crate::service::{CreateWorkPackageParams, UpdateWorkPackageParams};
        let (what, result) = match cmd {
            WorkPackageCommand::Link { id, item } => (
                "linking to work package",
                self.work_package_svc
                    .update_work_package(UpdateWorkPackageParams {
                        link: vec![item],
                        ..UpdateWorkPackageParams::for_package(id)
                    })
                    .await,
            ),
            WorkPackageCommand::Unlink { id, item } => (
                "unlinking from work package",
                self.work_package_svc
                    .update_work_package(UpdateWorkPackageParams {
                        unlink: vec![item],
                        ..UpdateWorkPackageParams::for_package(id)
                    })
                    .await,
            ),
            WorkPackageCommand::Create { title, item } => (
                "creating work package",
                self.work_package_svc
                    .create_work_package(CreateWorkPackageParams {
                        title,
                        description: String::new(),
                        link: vec![item],
                    })
                    .await
                    .map(|_| ()),
            ),
            WorkPackageCommand::Rename { id, title } => (
                "renaming work package",
                self.work_package_svc
                    .update_work_package(UpdateWorkPackageParams {
                        title: Some(title),
                        ..UpdateWorkPackageParams::for_package(id)
                    })
                    .await,
            ),
            WorkPackageCommand::SetSortOrder { id, sort_order } => (
                "reordering work package",
                self.work_package_svc
                    .update_work_package(UpdateWorkPackageParams {
                        sort_order: Some(sort_order),
                        ..UpdateWorkPackageParams::for_package(id)
                    })
                    .await,
            ),
            WorkPackageCommand::Delete(id) => (
                "deleting work package",
                self.work_package_svc.delete_work_package(id).await,
            ),
        };
        if let Err(e) = result {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error(what, e),
            )));
        }
        self.exec_refresh_work_packages(app).await;
    }

    /// Link a task just created from a package view to that package
    /// (`work-packages.allium: CreateTaskInPackageView`). A refused link keeps
    /// the task and shows the error, like any other link from the board.
    pub(super) async fn link_new_task(
        &self,
        app: &mut App,
        package: Option<models::WorkPackageId>,
        task: models::TaskId,
    ) {
        if let Some(id) = package {
            self.exec_work_package_command(
                app,
                WorkPackageCommand::Link {
                    id,
                    item: models::WorkPackageItem::Task(task),
                },
            )
            .await;
        }
    }

    /// Re-read the board's packages and links through `board_reads`, the read
    /// seam every card on screen comes through.
    pub(super) async fn exec_refresh_work_packages(&self, app: &mut App) {
        let packages = self.board_reads.list_work_packages().await;
        let links = self.board_reads.list_work_package_links().await;
        app.update(Message::WorkPackage(
            crate::tui::messages::WorkPackageMessage::Refresh { packages, links },
        ));
    }
}
