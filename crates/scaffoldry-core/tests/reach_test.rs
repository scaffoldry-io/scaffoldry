//! Access model phase 4: one grant shape and one function, `role_at`. These tests are the pure part: who a person is
//! as a holder, which grants reach an object, and the highest role among them.

use scaffoldry_core::coordinates::Places;
use scaffoldry_core::reach::{holder_set, reaches, role_at, AnchorKind, Grant, HolderKind, Holding, Object, Role};
use scaffoldry_core::tree::OrgTree;
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

const NOW: i64 = 1_800_000_000;
const DAY: i64 = 86_400;

/// Domains: `institution` > `research` > `grants`; `institution` > `finance`.
struct Domains;
impl Places for Domains {
    fn unit_within(&self, _: &str, _: &str, _: &str) -> bool {
        false
    }
    fn domain_within(&self, domain: &str, container: &str) -> bool {
        let parent = |d: &str| match d {
            "grants" => Some("research"),
            "research" | "finance" => Some("institution"),
            _ => None,
        };
        let mut current = Some(domain);
        while let Some(d) = current {
            if d == container {
                return true;
            }
            current = parent(d);
        }
        false
    }
    fn hierarchies(&self) -> Vec<String> {
        vec!["academic".to_string()]
    }
    fn root_unit(&self, _: &str) -> Option<String> {
        None
    }
}

/// root > science > physics > optics-lab; root > science > chemistry; root > arts.
struct World {
    tree: OrgTree,
    root: Uuid,
    science: Uuid,
    physics: Uuid,
    lab: Uuid,
    chemistry: Uuid,
    arts: Uuid,
}

fn world() -> World {
    let ids: Vec<Uuid> = (0..6).map(|_| Uuid::new_v4()).collect();
    let [root, science, physics, lab, chemistry, arts] = ids[..] else { unreachable!() };
    let parents: HashMap<Uuid, Option<Uuid>> = HashMap::from([(root, None), (science, Some(root)), (physics, Some(science)), (lab, Some(physics)), (chemistry, Some(science)), (arts, Some(root))]);
    World { tree: OrgTree::from_parent_map("academic", &parents), root, science, physics, lab, chemistry, arts }
}

fn grant(id: i64, holder: (HolderKind, &str), role: Role, anchor: (AnchorKind, Option<String>), depth: Option<i16>) -> Grant {
    Grant {
        id,
        holder_kind: holder.0,
        holder_id: holder.1.to_string(),
        role,
        hierarchy: if anchor.0 == AnchorKind::Container { None } else { Some("academic".to_string()) },
        anchor_kind: anchor.0,
        anchor_id: anchor.1,
        depth,
        domain: None,
        valid_from: NOW - DAY,
        valid_until: None,
        revoked: false,
    }
}

fn object_in(unit: Uuid, domain: &str) -> Object {
    Object { containers: vec!["workspace:ws".to_string()], units: BTreeMap::from([("academic".to_string(), unit)]), domain: domain.to_string() }
}

fn holding(unit: Uuid, position: Option<&str>) -> Holding {
    Holding { unit, position_key: position.map(str::to_string), effective: Some(NOW - 30 * DAY), end: None }
}

// 2. "Every chair@* gets viewer on their own unit's subtree." Appointing a new chair moves the access.
#[test]
fn a_holder_unit_grant_serves_every_department_and_follows_the_appointment() {
    let w = world();
    let g = grant(1, (HolderKind::Position, "chair@*"), Role::Viewer, (AnchorKind::HolderUnit, None), None);
    let physics_chair = holder_set("ada@x.edu", &[holding(w.physics, Some("chair"))], &w.tree, [], NOW);
    let physics_ws = object_in(w.physics, "institution");
    let chemistry_ws = object_in(w.chemistry, "institution");
    assert_eq!(role_at(&physics_chair, &[g.clone()], &physics_ws, &w.tree, &Domains, NOW), Some((Role::Viewer, vec![1])));
    assert_eq!(role_at(&physics_chair, &[g.clone()], &chemistry_ws, &w.tree, &Domains, NOW), None, "nothing on Chemistry");
    assert_eq!(role_at(&physics_chair, &[g.clone()], &object_in(w.lab, "institution"), &w.tree, &Domains, NOW).map(|r| r.0), Some(Role::Viewer), "the subtree below Physics is reached");

    // The same grant, a new chair in Chemistry, and no other change.
    let chemistry_chair = holder_set("bo@x.edu", &[holding(w.chemistry, Some("chair"))], &w.tree, [], NOW);
    assert_eq!(role_at(&chemistry_chair, &[g.clone()], &chemistry_ws, &w.tree, &Domains, NOW).map(|r| r.0), Some(Role::Viewer));
    assert_eq!(role_at(&chemistry_chair, &[g.clone()], &physics_ws, &w.tree, &Domains, NOW), None);
    // A dean is not a chair.
    let dean = holder_set("cy@x.edu", &[holding(w.physics, Some("dean"))], &w.tree, [], NOW);
    assert_eq!(role_at(&dean, &[g], &physics_ws, &w.tree, &Domains, NOW), None);
}

// 3. A unit grant with depth 1 reaches a department and not a lab below it.
#[test]
fn a_unit_grant_with_depth_one_reaches_the_child_and_not_the_grandchild() {
    let w = world();
    let g = grant(2, (HolderKind::Person, "ada@x.edu"), Role::Editor, (AnchorKind::Unit, Some(w.science.to_string())), Some(1));
    let ada = holder_set("ada@x.edu", &[], &w.tree, [], NOW);
    let at = |unit| role_at(&ada, &[g.clone()], &object_in(unit, "institution"), &w.tree, &Domains, NOW).map(|r| r.0);
    assert_eq!(at(w.science), Some(Role::Editor), "the anchor itself");
    assert_eq!(at(w.physics), Some(Role::Editor), "a department");
    assert_eq!(at(w.lab), None, "a lab below the department");
    assert_eq!(at(w.arts), None, "a sibling subtree");
    let all = Grant { depth: None, ..g.clone() };
    assert_eq!(role_at(&ada, &[all], &object_in(w.lab, "institution"), &w.tree, &Domains, NOW).map(|r| r.0), Some(Role::Editor), "no depth is the whole subtree");
    let only = Grant { depth: Some(0), ..g };
    assert_eq!(role_at(&ada, &[only.clone()], &object_in(w.physics, "institution"), &w.tree, &Domains, NOW), None, "depth 0 is the anchor only");
    assert!(role_at(&ada, &[only], &object_in(w.science, "institution"), &w.tree, &Domains, NOW).is_some());
}

// 4. A grant with domain research does not reach an app in finance.
#[test]
fn a_grant_with_a_domain_reaches_that_domain_and_below() {
    let w = world();
    let g = Grant { domain: Some("research".to_string()), ..grant(3, (HolderKind::Person, "ada@x.edu"), Role::Viewer, (AnchorKind::Unit, Some(w.root.to_string())), None) };
    let ada = holder_set("ada@x.edu", &[], &w.tree, [], NOW);
    let at = |domain: &str| role_at(&ada, &[g.clone()], &object_in(w.physics, domain), &w.tree, &Domains, NOW).map(|r| r.0);
    assert_eq!(at("research"), Some(Role::Viewer));
    assert_eq!(at("grants"), Some(Role::Viewer), "a domain below research");
    assert_eq!(at("finance"), None);
    assert_eq!(at("institution"), None, "the domain above does not narrow into research");
    // An object with no unit in the grant's hierarchy is not reached.
    let nowhere = Object { containers: vec![], units: BTreeMap::new(), domain: "research".to_string() };
    assert_eq!(role_at(&ada, &[g], &nowhere, &w.tree, &Domains, NOW), None);
}

// 5. An expired or revoked grant reaches nothing. A role row past its end date is not in the holder set.
#[test]
fn expired_revoked_and_ended_things_count_for_nothing() {
    let w = world();
    let base = grant(4, (HolderKind::Person, "ada@x.edu"), Role::Admin, (AnchorKind::Unit, Some(w.root.to_string())), None);
    let ada = holder_set("ada@x.edu", &[], &w.tree, [], NOW);
    let obj = object_in(w.physics, "institution");
    let reached = |g: Grant, at: i64| role_at(&ada, &[g], &obj, &w.tree, &Domains, at).is_some();
    assert!(reached(base.clone(), NOW));
    assert!(!reached(Grant { valid_until: Some(NOW - 1), ..base.clone() }, NOW), "expired");
    assert!(!reached(Grant { valid_from: NOW + 1, ..base.clone() }, NOW), "not yet valid");
    assert!(!reached(Grant { revoked: true, ..base }, NOW), "revoked");

    // A role past its end date, or not yet effective, is not held.
    let pos = Grant { holder_kind: HolderKind::Position, holder_id: "chair@*".to_string(), anchor_kind: AnchorKind::HolderUnit, anchor_id: None, ..grant(5, (HolderKind::Position, "chair@*"), Role::Viewer, (AnchorKind::HolderUnit, None), None) };
    let ended = Holding { end: Some(NOW - 1), ..holding(w.physics, Some("chair")) };
    let future = Holding { effective: Some(NOW + DAY), ..holding(w.physics, Some("chair")) };
    for h in [ended, future] {
        let set = holder_set("ada@x.edu", &[h], &w.tree, [], NOW);
        assert_eq!(role_at(&set, &[pos.clone()], &obj, &w.tree, &Domains, NOW), None);
    }
}

// 8. role_at returns the ids of the grants that gave the role.
#[test]
fn role_at_returns_the_highest_role_and_the_grants_that_gave_it() {
    let w = world();
    let ada = holder_set("ada@x.edu", &[holding(w.physics, None)], &w.tree, ["g1".to_string()], NOW);
    let obj = object_in(w.physics, "institution");
    let grants = vec![
        grant(10, (HolderKind::Person, "ADA@x.edu"), Role::Viewer, (AnchorKind::Unit, Some(w.root.to_string())), None),
        grant(11, (HolderKind::Group, "g1"), Role::Admin, (AnchorKind::Unit, Some(w.science.to_string())), None),
        grant(12, (HolderKind::Unit, &w.science.to_string()), Role::Admin, (AnchorKind::Unit, Some(w.physics.to_string())), Some(0)),
        grant(13, (HolderKind::Person, "someone-else@x.edu"), Role::Owner, (AnchorKind::Unit, Some(w.root.to_string())), None),
        grant(14, (HolderKind::Group, "g2"), Role::Owner, (AnchorKind::Unit, Some(w.root.to_string())), None),
        grant(15, (HolderKind::Person, "ada@x.edu"), Role::Owner, (AnchorKind::Unit, Some(w.arts.to_string())), None),
    ];
    let (role, ids) = role_at(&ada, &grants, &obj, &w.tree, &Domains, NOW).expect("a role");
    assert_eq!(role, Role::Admin);
    assert_eq!(ids, vec![11, 12], "the grants that reach and give the highest role, and no others");
    assert!(reaches(&grants[0], &ada, &obj, &w.tree, &Domains, NOW));
    assert!(!reaches(&grants[3], &ada, &obj, &w.tree, &Domains, NOW), "another person's grant");
    assert!(!reaches(&grants[5], &ada, &obj, &w.tree, &Domains, NOW), "a grant anchored in a sibling subtree");
}

// A container grant reaches the container and what is in it. It does not reach another container.
#[test]
fn a_container_grant_reaches_its_container_only() {
    let w = world();
    let ada = holder_set("ada@x.edu", &[], &w.tree, [], NOW);
    let g = grant(20, (HolderKind::Person, "ada@x.edu"), Role::Editor, (AnchorKind::Container, Some("app:payroll".to_string())), None);
    let mut inside = object_in(w.physics, "institution");
    inside.containers.push("app:payroll".to_string());
    assert_eq!(role_at(&ada, &[g.clone()], &inside, &w.tree, &Domains, NOW).map(|r| r.0), Some(Role::Editor));
    assert_eq!(role_at(&ada, &[g], &object_in(w.physics, "institution"), &w.tree, &Domains, NOW), None, "the workspace alone is not inside the app");
}

#[test]
fn a_holder_is_every_unit_where_they_hold_a_role_with_its_ancestors() {
    let w = world();
    let set = holder_set("Ada@x.edu", &[holding(w.physics, Some("chair")), holding(w.chemistry, None)], &w.tree, [], NOW);
    assert!(set.units.contains(&w.physics) && set.units.contains(&w.science) && set.units.contains(&w.root));
    assert!(!set.units.contains(&w.arts) && !set.units.contains(&w.lab), "ancestors, not descendants");
    assert_eq!(set.person, "ada@x.edu");
    assert_eq!(set.positions, vec![("chair".to_string(), w.physics)]);
}
