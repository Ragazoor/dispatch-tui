//! The compact epic tree a flattened board section and the agent-tree pane's
//! agents section lay their epic groups out as (`board-layout.allium`:
//! `EpicGroupTree`; `agent-tree.allium`: `AgentListGroupsByEpic`; task
//! #99033).
//!
//! A header is drawn for an epic that owns a group, and for an epic where two
//! or more branches meet. Every other epic is skipped: it draws no header and
//! adds no level. Under one parent, the drawn children with no drawn child of
//! their own come first, then the rest, each run in the caller's sibling
//! order.

use std::collections::{HashMap, HashSet};

use crate::models::EpicId;

/// The compact layout of a set of groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompactTree {
    /// The groups in the order they are drawn, as indices into the input.
    pub(crate) order: Vec<usize>,
    /// Each input group's compact path: the drawn epics from the top down to
    /// its own, in input order. Its depth is `len() - 1`.
    pub(crate) paths: Vec<Vec<EpicId>>,
}

impl CompactTree {
    /// Lay out `groups`, each given as its full path in view: the counted
    /// ancestors from the top down, ending with the group's own epic. No two
    /// groups share an own epic. `key` orders siblings within each run.
    pub(crate) fn build<K: Ord>(groups: &[Vec<EpicId>], key: impl Fn(EpicId) -> K) -> Self {
        let owners: HashSet<EpicId> = groups.iter().filter_map(|p| p.last().copied()).collect();
        // The distinct children of each epic on some group's path.
        let mut below: HashMap<EpicId, HashSet<EpicId>> = HashMap::new();
        for path in groups {
            for pair in path.windows(2) {
                below.entry(pair[0]).or_default().insert(pair[1]);
            }
        }
        let shown =
            |epic: &EpicId| owners.contains(epic) || below.get(epic).is_some_and(|c| c.len() >= 2);
        let paths: Vec<Vec<EpicId>> = groups
            .iter()
            .map(|path| path.iter().copied().filter(|e| shown(e)).collect())
            .collect();

        // The compact tree's edges, from every compact path's prefixes.
        let mut children: HashMap<Option<EpicId>, Vec<EpicId>> = HashMap::new();
        let mut seen: HashSet<EpicId> = HashSet::new();
        for path in &paths {
            let mut parent = None;
            for &epic in path {
                if seen.insert(epic) {
                    children.entry(parent).or_default().push(epic);
                }
                parent = Some(epic);
            }
        }
        let parents: HashSet<EpicId> = children.keys().filter_map(|k| *k).collect();
        for list in children.values_mut() {
            list.sort_by_cached_key(|e| (parents.contains(e), key(*e)));
        }

        let group_of: HashMap<EpicId, usize> = groups
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.last().map(|&e| (e, i)))
            .collect();
        let mut order = Vec::with_capacity(groups.len());
        // Pre-order: an epic's own group, then its children's subtrees.
        let mut stack: Vec<EpicId> = children
            .get(&None)
            .map(|roots| roots.iter().rev().copied().collect())
            .unwrap_or_default();
        while let Some(epic) = stack.pop() {
            if let Some(&i) = group_of.get(&epic) {
                order.push(i);
            }
            if let Some(kids) = children.get(&Some(epic)) {
                stack.extend(kids.iter().rev().copied());
            }
        }
        Self { order, paths }
    }

    /// The drawn epics nearest below `epic` on the groups' paths: what takes
    /// its place when it is skipped (`tasks.allium`:
    /// `FoldMovesToDrawnDescendant`). Empty when `epic` is drawn itself or
    /// no group lies below it.
    pub(crate) fn takers(&self, groups: &[Vec<EpicId>], epic: EpicId) -> Vec<EpicId> {
        if self.draws(epic) {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (full, compact) in groups.iter().zip(&self.paths) {
            let Some(at) = full.iter().position(|&e| e == epic) else {
                continue;
            };
            let below: HashSet<EpicId> = full[at + 1..].iter().copied().collect();
            if let Some(&first) = compact.iter().find(|e| below.contains(e)) {
                if !out.contains(&first) {
                    out.push(first);
                }
            }
        }
        out
    }

    /// Whether `epic` draws a header here.
    pub(crate) fn draws(&self, epic: EpicId) -> bool {
        self.paths.iter().any(|p| p.contains(&epic))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(path: &[i64]) -> Vec<EpicId> {
        path.iter().copied().map(EpicId).collect()
    }

    /// A › B › X › C, A › B › D, A › E (task #99033's example).
    fn example() -> Vec<Vec<EpicId>> {
        vec![ids(&[1, 2, 3, 4]), ids(&[1, 2, 5]), ids(&[1, 6])]
    }

    #[test]
    fn skips_single_branch_epics_and_puts_leaves_first() {
        let tree = CompactTree::build(&example(), |e| e.0);
        assert_eq!(
            tree.paths,
            vec![ids(&[1, 2, 4]), ids(&[1, 2, 5]), ids(&[1, 6])]
        );
        assert_eq!(tree.order, vec![2, 0, 1]);
    }

    #[test]
    fn a_lone_group_is_flush() {
        let tree = CompactTree::build(&[ids(&[1, 2, 3])], |e| e.0);
        assert_eq!(tree.paths, vec![ids(&[3])]);
    }

    #[test]
    fn an_owner_with_one_child_still_draws() {
        let tree = CompactTree::build(&[ids(&[1]), ids(&[1, 2])], |e| e.0);
        assert_eq!(tree.paths, vec![ids(&[1]), ids(&[1, 2])]);
        assert_eq!(tree.order, vec![0, 1]);
    }

    #[test]
    fn a_skipped_epics_takers_are_the_nearest_drawn_below() {
        let groups = vec![ids(&[1, 2, 3, 4]), ids(&[1, 2, 5])];
        let tree = CompactTree::build(&groups, |e| e.0);
        assert_eq!(tree.takers(&groups, EpicId(1)), ids(&[2]));
        assert_eq!(tree.takers(&groups, EpicId(3)), ids(&[4]));
        assert!(tree.takers(&groups, EpicId(2)).is_empty(), "2 draws");
        assert!(
            tree.takers(&groups, EpicId(9)).is_empty(),
            "nothing below 9"
        );
    }
}
