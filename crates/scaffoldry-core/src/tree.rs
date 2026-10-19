//! Organization hierarchy tree and transitive closure engine.
//! Standard search: NIST SP 800-162, SP 800-178, NGAC, INCITS 359, RFC 7643.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// A row in the `org_closure` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgClosureRow {
    pub hierarchy_key: String,
    pub ancestor: Uuid,
    pub descendant: Uuid,
    pub depth: i16,
}

/// In-memory transitive closure index for organization hierarchies.
/// Answers ancestor, descendant, and containment queries in O(1) without recursion or step caps.
#[derive(Debug, Clone, Default)]
pub struct OrgTree {
    /// ancestor -> list of (descendant, depth)
    descendants_by_ancestor: HashMap<Uuid, Vec<(Uuid, i16)>>,
    /// descendant -> list of (ancestor, depth), sorted by depth ascending
    ancestors_by_descendant: HashMap<Uuid, Vec<(Uuid, i16)>>,
    /// (ancestor, descendant) pairs for O(1) containment checks
    pairs: HashSet<(Uuid, Uuid)>,
    /// unit -> hierarchy_key
    hierarchies: HashMap<Uuid, String>,
}

impl OrgTree {
    /// Builds a tree index from closure rows.
    pub fn new(rows: impl IntoIterator<Item = OrgClosureRow>) -> Self {
        let mut descendants_by_ancestor: HashMap<Uuid, Vec<(Uuid, i16)>> = HashMap::new();
        let mut ancestors_by_descendant: HashMap<Uuid, Vec<(Uuid, i16)>> = HashMap::new();
        let mut pairs = HashSet::new();
        let mut hierarchies = HashMap::new();

        for row in rows {
            pairs.insert((row.ancestor, row.descendant));
            hierarchies
                .entry(row.descendant)
                .or_insert_with(|| row.hierarchy_key.clone());
            descendants_by_ancestor
                .entry(row.ancestor)
                .or_default()
                .push((row.descendant, row.depth));
            ancestors_by_descendant
                .entry(row.descendant)
                .or_default()
                .push((row.ancestor, row.depth));
        }

        for list in descendants_by_ancestor.values_mut() {
            list.sort_by_key(|(_, depth)| *depth);
        }
        for list in ancestors_by_descendant.values_mut() {
            list.sort_by_key(|(_, depth)| *depth);
        }

        Self {
            descendants_by_ancestor,
            ancestors_by_descendant,
            pairs,
            hierarchies,
        }
    }

    /// Computes full closure from a `parent_id` mapping. Useful for offline generation,
    /// migrations, and property-based test comparisons.
    pub fn from_parent_map(hierarchy_key: &str, parent_map: &HashMap<Uuid, Option<Uuid>>) -> Self {
        let mut rows = Vec::new();

        // 1. Depth 0 self-rows
        for &id in parent_map.keys() {
            rows.push(OrgClosureRow {
                hierarchy_key: hierarchy_key.to_string(),
                ancestor: id,
                descendant: id,
                depth: 0,
            });
        }

        // 2. Direct and transitive ancestors by walking parent_id
        for (&descendant, &parent_opt) in parent_map {
            let mut current = parent_opt;
            let mut depth = 1i16;
            let mut visited = HashSet::new();
            visited.insert(descendant);

            while let Some(ancestor) = current {
                if !visited.insert(ancestor) {
                    // Cycle in bad input; terminate walk
                    break;
                }
                rows.push(OrgClosureRow {
                    hierarchy_key: hierarchy_key.to_string(),
                    ancestor,
                    descendant,
                    depth,
                });
                depth += 1;
                current = parent_map.get(&ancestor).copied().flatten();
            }
        }

        Self::new(rows)
    }

    /// Returns every ancestor of `unit`, ordered by depth ascending:
    /// depth 0 is `unit` itself, depth 1 is its parent, depth 2 is its grandparent, etc.
    pub fn ancestors(&self, unit: Uuid) -> Vec<Uuid> {
        self.ancestors_by_descendant
            .get(&unit)
            .map(|list| list.iter().map(|(id, _)| *id).collect())
            .unwrap_or_else(|| vec![unit])
    }

    /// Returns every descendant of `unit`, optionally filtered to a maximum depth.
    /// Ordered by depth ascending. Depth 0 is `unit` itself.
    pub fn descendants(&self, unit: Uuid, depth: Option<i16>) -> Vec<Uuid> {
        match self.descendants_by_ancestor.get(&unit) {
            Some(list) => list
                .iter()
                .filter(|(_, d)| depth.is_none_or(|max_d| *d <= max_d))
                .map(|(id, _)| *id)
                .collect(),
            None => vec![unit],
        }
    }

    /// Returns `true` if `ancestor` is an ancestor of `unit` (including the unit itself at depth 0).
    pub fn contains(&self, ancestor: Uuid, unit: Uuid) -> bool {
        if ancestor == unit {
            return true;
        }
        self.pairs.contains(&(ancestor, unit))
    }

    /// Returns the immediate parent of `unit` (depth 1 ancestor), if any.
    pub fn parent_of(&self, unit: Uuid) -> Option<Uuid> {
        self.ancestors_by_descendant
            .get(&unit)
            .and_then(|list| list.iter().find(|(_, d)| *d == 1).map(|(id, _)| *id))
    }

    /// Returns the hierarchy key for `unit`.
    pub fn hierarchy_of(&self, unit: Uuid) -> Option<&str> {
        self.hierarchies.get(&unit).map(|s| s.as_str())
    }

    /// Returns the total number of distinct units in this tree index.
    pub fn len(&self) -> usize {
        self.hierarchies.len()
    }

    /// Returns `true` if the tree has no units.
    pub fn is_empty(&self) -> bool {
        self.hierarchies.is_empty()
    }
}
