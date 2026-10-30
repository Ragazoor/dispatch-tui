//! People and display names. Mirrors the module's `user_profile.rs`.

use anyhow::Result;

use dispatch_spacetime_module as module;

use super::super::writes::ReducerOutcome;
use super::MemoryReducerCaller;

impl MemoryReducerCaller {
    /// `workspaces.allium: RegisterUserProfile`: what the module's
    /// `client_connected` does for the identity this caller writes as. The
    /// real module runs it on every connect; a memory store has no connect
    /// event, so a fixture calls this where a connection would. Idempotent.
    pub fn register_user_profile(&self) {
        let identity = self.sender.clone();
        let mut tables = self.lock();
        if tables.user_profiles.contains_key(&identity) {
            return;
        }
        let row = module::UserProfile {
            identity: identity.clone(),
            display_name: String::new(),
            is_super_admin: false,
            created_at: self.now(),
        };
        tables.user_profiles.insert(identity, row.clone());
        self.rows.upsert_user_profile(&row.into());
    }

    /// A profile for ANOTHER person, as their own first connection would have
    /// made it: for a test whose subject is a second person in the directory.
    /// Idempotent; an existing profile keeps its name.
    pub fn seed_person(&self, identity: &str, display_name: &str) {
        let mut tables = self.lock();
        if tables.user_profiles.contains_key(identity) {
            return;
        }
        let row = module::UserProfile {
            identity: identity.to_string(),
            display_name: display_name.to_string(),
            is_super_admin: false,
            created_at: self.now(),
        };
        tables
            .user_profiles
            .insert(identity.to_string(), row.clone());
        self.rows.upsert_user_profile(&row.into());
    }

    pub(super) fn apply_set_display_name(&self, name: String) -> Result<ReducerOutcome> {
        let name = match module::validate_display_name(&name) {
            Ok(name) => name,
            Err(reason) => return Ok(ReducerOutcome::Refused(reason)),
        };
        let mut tables = self.lock();
        let Some(profile) = tables.user_profiles.get_mut(&self.sender) else {
            return Ok(ReducerOutcome::Refused(
                "no profile for this identity; reconnect and try again".into(),
            ));
        };
        profile.display_name = name;
        let row = profile.clone();
        self.rows.upsert_user_profile(&row.into());
        Ok(ReducerOutcome::Applied(vec![]))
    }
}
