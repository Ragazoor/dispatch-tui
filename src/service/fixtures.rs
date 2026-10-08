//! Test fixtures for the service layer's create params.
//!
//! Gated like `crate::models::TaskBuilder`: `cfg(any(test, feature =
//! "test-support"))`. A test spells out only the fields it cares about and
//! takes the rest with struct-update syntax:
//! `CreateEpicParams { parent_epic_id: Some(p), ..CreateEpicParams::fixture("Child") }`.

use super::CreateEpicParams;

impl CreateEpicParams {
    /// A top-level epic with no description, sort order or feed.
    pub fn fixture(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: String::new(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_epic_params_fixture_is_a_plain_top_level_epic() {
        let p = CreateEpicParams::fixture("E");
        assert_eq!(p.title, "E");
        assert_eq!(p.description, "");
        assert_eq!(p.sort_order, None);
        assert_eq!(p.parent_epic_id, None);
        assert_eq!(p.feed_command, None);
        assert_eq!(p.feed_interval_secs, None);
    }
}
