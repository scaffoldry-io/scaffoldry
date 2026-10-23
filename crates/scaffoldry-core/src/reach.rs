//! Reach: who may do what where (access model phase 4).
//!
//! A grant says a holder has a role at an anchor. An object says where it sits. `role_at` is the one function that
//! turns the two into a role: the highest among the grants that reach the object. Everything here is pure. The tree and
//! the domains come in as arguments, and time is unix seconds.

use crate::coordinates::Places;
use crate::tree::OrgTree;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Roles in rank order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Viewer,
    Commenter,
    Editor,
    Admin,
    Owner,
}

impl Role {
    pub fn parse(s: &str) -> Option<Role> {
        match s.to_lowercase().as_str() {
            "viewer" => Some(Role::Viewer),
            "commenter" => Some(Role::Commenter),
            "editor" => Some(Role::Editor),
            "admin" => Some(Role::Admin),
            "owner" => Some(Role::Owner),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Commenter => "commenter",
            Role::Editor => "editor",
            Role::Admin => "admin",
            Role::Owner => "owner",
        }
    }
}

/// Who holds a grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HolderKind {
    Person,
    Position,
    Unit,
    Group,
}

impl HolderKind {
    pub const ALL: [&'static str; 4] = ["person", "position", "unit", "group"];

    pub fn parse(s: &str) -> Option<HolderKind> {
        match s {
            "person" => Some(HolderKind::Person),
            "position" => Some(HolderKind::Position),
            "unit" => Some(HolderKind::Unit),
            "group" => Some(HolderKind::Group),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            HolderKind::Person => "person",
            HolderKind::Position => "position",
            HolderKind::Unit => "unit",
            HolderKind::Group => "group",
        }
    }
}

/// Where a grant is anchored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorKind {
    /// A workspace, app or table, and everything inside it.
    Container,
    /// A unit, and the objects placed in its subtree, to a depth.
    Unit,
    /// Wherever the holder holds the position the grant names.
    HolderUnit,
}

impl AnchorKind {
    pub const ALL: [&'static str; 3] = ["container", "unit", "holder_unit"];

    pub fn parse(s: &str) -> Option<AnchorKind> {
        match s {
            "container" => Some(AnchorKind::Container),
            "unit" => Some(AnchorKind::Unit),
            "holder_unit" => Some(AnchorKind::HolderUnit),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AnchorKind::Container => "container",
            AnchorKind::Unit => "unit",
            AnchorKind::HolderUnit => "holder_unit",
        }
    }
}

/// One grant, as far as reach is concerned. Clearance and source do not change what a grant reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub id: i64,
    pub holder_kind: HolderKind,
    /// An address, `key@<unit id>` or `key@*`, a unit id, or a group id.
    pub holder_id: String,
    pub role: Role,
    pub hierarchy: Option<String>,
    pub anchor_kind: AnchorKind,
    /// A container id such as `workspace:<id>` or `app:<slug>`, or a unit id.
    pub anchor_id: Option<String>,
    /// 0 is the anchor only. `None` is the whole subtree.
    pub depth: Option<i16>,
    pub domain: Option<String>,
    pub valid_from: i64,
    pub valid_until: Option<i64>,
    pub revoked: bool,
}

/// One role a person holds at a unit: an appointment, an affiliation, or a position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holding {
    pub unit: Uuid,
    pub position_key: Option<String>,
    pub effective: Option<i64>,
    /// The last moment the holding counts.
    pub end: Option<i64>,
}

/// What a grant can be held by, for one person right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HolderSet {
    /// The person's address, lowercase.
    pub person: String,
    /// `(position key, unit)` for each position held.
    pub positions: Vec<(String, Uuid)>,
    /// Each unit where the person holds any role, and every ancestor of it.
    pub units: BTreeSet<Uuid>,
    pub groups: BTreeSet<String>,
}

/// Where an object sits: the containers it is inside, a unit in each hierarchy, and a domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub containers: Vec<String>,
    pub units: BTreeMap<String, Uuid>,
    pub domain: String,
}

/// The holder set of one person. A holding outside its dates is not counted.
pub fn holder_set(person: &str, holdings: &[Holding], tree: &OrgTree, groups: impl IntoIterator<Item = String>, now: i64) -> HolderSet {
    let mut set = HolderSet { person: person.to_lowercase(), positions: Vec::new(), units: BTreeSet::new(), groups: groups.into_iter().collect() };
    for h in holdings.iter().filter(|h| h.effective.is_none_or(|e| e <= now) && h.end.is_none_or(|e| e >= now)) {
        set.units.extend(tree.ancestors(h.unit));
        if let Some(key) = &h.position_key {
            if !set.positions.contains(&(key.clone(), h.unit)) {
                set.positions.push((key.clone(), h.unit));
            }
        }
    }
    set
}

/// The position key of a position grant's holder id, and the unit it names if it names one.
fn position_of(holder_id: &str) -> (&str, Option<Uuid>) {
    match holder_id.split_once('@') {
        Some((key, "*")) => (key, None),
        Some((key, unit)) => (key, Uuid::parse_str(unit).ok()),
        None => (holder_id, None),
    }
}

impl HolderSet {
    /// Whether the grant's holder is this person.
    pub fn is_holder(&self, g: &Grant) -> bool {
        match g.holder_kind {
            HolderKind::Person => g.holder_id.eq_ignore_ascii_case(&self.person),
            HolderKind::Group => self.groups.contains(&g.holder_id),
            HolderKind::Unit => Uuid::parse_str(&g.holder_id).is_ok_and(|u| self.units.contains(&u)),
            HolderKind::Position => {
                let (key, unit) = position_of(&g.holder_id);
                self.positions.iter().any(|(k, u)| k == key && unit.is_none_or(|n| n == *u))
            }
        }
    }
}

/// Whether `unit` is the anchor or below it, within `depth` levels.
fn within_depth(tree: &OrgTree, unit: Uuid, anchor: Uuid, depth: Option<i16>) -> bool {
    tree.ancestors(unit).iter().position(|a| *a == anchor).is_some_and(|d| depth.is_none_or(|max| d as i16 <= max))
}

/// Whether the grant gives its holder a role on the object. Every axis must match within the one grant.
pub fn reaches(g: &Grant, holders: &HolderSet, object: &Object, tree: &OrgTree, places: &dyn Places, now: i64) -> bool {
    if g.revoked || g.valid_from > now || g.valid_until.is_some_and(|u| u <= now) || !holders.is_holder(g) {
        return false;
    }
    if g.domain.as_deref().is_some_and(|d| !places.domain_within(&object.domain, d)) {
        return false;
    }
    let placed = || g.hierarchy.as_deref().and_then(|h| object.units.get(h)).copied().filter(|u| g.hierarchy.as_deref().is_some_and(|h| tree.hierarchy_of(*u) == Some(h)));
    match g.anchor_kind {
        AnchorKind::Container => g.anchor_id.as_ref().is_some_and(|a| object.containers.contains(a)),
        AnchorKind::Unit => match (placed(), g.anchor_id.as_deref().and_then(|a| Uuid::parse_str(a).ok())) {
            (Some(unit), Some(anchor)) => within_depth(tree, unit, anchor, g.depth),
            _ => false,
        },
        AnchorKind::HolderUnit => {
            let (key, named) = position_of(&g.holder_id);
            let Some(unit) = placed() else { return false };
            g.holder_kind == HolderKind::Position
                && holders.positions.iter().filter(|(k, u)| k == key && named.is_none_or(|n| n == *u)).any(|(_, anchor)| within_depth(tree, unit, *anchor, g.depth))
        }
    }
}

/// The highest role among the grants that reach the object, and the ids of the grants that gave that role.
pub fn role_at(holders: &HolderSet, grants: &[Grant], object: &Object, tree: &OrgTree, places: &dyn Places, now: i64) -> Option<(Role, Vec<i64>)> {
    let reaching: Vec<&Grant> = grants.iter().filter(|g| reaches(g, holders, object, tree, places, now)).collect();
    let top = reaching.iter().map(|g| g.role).max()?;
    Some((top, reaching.iter().filter(|g| g.role == top).map(|g| g.id).collect()))
}

/// What a person reaches among the rows of one table (access model phase 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowReach {
    /// Every row. A list adds no predicate.
    All,
    /// The rows placed in these units, for each hierarchy that places the table's rows. A hierarchy with no entry
    /// reaches nothing. Rows reached by any hierarchy are the union.
    Units(BTreeMap<String, BTreeSet<Uuid>>),
}

/// The units a person reaches in each hierarchy that places the rows of a table, as one set per hierarchy.
///
/// `table` is the table as an object: its containers, the unit it sits at in each hierarchy, and its domain. A row is
/// the same object with its own unit in each hierarchy in `row_hierarchies`. This returns what `reaches` would give for
/// every such row, in one pass over the grants and without looking at a row. A test keeps the two the same.
///
/// A grant in a hierarchy that does not place rows covers the table as a whole, so it gives `All` or nothing.
pub fn row_reach(holders: &HolderSet, grants: &[Grant], table: &Object, row_hierarchies: &BTreeSet<String>, tree: &OrgTree, places: &dyn Places, now: i64) -> RowReach {
    let mut sets: BTreeMap<String, BTreeSet<Uuid>> = BTreeMap::new();
    for g in grants {
        if g.revoked || g.valid_from > now || g.valid_until.is_some_and(|u| u <= now) || !holders.is_holder(g) {
            continue;
        }
        // A row has no domain of its own. The table's domain is every row's.
        if g.domain.as_deref().is_some_and(|d| !places.domain_within(&table.domain, d)) {
            continue;
        }
        if g.anchor_kind == AnchorKind::Container {
            if g.anchor_id.as_ref().is_some_and(|a| table.containers.contains(a)) {
                return RowReach::All;
            }
            continue;
        }
        let Some(h) = g.hierarchy.as_deref() else { continue };
        // The anchors of this grant: the one it names, or each unit where the holder holds the position.
        let anchors: Vec<Uuid> = match g.anchor_kind {
            AnchorKind::Unit => g.anchor_id.as_deref().and_then(|a| Uuid::parse_str(a).ok()).into_iter().collect(),
            _ if g.holder_kind != HolderKind::Position => Vec::new(),
            _ => {
                let (key, named) = position_of(&g.holder_id);
                holders.positions.iter().filter(|(k, u)| k == key && named.is_none_or(|n| n == *u)).map(|(_, u)| *u).collect()
            }
        };
        for anchor in anchors.into_iter().filter(|a| tree.hierarchy_of(*a) == Some(h)) {
            if row_hierarchies.contains(h) {
                sets.entry(h.to_string()).or_default().extend(tree.descendants(anchor, g.depth));
            } else if table.units.get(h).is_some_and(|u| tree.hierarchy_of(*u) == Some(h) && within_depth(tree, *u, anchor, g.depth)) {
                return RowReach::All;
            }
        }
    }
    RowReach::Units(sets)
}
