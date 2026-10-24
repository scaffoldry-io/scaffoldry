//! Reference reach evaluator (access model phase 5).
//!
//! A reference evaluator that walks `parent_id` upward from an object's unit
//! and checks each grant directly, with no closure and no index.
//! It is slow and obviously correct. It is used only by tests.

use crate::coordinates::Places;
use crate::reach::{AnchorKind, Grant, HolderKind, HolderSet, Holding, Object, Role};
use std::collections::BTreeSet;
use uuid::Uuid;

/// One unit in the reference hierarchy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceUnit {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub hierarchy: String,
}

/// One domain in the reference domain tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceDomain {
    pub key: String,
    pub parent_key: Option<String>,
}

/// A pure implementation of `Places` using linear searches over reference units and domains.
#[derive(Debug, Clone)]
pub struct ReferencePlaces<'a> {
    pub units: &'a [ReferenceUnit],
    pub domains: &'a [ReferenceDomain],
    pub hierarchies: Vec<String>,
}

impl<'a> Places for ReferencePlaces<'a> {
    fn unit_within(&self, hierarchy: &str, unit: &str, container: &str) -> bool {
        let Ok(u) = Uuid::parse_str(unit) else { return false };
        let Ok(c) = Uuid::parse_str(container) else { return false };
        reference_unit_within_depth(self.units, u, c, None, Some(hierarchy))
    }

    fn domain_within(&self, domain: &str, container: &str) -> bool {
        reference_domain_within(self.domains, domain, container)
    }

    fn hierarchies(&self) -> Vec<String> {
        self.hierarchies.clone()
    }

    fn root_unit(&self, hierarchy: &str) -> Option<String> {
        self.units
            .iter()
            .find(|u| u.hierarchy == hierarchy && u.parent_id.is_none())
            .map(|u| u.id.to_string())
    }
}

/// Builds the holder set of a person by walking `parent_id` upward for each holding.
pub fn reference_holder_set(
    person: &str,
    holdings: &[Holding],
    units: &[ReferenceUnit],
    groups: impl IntoIterator<Item = String>,
    now: i64,
) -> HolderSet {
    let mut set = HolderSet {
        person: person.to_lowercase(),
        positions: Vec::new(),
        units: BTreeSet::new(),
        groups: groups.into_iter().collect(),
    };

    for h in holdings.iter().filter(|h| {
        h.effective.is_none_or(|e| e <= now) && h.end.is_none_or(|e| e >= now)
    }) {
        let mut curr = Some(h.unit);
        while let Some(u) = curr {
            set.units.insert(u);
            curr = units.iter().find(|n| n.id == u).and_then(|n| n.parent_id);
        }
        if let Some(key) = &h.position_key {
            if !set.positions.contains(&(key.clone(), h.unit)) {
                set.positions.push((key.clone(), h.unit));
            }
        }
    }
    set
}

/// Walks `parent_id` upward from `unit` to check if `anchor` is reached within `depth`.
pub fn reference_unit_within_depth(
    units: &[ReferenceUnit],
    unit: Uuid,
    anchor: Uuid,
    depth: Option<i16>,
    expected_hierarchy: Option<&str>,
) -> bool {
    let mut curr = Some(unit);
    let mut d: i16 = 0;
    while let Some(u) = curr {
        let node = units.iter().find(|n| n.id == u);
        if let Some(h) = expected_hierarchy {
            if let Some(n) = node {
                if n.hierarchy != h {
                    return false;
                }
            } else {
                return false;
            }
        }
        if u == anchor {
            return depth.is_none_or(|max| d <= max);
        }
        if depth.is_some_and(|max| d >= max) {
            return false;
        }
        curr = node.and_then(|n| n.parent_id);
        d += 1;
    }
    false
}

/// Walks `parent_key` upward from `domain` to check if `target_ancestor` is reached.
pub fn reference_domain_within(
    domains: &[ReferenceDomain],
    domain: &str,
    target_ancestor: &str,
) -> bool {
    let mut curr = Some(domain);
    while let Some(d) = curr {
        if d == target_ancestor {
            return true;
        }
        curr = domains
            .iter()
            .find(|dom| dom.key == d)
            .and_then(|dom| dom.parent_key.as_deref());
    }
    false
}

fn position_of(holder_id: &str) -> (&str, Option<Uuid>) {
    match holder_id.split_once('@') {
        Some((key, "*")) => (key, None),
        Some((key, unit)) => (key, Uuid::parse_str(unit).ok()),
        None => (holder_id, None),
    }
}

/// Evaluates whether a grant reaches an object using pure parent walking without closures or indexes.
pub fn reference_reaches(
    g: &Grant,
    holders: &HolderSet,
    object: &Object,
    units: &[ReferenceUnit],
    domains: &[ReferenceDomain],
    now: i64,
) -> bool {
    if g.revoked || g.valid_from > now || g.valid_until.is_some_and(|u| u <= now) || !holders.is_holder(g) {
        return false;
    }
    if let Some(d) = &g.domain {
        if !reference_domain_within(domains, &object.domain, d) {
            return false;
        }
    }

    // A sealed container stops a grant anchored above it. For a container grant that means a sealed container nested inside
    // its anchor. A unit grant is above every container, so any sealed container stops it.
    let sealed_above = |anchor_index: Option<usize>| -> bool {
        object.containers.iter().enumerate().any(|(i, c)| object.sealed.contains(c) && anchor_index.is_none_or(|a| i > a))
    };
    let placed = g.hierarchy.as_deref().and_then(|h| object.units.get(h)).copied();
    match g.anchor_kind {
        AnchorKind::Container => match g.anchor_id.as_ref().and_then(|a| object.containers.iter().position(|c| c == a)) {
            Some(at) => !sealed_above(Some(at)),
            None => false,
        },
        AnchorKind::Unit => match (placed, g.anchor_id.as_deref().and_then(|a| Uuid::parse_str(a).ok())) {
            (Some(unit), Some(anchor)) => reference_unit_within_depth(units, unit, anchor, g.depth, g.hierarchy.as_deref()) && !sealed_above(None),
            _ => false,
        },
        AnchorKind::HolderUnit => {
            let (key, named) = position_of(&g.holder_id);
            let Some(unit) = placed else { return false };
            g.holder_kind == HolderKind::Position
                && !sealed_above(None)
                && holders.positions.iter().filter(|(k, u)| k == key && named.is_none_or(|n| n == *u)).any(|(_, anchor)| {
                    reference_unit_within_depth(units, unit, *anchor, g.depth, g.hierarchy.as_deref())
                })
        }
    }
}

/// Evaluates the highest role among grants that reach an object using the reference evaluator.
pub fn reference_role_at(
    holders: &HolderSet,
    grants: &[Grant],
    object: &Object,
    units: &[ReferenceUnit],
    domains: &[ReferenceDomain],
    now: i64,
) -> Option<(Role, Vec<i64>)> {
    let reaching: Vec<&Grant> = grants
        .iter()
        .filter(|g| reference_reaches(g, holders, object, units, domains, now))
        .collect();
    let top = reaching.iter().map(|g| g.role).max()?;
    Some((
        top,
        reaching.iter().filter(|g| g.role == top).map(|g| g.id).collect(),
    ))
}
