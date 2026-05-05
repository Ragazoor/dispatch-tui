use serde::{Deserialize, Serialize};

define_id_newtype!(
    #[doc = "```compile_fail"]
    #[doc = "use dispatch_tui::models::{ProjectId, EpicId};"]
    #[doc = "fn takes_epic(id: EpicId) {}"]
    #[doc = "takes_epic(ProjectId(1)); // must not compile"]
    #[doc = "```"]
    ProjectId,
    project_id_tests
);

#[derive(Debug, Clone)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub sort_order: i64,
    pub is_default: bool,
}
