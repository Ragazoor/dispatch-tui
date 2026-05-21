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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn project_id_from_str_valid() {
        let id: ProjectId = "42".parse().expect("should parse");
        assert_eq!(id, ProjectId(42));
    }

    #[test]
    fn project_id_from_str_negative() {
        let id: ProjectId = "-1".parse().expect("negative should parse");
        assert_eq!(id, ProjectId(-1));
    }

    #[test]
    fn project_id_from_str_invalid() {
        assert!("not-a-number".parse::<ProjectId>().is_err());
        assert!("".parse::<ProjectId>().is_err());
    }
}
