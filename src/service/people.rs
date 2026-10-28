//! People and display names (`docs/specs/workspaces.allium`): the service a
//! keypress and an MCP call both set the caller's own name through. The data
//! rules (trimmed, non-empty, at most 64 characters) live in the store's
//! `set_display_name` reducer.

use std::sync::Arc;

use crate::store;

use super::ServiceError;

pub struct PeopleService {
    db: Arc<dyn store::PeopleStore>,
}

impl PeopleService {
    pub fn new(db: Arc<dyn store::PeopleStore>) -> Self {
        Self { db }
    }

    /// `SetDisplayName`: answers the saved (trimmed) name.
    pub async fn set_display_name(&self, name: &str) -> Result<String, ServiceError> {
        Ok(self.db.set_display_name(name).await?)
    }
}
