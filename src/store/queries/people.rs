//! People and display names (`docs/specs/workspaces.allium`): the one write a
//! board makes about a person is their own name.

use anyhow::Result;

use super::super::Store;

#[async_trait::async_trait]
impl super::super::PeopleStore for Store {
    async fn set_display_name(&self, name: &str) -> Result<String> {
        self.caller
            .set_display_name(name.to_string())
            .await?
            .applied()?;
        Ok(name.trim().to_string())
    }
}
