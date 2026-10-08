//! Where the board gets the rows it draws.
//!
//! Spec: `docs/specs/sync.allium`'s `BoardReadsFromTheSubscription`.
//!
//! # One read path for cards
//!
//! [`BoardReads`] is the handle the board draws its cards from — the reads
//! the row-change pump and the tick's revision guard refresh. Its card reads
//! are the store's own read traits ([`TaskRead`], [`EpicRead`],
//! [`RepoConfigRead`]), so there is one `get_task`, not a second copy on a
//! second trait; this trait adds only what drawing needs beyond them: the
//! poll owner and the revision.
//!
//! [`crate::store::Store`] is the one implementation, answering from the
//! subscription's rows. The TUI holds the same `Store` twice — as
//! `dyn BoardReads` for cards and as `dyn TaskReadStore` for everything else —
//! so a card read through either answers from the same rows; the separate
//! handle is what makes "the board draws from here" visible at the call site.
//!
//! # The revision number
//!
//! [`BoardReads::revision`] is the cheap "has anything changed?": the
//! subscription's generation. The tick-driven refresh compares it before
//! re-reading, which is what keeps a speculative refresh free. Nothing
//! persists one, so a fresh connection simply costs one extra refresh.

use anyhow::Result;
use async_trait::async_trait;

use crate::models::PollScopeId;
use crate::store::{EpicRead, RepoConfigRead, TaskRead};

/// The reads a board performs to draw itself.
///
/// Read-only by construction: the supertraits are the read halves of the
/// task, epic and repo-config domains, so a `dyn BoardReads` reaches no
/// mutation.
#[async_trait]
pub trait BoardReads: TaskRead + EpicRead + RepoConfigRead {
    /// The `Host.id` allowed to run recurring background polling for
    /// `target`, or `None` if unclaimed (`core.allium: PollOwner`). See
    /// `pr-workflow.allium: PollPrStatus` and `feeds.allium: FeedTick`, the two
    /// callers.
    async fn poll_owner(&self, target: PollScopeId) -> Result<Option<String>>;

    /// A number that changes when the rows do.
    ///
    /// `None` means "cannot tell" — take it as changed. Erring towards one
    /// wasted refresh is the right side to err on: the other side is a board
    /// that stops updating and says nothing.
    ///
    /// `Option<u64>` rather than a signed sentinel. The caller also has to
    /// represent "never read yet", and with one `-1` standing for both that
    /// value meant two different absences on the same line.
    async fn revision(&self) -> Option<u64>;
}
