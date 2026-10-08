//! [`StartupAbort`] — every condition that stops the board before it draws.
//!
//! A leaf module so the layers that discover an abort — the host file
//! (`src/host_file/`) and the managed store (`src/spacetime/managed_store.rs`)
//! — depend on it downward rather than on `startup`. The operator-facing
//! wording (`StartupAbort::message`, `Display`) lives in
//! `src/startup/launch.rs`, beside the session and window names it cites.

/// Every condition that stops the board before it draws.
/// `startup.allium`'s `StartupAbortReason`.
///
/// One enumeration rather than a message raised wherever each is discovered, so
/// `StartupAbortsOnlyOnAnUnusableSubstrate` has somewhere to be read off: a new
/// way to abort means a variant here, in front of the invariant that says
/// whether it belongs at startup at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupAbort {
    /// No tmux on `PATH`.
    TmuxUnavailable,
    /// tmux was reached and refused to start or attach.
    LaunchRejected,
    /// The launch came from a pane inside the board's own window.
    BoardAlreadyInThisWindow,
    /// The previous board's window would not close.
    PreviousBoardNotRetired,
    /// tmux would not say which session this process is in.
    SessionUnidentified,
    /// Another process holds the port agents reach the board on.
    AgentPortUnavailable { port: u16 },
    /// This machine has no label and nobody could be asked for one.
    /// `startup.allium`'s `AbortWhenTheHostIsUnnamedAndNoOneCanAnswer`.
    HostUnnamed,
    /// The host file this machine's Host row lives in is unusable — either
    /// it exists and could not be read or parsed (and is never overwritten),
    /// a first run's new one could not be written, or the machine is unnamed
    /// and a label it was just given could not be written back. All are the
    /// same broken substrate with the same remedy (repair host.json or its
    /// directory), which is why one reason covers both rather than a second
    /// value alongside `HostUnnamed` (which means the identity read fine,
    /// the store would accept a label, and there simply isn't one yet).
    /// `startup.allium`'s `AbortWhenTheHostIdentityStoreIsUnusable`.
    HostIdentityUnavailable,
    /// No store is named and the `spacetime` CLI is not on `PATH`.
    /// `startup.allium`'s `AbortWhenTheManagedStoreHasNoCli`.
    SpacetimeCliMissing,
    /// No store is named and the managed address is held by something that
    /// is not a store. `AbortWhenTheManagedAddressIsHeldByAnotherProgram`.
    ManagedStorePortTaken { address: String },
    /// The started managed store exited or did not serve HTTP in time.
    /// `StartTheManagedStoreWhenNothingAnswers`. Carries the store's error and
    /// whether the abort's stop finished in time (`false`: the store may still
    /// be running, and the message must not say otherwise).
    ManagedStoreDidNotStart { reason: String, stopped: bool },
    /// The embedded module would be accepted only by deleting data.
    /// `AbortWhenTheModuleCannotMigrateAutomatically`.
    ModuleNeedsManualMigration { reason: String },
    /// Publishing the embedded module failed for any other reason.
    /// `AbortWhenTheModulePublishFails`.
    ModulePublishFailed { reason: String },
    /// A store is named and the first connection to it failed, or it
    /// identified this install as somebody else. `startup.allium`'s
    /// `AbortWhenTheStoreCannotBeReached`. Carries the attempt's own reason.
    StoreUnavailable { reason: String },
    /// The store holds a different database from the one this install's
    /// boards last used. `AbortWhenTheStoreIsNotTheOneThisInstallUses`.
    StoreSwitched {
        address: String,
        pinned: String,
        found: String,
    },
}
