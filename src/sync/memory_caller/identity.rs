//! Who the store records as acting: this store's reimplementation of
//! `spacetime/module/src/identity.rs` (`store-split.allium:
//! IdentitiesAreTheCallers`). One person uses the store, so nothing here
//! withholds a row from anyone.
//!
//! OFF BY DEFAULT. A store built with [`MemoryReducerCaller::new`] runs as
//! the board did before identities were stamped: a client-named identity is
//! kept and no own-identity check refuses. Most board tests are about
//! something else, so enforcing here would make each of them name a sender.
//! A test of the rules, and the conformance scenarios that pair this store
//! with the module, turn it on with [`MemoryReducerCaller::with_identity_rules`].

use dispatch_spacetime_module as module;

use super::{MemoryReducerCaller, Tables};

/// The rules in force for one call: whether they are on, and who is calling.
#[derive(Clone)]
pub(super) struct Scope {
    on: bool,
    who: String,
}

impl MemoryReducerCaller {
    /// Turn the identity rules on: a write records the caller's own identity
    /// whatever the row claims, and a subscriber or bookmark names the caller.
    pub fn with_identity_rules(mut self) -> Self {
        self.identity_rules = true;
        self
    }

    pub(super) fn scope(&self) -> Scope {
        Scope {
            on: self.identity_rules,
            who: self.sender.clone(),
        }
    }
}

impl Scope {
    /// The module's `caller_stamp`: the identity a write records for the
    /// caller, whatever the row claims. A memory store has no operator, and
    /// with the rules off it runs as the board did before identities existed.
    pub(super) fn stamp(&self, claimed: String) -> String {
        if self.on {
            module::stamped_identity(false, &self.who, claimed)
        } else {
            claimed
        }
    }
}

impl Tables {
    /// Everyone a signalled task marks: every known person (every profile),
    /// the module's `mark_task_unseen_for_everyone`. With the rules off the
    /// only person this double knows by name may have no profile, so the
    /// sender is added.
    pub(super) fn known_people(&self, scope: &Scope) -> std::collections::BTreeSet<String> {
        let mut viewers: std::collections::BTreeSet<String> =
            self.user_profiles.keys().cloned().collect();
        if !scope.on && !scope.who.is_empty() {
            viewers.insert(scope.who.clone());
        }
        viewers
    }

    /// The row a new learning is stored as: the dead `workspace_id` at its
    /// default, and the author stamped with the caller when the rules are on.
    pub(super) fn settle_new_learning(
        &self,
        scope: &Scope,
        row: module::Learning,
    ) -> module::Learning {
        let row = module::Learning {
            workspace_id: 0,
            ..row
        };
        if !scope.on {
            return row;
        }
        module::Learning {
            author: scope.who.clone(),
            ..row
        }
    }

    /// The module's `require_epic`: the epic `id`, or the refusal naming it
    /// missing.
    pub(super) fn require_epic(&self, id: i64) -> Result<&module::Epic, String> {
        self.epics
            .get(&id)
            .ok_or_else(|| format!("epic {id} does not exist"))
    }

    /// The module's `check_epic_exists`.
    pub(super) fn check_epic_exists(&self, label: &str, epic_id: i64) -> Result<(), String> {
        if epic_id == 0 {
            return Ok(());
        }
        self.require_epic(epic_id)
            .map(|_| ())
            .map_err(|why| format!("{label}: {why}"))
    }

    /// The module's `settle_new_epic`: the parent must exist, the dead
    /// columns take their defaults, and with the rules on the caller is the
    /// creator and the setter of any command the row carries.
    pub(super) fn settle_new_epic(
        &self,
        scope: &Scope,
        row: module::Epic,
    ) -> Result<module::Epic, String> {
        if row.parent_epic_id != 0 {
            self.require_epic(row.parent_epic_id)?;
        }
        let blank = module::blank_epic();
        let row = module::Epic {
            workspace_id: blank.workspace_id,
            visibility: blank.visibility,
            ..row
        };
        if !scope.on {
            return Ok(row);
        }
        let created_by = scope.who.clone();
        let feed_command_set_by = if row.feed_command.is_empty() {
            String::new()
        } else {
            created_by.clone()
        };
        Ok(module::Epic {
            created_by,
            feed_command_set_by,
            ..row
        })
    }

    /// The module's `settle_new_package`.
    pub(super) fn settle_new_package(
        &self,
        scope: &Scope,
        row: module::WorkPackage,
    ) -> module::WorkPackage {
        let blank = module::blank_work_package();
        module::WorkPackage {
            workspace_id: blank.workspace_id,
            visibility: blank.visibility,
            created_by: scope.stamp(row.created_by.clone()),
            ..row
        }
    }

    /// `require_own_identity`: a subscriber is the caller's own identity. Off
    /// without the identity rules, as every other identity check here is.
    pub(super) fn require_own_identity(scope: &Scope, subscriber: &str) -> Result<(), String> {
        if !scope.on || subscriber == scope.who {
            Ok(())
        } else {
            Err("a subscriber must be the caller's own identity".into())
        }
    }

    /// The module's `write_bookmark` identity rule: only the person themselves,
    /// the operator included. This double has no operator, so it is the
    /// caller or nobody. Off without the identity rules.
    pub(super) fn require_bookmark_owner(scope: &Scope, subscriber: &str) -> Result<(), String> {
        if !scope.on || subscriber == scope.who {
            Ok(())
        } else {
            Err("a bookmark is written only by the person it belongs to".into())
        }
    }
}
