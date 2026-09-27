//! Which epics a board asks for beyond the ones it follows.
//!
//! Spec: `docs/specs/sync.allium`'s `BoardConnection.covered_epics` and
//! `ASubEpicOfAFollowedEpicIsAskedForToo`. Following an epic follows its whole
//! sub-epic tree, at any depth — but the store's SQL cannot walk a parent
//! chain, so the board walks it: each covered epic is asked for by its direct
//! sub-epics (`sdk_connector::subtree_queries`), and each sub-epic that
//! arrives under a covered parent becomes covered in turn.
//!
//! Pure bookkeeping, so the walk is testable without a store; the connector
//! feeds it arriving rows and subscribes to whatever it reports.

use std::collections::{HashMap, HashSet};

/// The covered set of one connection's subscription.
///
/// Only grows. An unfollow, a delete or a move out of the tree leaves the epic
/// covered until the next connection starts a fresh one — asking for rows that
/// are gone sends nothing (see the rule's "ONLY WIDENS" clause).
#[derive(Default)]
pub struct SubtreeCover {
    covered: HashSet<i64>,
}

impl SubtreeCover {
    /// Start from the followed epics, which the initial subscription already
    /// asks for in full.
    pub fn new(followed: impl IntoIterator<Item = i64>) -> Self {
        Self {
            covered: followed.into_iter().collect(),
        }
    }

    /// Whether `epic` is covered. The cheap check a caller runs before
    /// gathering `known` for [`Self::delivered`], which no arrival under an
    /// uncovered parent needs.
    pub fn covers(&self, epic: i64) -> bool {
        self.covered.contains(&epic)
    }

    /// An epic row arrived, or its parent changed. Returns the epics that
    /// became covered because of it, each of which needs its subtree asked
    /// for.
    ///
    /// `known` is every `(id, parent)` pair the board currently holds, this
    /// one included; `0` is the no-parent sentinel. It matters because a
    /// descendant may already be here — delivered through own_creations
    /// before its parent was covered — and the store will not deliver it a
    /// second time, so it is covered now, with its parent, or never.
    pub fn delivered(&mut self, id: i64, parent: i64, known: &[(i64, i64)]) -> Vec<i64> {
        if self.covers(id) || !self.covers(parent) {
            return Vec::new();
        }
        let mut children: HashMap<i64, Vec<i64>> = HashMap::new();
        for &(child, p) in known {
            children.entry(p).or_default().push(child);
        }
        let mut newly = Vec::new();
        let mut frontier = vec![id];
        while let Some(epic) = frontier.pop() {
            if !self.covered.insert(epic) {
                continue;
            }
            newly.push(epic);
            frontier.extend(children.get(&epic).into_iter().flatten());
        }
        newly
    }
}
