//! Coordinates (access model phase 3): the server's view of where units and domains sit, and the checks a manifest and
//! a workspace go through. The rules themselves are in `scaffoldry_core::coordinates`.

use crate::service::ServiceError;
use crate::state::{OrganizationNode, SharedState, WorkspaceRecord};
use scaffoldry_core::coordinates::{narrow, sensitivity_rank, workspace_effective, Coordinates, Effective, Places};
use scaffoldry_core::tree::OrgTree;
use scaffoldry_engine::AppManifest;
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

struct ServerPlaces<'a> {
    tree: &'a OrgTree,
    orgs: &'a HashMap<Uuid, OrganizationNode>,
    domains: &'a [(String, Option<String>)],
}

impl Places for ServerPlaces<'_> {
    fn unit_within(&self, hierarchy: &str, unit: &str, container: &str) -> bool {
        let (Ok(u), Ok(c)) = (Uuid::parse_str(unit), Uuid::parse_str(container)) else { return false };
        self.tree.hierarchy_of(u) == Some(hierarchy) && self.tree.contains(c, u)
    }

    fn domain_within(&self, domain: &str, container: &str) -> bool {
        let mut current = self.domains.iter().find(|(k, _)| k == domain).map(|(k, _)| k.as_str());
        while let Some(d) = current {
            if d == container {
                return true;
            }
            current = self.domains.iter().find(|(k, _)| k == d).and_then(|(_, p)| p.as_deref());
        }
        false
    }

    fn hierarchies(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.orgs.values().map(|o| o.hierarchy_key.clone()).collect();
        keys.sort();
        keys.dedup();
        keys
    }

    fn root_unit(&self, hierarchy: &str) -> Option<String> {
        self.orgs.values().filter(|o| o.hierarchy_key == hierarchy && o.parent_id.is_none()).map(|o| o.id).min().map(|id| id.to_string())
    }
}

/// The domain tree, read once.
pub fn domains(state: &SharedState) -> Result<&Vec<(String, Option<String>)>, ServiceError> {
    if state.domains.get().is_none() {
        let loaded = state.repository.list_domains().map_err(|e| ServiceError::internal(e.to_string()))?;
        let _ = state.domains.set(loaded);
    }
    state.domains.get().ok_or_else(|| ServiceError::internal("The domain tree is not loaded"))
}

/// Runs `f` with the places this instance holds.
pub fn with_places<R>(state: &SharedState, f: impl FnOnce(&dyn Places) -> R) -> Result<R, ServiceError> {
    let domains = domains(state)?;
    let tree = state.tree.read().unwrap_or_else(|p| p.into_inner());
    let orgs = state.organizations.read().unwrap_or_else(|p| p.into_inner());
    Ok(f(&ServerPlaces { tree: &tree, orgs: &orgs, domains }))
}

/// Runs `f` with the places this instance holds and the unit tree they come from.
pub fn with_world<R>(state: &SharedState, f: impl FnOnce(&dyn Places, &OrgTree) -> R) -> Result<R, ServiceError> {
    let domains = domains(state)?;
    let tree = state.tree.read().unwrap_or_else(|p| p.into_inner());
    let orgs = state.organizations.read().unwrap_or_else(|p| p.into_inner());
    Ok(f(&ServerPlaces { tree: &tree, orgs: &orgs, domains }, &tree))
}

/// Where a workspace, or one app in it, sits: a unit in each hierarchy and a domain. A workspace's own unit is its place
/// in that unit's hierarchy.
pub fn object_place(places: &dyn Places, tree: &OrgTree, ws: &WorkspaceRecord, app: Option<&AppManifest>) -> (BTreeMap<String, Uuid>, String) {
    let ws_coords = (!ws.coordinates.is_empty()).then_some(&ws.coordinates);
    let effective = match app {
        None => workspace_effective(ws_coords, places).ok(),
        Some(m) => walk(ws_coords, m, places).0.map(|(a, _)| a),
    };
    let Some(e) = effective else { return (BTreeMap::new(), scaffoldry_core::coordinates::ROOT_DOMAIN.to_string()) };
    let mut units: BTreeMap<String, Uuid> = e.units.iter().filter_map(|(h, u)| Uuid::parse_str(u).ok().map(|u| (h.clone(), u))).collect();
    if let Some(own) = ws.organization_id {
        if let Some(h) = tree.hierarchy_of(own) {
            units.insert(h.to_string(), own);
        }
    }
    (units, e.domain)
}

/// Where one table sits: a unit in each hierarchy and a domain, after the workspace, the app and the table have each
/// narrowed what they were given. A table that names nothing sits where its app sits.
pub fn table_place(places: &dyn Places, tree: &OrgTree, ws: &WorkspaceRecord, app: &AppManifest, table_id: &str) -> (BTreeMap<String, Uuid>, String) {
    let ws_coords = (!ws.coordinates.is_empty()).then_some(&ws.coordinates);
    let Some((app_effective, tables)) = walk(ws_coords, app, places).0 else { return (BTreeMap::new(), scaffoldry_core::coordinates::ROOT_DOMAIN.to_string()) };
    let e = tables.get(table_id).cloned().unwrap_or(app_effective);
    let mut units: BTreeMap<String, Uuid> = e.units.iter().filter_map(|(h, u)| Uuid::parse_str(u).ok().map(|u| (h.clone(), u))).collect();
    if let Some(own) = ws.organization_id {
        if let Some(h) = tree.hierarchy_of(own) {
            units.entry(h.to_string()).or_insert(own);
        }
    }
    (units, e.domain)
}

/// Names every coordinate that points at a hierarchy, unit, or domain that is not there.
pub fn missing(own: &Coordinates, places: &dyn Places) -> Vec<String> {
    let mut out = Vec::new();
    let hierarchies = places.hierarchies();
    for (h, u) in &own.units {
        if !hierarchies.contains(h) {
            out.push(format!("hierarchy '{h}' does not exist"));
        } else if !places.unit_within(h, u, u) {
            out.push(format!("unit '{u}' is not a unit of hierarchy '{h}'"));
        }
    }
    if let Some(d) = &own.domain {
        if !places.domain_within(d, d) {
            out.push(format!("domain '{d}' does not exist"));
        }
    }
    out
}

/// A workspace's effective coordinates with the hierarchies it names, or the reasons it is refused.
pub fn effective_of_workspace(own: Option<&Coordinates>, places: &dyn Places) -> Result<Effective, Vec<String>> {
    let mut problems = own.map(|c| missing(c, places)).unwrap_or_default();
    match workspace_effective(own, places) {
        Ok(e) if problems.is_empty() => Ok(e),
        Ok(_) => Err(problems),
        Err(w) => {
            problems.extend(w.iter().map(|x| format!("workspace {x}")));
            Err(problems)
        }
    }
}

/// The effective coordinates of an app, and of each of its tables by id.
type Walked = (Effective, BTreeMap<String, Effective>);

/// The effective coordinates of an app and of each of its tables. A level that would widen takes its container's, so the
/// walk goes on and every problem is found. The problems are returned beside the result.
fn walk(ws: Option<&Coordinates>, m: &AppManifest, places: &dyn Places) -> (Option<Walked>, Vec<String>) {
    let mut problems = Vec::new();
    let base = match workspace_effective(ws, places) {
        Ok(e) => e,
        Err(w) => return (None, w.iter().map(|x| format!("workspace {x}")).collect()),
    };
    let mut step = |container: &Effective, own: Option<&Coordinates>, level: &'static str, path: &str| {
        if let Some(c) = own {
            problems.extend(missing(c, places).into_iter().map(|p| format!("{path}{p}")));
        }
        match narrow(container, own, level, places) {
            Ok(e) => e,
            Err(w) => {
                problems.extend(w.iter().map(|x| format!("{path}{x}")));
                container.clone()
            }
        }
    };
    let app = step(&base, m.coordinates.as_ref(), "app", "app ");
    let mut tables = BTreeMap::new();
    for t in &m.tables {
        let here = step(&app, t.coordinates.as_ref(), "table", &format!("table '{}' ", t.id));
        for f in &t.fields {
            if let Some(s) = &f.sensitivity {
                let own = Coordinates { sensitivity: Some(s.clone()), ..Coordinates::default() };
                step(&here, Some(&own), "field", &format!("table '{}' field '{}' ", t.id, f.name));
            }
        }
        tables.insert(t.id.clone(), here);
    }
    (Some((app, tables)), problems)
}

/// Every way the manifest's coordinates widen or point nowhere. Empty when they are sound.
pub fn manifest_problems(state: &SharedState, ws: &WorkspaceRecord, m: &AppManifest) -> Result<Vec<String>, ServiceError> {
    let ws_coords = (!ws.coordinates.is_empty()).then_some(&ws.coordinates);
    with_places(state, |places| {
        let mut problems = walk(ws_coords, m, places).1;
        let hierarchies = places.hierarchies();
        for t in &m.tables {
            for f in &t.fields {
                if let Some(h) = &f.hierarchy {
                    if !hierarchies.contains(h) {
                        problems.push(format!("table '{}' field '{}': hierarchy '{h}' does not exist", t.id, f.name));
                    }
                }
            }
            for h in t.row_place.keys() {
                if !hierarchies.contains(h) {
                    problems.push(format!("table '{}': row place names hierarchy '{h}', which does not exist", t.id));
                }
            }
        }
        problems
    })
}

/// Where `new` is looser than `old`: a lower sensitivity, a wider domain, or a wider unit.
pub fn looser(old: &Effective, new: &Effective, places: &dyn Places) -> Vec<String> {
    let mut out = Vec::new();
    if sensitivity_rank(&new.sensitivity) < sensitivity_rank(&old.sensitivity) {
        out.push(format!("sensitivity went from {} to {}", old.sensitivity, new.sensitivity));
    }
    if !places.domain_within(&new.domain, &old.domain) {
        out.push(format!("domain went from {} to {}", old.domain, new.domain));
    }
    for (h, was) in &old.units {
        if let Some(now) = new.units.get(h) {
            if !places.unit_within(h, now, was) {
                out.push(format!("unit:{h} went from {was} to {now}"));
            }
        }
    }
    out
}

/// Where a change to a live app loosens a coordinate of the app, one of its tables, or one of its fields.
pub fn widened(state: &SharedState, ws: &WorkspaceRecord, old: &AppManifest, new: &AppManifest) -> Result<Vec<String>, ServiceError> {
    let ws_coords = (!ws.coordinates.is_empty()).then_some(&ws.coordinates);
    with_places(state, |places| {
        let (Some((old_app, old_tables)), _) = walk(ws_coords, old, places) else { return Vec::new() };
        let (Some((new_app, new_tables)), _) = walk(ws_coords, new, places) else { return Vec::new() };
        let mut out: Vec<String> = looser(&old_app, &new_app, places).into_iter().map(|p| format!("app {p}")).collect();
        for t in &new.tables {
            let (Some(was), Some(now)) = (old_tables.get(&t.id), new_tables.get(&t.id)) else { continue };
            out.extend(looser(was, now, places).into_iter().map(|p| format!("table '{}' {p}", t.id)));
            let Some(old_table) = old.tables.iter().find(|o| o.id == t.id) else { continue };
            for f in &t.fields {
                let Some(old_field) = old_table.fields.iter().find(|o| o.name == f.name) else { continue };
                let rank = |table: &Effective, own: &Option<String>| own.as_deref().and_then(sensitivity_rank).max(sensitivity_rank(&table.sensitivity));
                if rank(now, &f.sensitivity) < rank(was, &old_field.sensitivity) {
                    out.push(format!("table '{}' field '{}' sensitivity was lowered", t.id, f.name));
                }
            }
        }
        out
    })
}

/// A domain and every domain above it.
fn domain_path(domains: &[(String, Option<String>)], domain: &str) -> std::collections::BTreeSet<String> {
    let mut path = std::collections::BTreeSet::new();
    let mut current = Some(domain.to_string());
    while let Some(d) = current {
        if !path.insert(d.clone()) {
            break;
        }
        current = domains.iter().find(|(k, _)| *k == d).and_then(|(_, p)| p.clone());
    }
    path
}

/// What Cedar is told about a place: the domain path and the sensitivity rank. For an app, the sensitivity is the highest
/// of the app's and its tables'.
pub fn cedar_place(state: &SharedState, ws: &WorkspaceRecord, app: Option<&AppManifest>) -> (std::collections::BTreeSet<String>, i64) {
    let ws_coords = (!ws.coordinates.is_empty()).then_some(&ws.coordinates);
    let Ok(domains) = domains(state) else { return Default::default() };
    let (domain, rank) = with_places(state, |places| match app {
        None => workspace_effective(ws_coords, places).map(|e| (e.domain, sensitivity_rank(&e.sensitivity))).ok(),
        Some(m) => walk(ws_coords, m, places).0.map(|(a, tables)| {
            let top = tables.values().map(|t| sensitivity_rank(&t.sensitivity)).chain([sensitivity_rank(&a.sensitivity)]).max().flatten();
            (a.domain, top)
        }),
    })
    .ok()
    .flatten()
    .unwrap_or_default();
    (domain_path(domains, &domain), rank.map_or(0, i64::from))
}
