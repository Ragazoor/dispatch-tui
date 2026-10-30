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

    async fn my_workspaces(&self) -> Vec<crate::models::WorkspaceRef> {
        let Ok(Some(identity)) = self.identity.user().await else {
            return Vec::new();
        };
        let mine: std::collections::HashSet<i64> = self
            .rows
            .memberships()
            .into_iter()
            .filter(|m| m.user == identity)
            .map(|m| m.workspace_id)
            .collect();
        self.rows
            .workspaces()
            .into_iter()
            .filter(|w| mine.contains(&w.id))
            .map(|w| crate::models::WorkspaceRef {
                id: w.id,
                name: w.name,
            })
            .collect()
    }
}
