//! The board's relationship with the shared store: identity, subscriptions and
//! the connection that carries them.
//!
//! Spec: [`docs/specs/sync.allium`](../../docs/specs/sync.allium), plus
//! `docs/specs/host.allium` for the identity half.
//!
//! # The two identities
//!
//! A **host id** is this machine. It is minted offline on first run, never
//! leaves, and is what `core/Task.host` names — "which disk holds this
//! worktree?". A **user identity** is the person. It is minted by the shared
//! store on first connection, is the same value on every machine that person
//! uses, and is what `core/Task.owner` names — "whose board does this sit on?".
//!
//! One person with two laptops has one user identity and two host ids. Reading
//! either as the other puts a teammate's card on your board, or aims a dispatch
//! at a worktree on somebody else's disk. Nothing in this module mixes them, and
//! adding an owner to a machine changes no locality gate: those still compare
//! machine against machine.
//!
//! # Shape of the module
//!
//! - [`identity`] — the three-way verdict on an identity the store offers.
//!   Pure, and the only fatal decision in the subsystem.
//! - [`connection`] — the connection lifecycle as a state machine. No I/O, no
//!   clock of its own; the caller supplies both the event and the instant.
//! - [`connector`] — the seam to a real store. One trait, so the state machine
//!   above can be driven against a fake that fails on command.
//!
//! # What this module does NOT do
//!
//! It does not repoint the board's reads. The board still reads and writes its
//! local store; this builds the connection beside it. Moving the read path is a
//! later change with its own spec work — see `sync.allium`'s Excludes.
//!
//! **The Rust SDK has no auto-reconnect.** That is not a gap being worked
//! around here, it is the reason [`connection`] exists at all: without the
//! retry loop, a board needs restarting after every wifi handover.

/// The name of the shared database on whichever server a board is pointed at.
///
/// Fixed rather than configurable: a server hosts many databases, and the one
/// dispatch means is always this one. Making it a knob would add a way to point
/// two boards at the same server and have them silently not share anything.
pub const SHARED_DATABASE_NAME: &str = "dispatch";

pub mod board_reads;
pub mod connection;
pub mod connector;
pub mod decode;
pub mod identity;
pub mod rows;
pub mod sdk_connector;
pub mod session;

#[cfg(test)]
mod tests;

pub use board_reads::{BoardReads, LocalBoardReads, SubscriptionBoardReads};
pub use connection::{
    backoff, BoardConnection, ConnectionEvent, ConnectionStatus, CONNECT_TIMEOUT,
    RECONNECT_BACKOFF_BASE, RECONNECT_BACKOFF_MAX,
};
pub use connector::{Accepted, ConnectError, StoreConnector, SubscriptionRequest};
pub use decode::DecodeError;
pub use identity::{identity_conflict_message, settle_identity, IdentityVerdict};
pub use rows::{HostRow, RepoBaseBranchRow, RepoPathRow, SharedRows};
pub use sdk_connector::SpacetimeSdkConnector;
pub use session::{StepOutcome, SyncSession, SyncStore};
