//! Sending a shared mutation to the store.
//!
//! Spec: `docs/specs/sync.allium`'s `BoardWritesThroughTheStore`,
//! `AWriteWithNoConnectionIsRefused` and `StoreRejectsAnInvalidMutation`.
//!
//! This is the implementation of [`crate::db::SharedWriter`] that a configured
//! board runs. [`ReducerWriter`] holds the two things a mutation needs that the
//! store cannot supply — who is writing, and when — encodes the row, and hands
//! it to a [`ReducerCaller`].
//!
//! # Why the transport is a second trait
//!
//! Everything interesting here is the encoding and the refusal path, and
//! neither needs a server. Splitting the transport out means the tests for both
//! run in CI, where no SpacetimeDB exists — the same reason
//! [`super::StoreConnector`] is a trait. [`SdkReducerCaller`] is the one
//! implementation that talks to a real store.
//!
//! # Nothing here retries
//!
//! A refusal is returned, once, to the caller that asked. There is no buffer to
//! put it in and no loop to put it through: `sync.allium: NoWriteIsEverQueued`,
//! and the reasoning is in its guidance — a replayed write carries an intent
//! formed against a board state that no longer holds, and the operator was told
//! it had already worked.

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::db::{CreateTaskRequest, SharedWriter, TaskPatch};
use crate::models::TaskId;
use crate::spacetime::bindings;

use super::encode;

/// One reducer call, and the store's verdict on it.
///
/// A method per reducer rather than one `call(name, args)`, so the argument
/// types are the generated ones and a signature that drifts from the module is
/// a compile error rather than a runtime decode failure.
#[async_trait]
pub trait ReducerCaller: Send + Sync {
    /// Insert a task and answer with the id the store generated.
    ///
    /// The one call here that returns something. A reducer cannot answer, so
    /// the id is read back off the row as it arrives — see
    /// [`SdkReducerCaller::create_task`] for how, and for what that costs.
    async fn create_task(&self, row: bindings::Task) -> Result<TaskId>;
    async fn patch_task(&self, id: TaskId, patch: bindings::TaskPatch) -> Result<()>;
    async fn delete_task(&self, id: TaskId) -> Result<()>;
    async fn save_repo_path(&self, path: String, last_used: String) -> Result<()>;
}

/// Who the board is writing as.
///
/// # Why this is read per write rather than resolved at bootstrap
///
/// The obvious design is to resolve it once and hold the string. It does not
/// work: the writer is built at bootstrap, the identity is settled by the
/// CONNECTION, and the connection has not happened yet. A string captured then
/// would be whatever the last run stored, or nothing at all on a first run.
///
/// So this is a reader, consulted by the one mutation that needs an answer.
/// Creates are rare enough that a settings lookup on that path costs nothing
/// worth designing around, and everything else — every patch, every delete —
/// never asks.
#[async_trait]
pub trait WriterIdentity: Send + Sync {
    /// The user this board writes as, or `None` before it has ever connected.
    async fn user(&self) -> Result<Option<String>>;
}

/// The identity this board's connection settled on, once it has.
///
/// # Why a cell rather than a read of the local store
///
/// The local store does hold it — the handshake writes it there
/// (`host.allium: AdoptUserIdentity`) — but reading it from there at bootstrap
/// is not possible: the writer is built before the `Database` it would read,
/// and the two would refer to each other. Filling a cell afterwards breaks that
/// and is the truer statement besides. What a write may stamp is the identity
/// THIS CONNECTION settled, not whatever a previous run left on disk: a board
/// that has not connected must not create user-board tasks under a name it
/// cannot currently prove.
///
/// Empty until the first successful connection, and never emptied again. A
/// dropped connection does not clear it, because the person did not change —
/// and a write during the outage is refused by the transport anyway.
#[derive(Default)]
pub struct SettledIdentity(std::sync::Mutex<Option<String>>);

impl SettledIdentity {
    /// Record who the store said we are. Called once per successful connection.
    pub fn settle(&self, user: impl Into<String>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(user.into());
    }
}

#[async_trait]
impl WriterIdentity for SettledIdentity {
    async fn user(&self) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }
}

/// Shared mutations, as reducer calls.
pub struct ReducerWriter {
    caller: Arc<dyn ReducerCaller>,
    identity: Arc<dyn WriterIdentity>,
    clock: Arc<dyn crate::service::Clock>,
}

impl ReducerWriter {
    pub fn new(
        caller: Arc<dyn ReducerCaller>,
        identity: Arc<dyn WriterIdentity>,
        clock: Arc<dyn crate::service::Clock>,
    ) -> Self {
        Self {
            caller,
            identity,
            clock,
        }
    }

    /// This board's clock, in the store's timestamp format.
    ///
    /// The CREATE timestamps are the client's, deliberately, and they are the
    /// only ones that are. `created_at` records when the person asked, which is
    /// a fact about this machine; everything a reducer derives afterwards —
    /// `updated_at`, `completed_at`, the claim's seeded `last_pre_tool_use_at`
    /// — uses the store's clock, so that two boards' rows are ordered by one
    /// clock rather than by whose laptop is fast.
    fn now(&self) -> String {
        self.clock.now().format("%Y-%m-%d %H:%M:%S%.3f").to_string()
    }
}

#[async_trait]
impl SharedWriter for ReducerWriter {
    async fn create_task(&self, req: CreateTaskRequest<'_>) -> Result<TaskId> {
        // AN EPIC-LESS TASK NEEDS AN OWNER and a board that has never connected
        // has none, so this is refused here rather than sent and rejected. The
        // module would refuse it too (`write_task`), but the message an
        // operator can act on is this one. `core.allium:
        // OwnerTracksUserBoardTask`.
        let owner = match self.identity.user().await? {
            Some(user) => user,
            None if req.epic_id.is_none() => anyhow::bail!(
                "this board has no user identity yet, so a task with no epic has no board                  to sit on; it was not created"
            ),
            // A task in an epic carries no owner, so an unsettled identity is
            // not in its way.
            None => String::new(),
        };
        let row = encode::create_task_row(&req, &owner, &self.now());
        self.caller.create_task(row).await
    }

    async fn patch_task(&self, id: TaskId, patch: &TaskPatch<'_>) -> Result<()> {
        self.caller.patch_task(id, encode::task_patch(patch)).await
    }

    async fn delete_task(&self, id: TaskId) -> Result<()> {
        self.caller.delete_task(id).await
    }

    async fn save_repo_path(&self, path: &str) -> Result<()> {
        self.caller
            .save_repo_path(path.to_string(), self.now())
            .await
    }
}
