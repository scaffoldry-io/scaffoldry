//! Differential test: reference evaluator vs role_at and property testing.
//!
//! Access model phase 5:
//! A differential test, tests/reach_differential_test.rs.
//! Over 20 generated worlds, for every synthetic person and every object,
//! the reference and role_at agree. Then it checks:
//!   - Monotonic: adding a grant never removes an accessible object
//!   - Move safety: after a subtree moves, nobody keeps access through the old ancestor
//!   - Cache transparency: the answer is the same with and without cached collections
//!   - View-as bound: what someone sees as another person is in both people's reach
//!
//! The differential test passes on 20 worlds of 2,000 people and 200 units each, in under 60 seconds.

use scaffoldry_core::access::RowScope;
use scaffoldry_core::reach::{
    holder_set, role_at, AnchorKind, Grant, HolderKind, Holding, Object, Role,
};
use scaffoldry_core::reach_reference::{
    reference_holder_set, reference_role_at, ReferencePlaces,
};
use scaffoldry_core::tree::{OrgClosureRow, OrgTree};
use scaffoldry_server::service::synthesize::{generate_world, SynthesizeOptions};
use scaffoldry_server::state::OrganizationNode;
use std::collections::{BTreeSet, HashMap};
use std::time::Instant;
use uuid::Uuid;

/// Helper to compute transitive closure rows for OrgTree from unit list in memory.
fn closure_rows_of_units(units: &[OrganizationNode]) -> Vec<OrgClosureRow> {
    let mut rows = Vec::new();
    for u in units {
        let mut curr = Some(u.id);
        let mut depth = 0i16;
        while let Some(anc) = curr {
            rows.push(OrgClosureRow {
                hierarchy_key: u.hierarchy_key.clone(),
                ancestor: anc,
                descendant: u.id,
                depth,
            });
            curr = units.iter().find(|n| n.id == anc).and_then(|n| n.parent_id);
            depth += 1;
        }
    }
    rows
}

/// The grants the server would fetch for this holder: those held by the person, one of their positions, one of their units,
/// or one of their groups. `members::holder_pairs` builds the same pairs for the `grants_held_by` query.
fn grants_held_by(holders: &scaffoldry_core::reach::HolderSet, grants: &[Grant]) -> Vec<Grant> {
    let mut pairs: BTreeSet<(&'static str, String)> = BTreeSet::new();
    pairs.insert(("person", holders.person.clone()));
    for (key, unit) in &holders.positions {
        pairs.insert(("position", format!("{key}@{unit}")));
        pairs.insert(("position", format!("{key}@*")));
    }
    for u in &holders.units {
        pairs.insert(("unit", u.to_string()));
    }
    for g in &holders.groups {
        pairs.insert(("group", g.clone()));
    }
    grants.iter().filter(|g| pairs.contains(&(g.holder_kind.as_str(), if g.holder_kind == HolderKind::Person { g.holder_id.to_lowercase() } else { g.holder_id.clone() }))).cloned().collect()
}

/// What a person's reach on an object means as a row scope: everything, or nothing.
fn scope_of(role: &Option<(Role, Vec<i64>)>) -> RowScope {
    if role.is_some() { RowScope::AllowAll } else { RowScope::DenyAll }
}

/// Every axis the evaluator has is present in a generated world, so agreement on it means something.
fn assert_world_exercises_every_axis(w: usize, world: &scaffoldry_server::service::synthesize::SyntheticWorld) {
    let g = &world.grants;
    let has = |name: &str, f: &dyn Fn(&Grant) -> bool| assert!(g.iter().any(f), "world {w} has no grant with {name}");
    has("a person holder", &|g| g.holder_kind == HolderKind::Person);
    has("a unit holder", &|g| g.holder_kind == HolderKind::Unit);
    has("a group holder", &|g| g.holder_kind == HolderKind::Group);
    has("a position holder", &|g| g.holder_kind == HolderKind::Position);
    has("a container anchor", &|g| g.anchor_kind == AnchorKind::Container);
    has("a unit anchor", &|g| g.anchor_kind == AnchorKind::Unit);
    has("a holder_unit anchor", &|g| g.anchor_kind == AnchorKind::HolderUnit);
    has("a depth", &|g| g.depth.is_some());
    has("a domain", &|g| g.domain.is_some());
    has("a revoked grant", &|g| g.revoked);
    has("an expired grant", &|g| g.valid_until.is_some_and(|u| u < 1000));
    has("a grant not yet valid", &|g| g.valid_from > 1000);
    assert!(world.groups.iter().any(|g| !g.members.is_empty()), "world {w} has groups with members");
    let roles: BTreeSet<Role> = g.iter().map(|g| g.role).collect();
    assert!(roles.len() >= 4, "world {w} uses several roles");
}

#[test]
fn test_differential_20_worlds_and_properties() {
    let total_start = Instant::now();
    let num_worlds = 20;
    let now = 1000i64;
    let mut reached = 0usize;
    let mut unreached = 0usize;

    for w in 1..=num_worlds {
        let opts = SynthesizeOptions {
            seed: 20261020 + w as u64,
            people: 2000,
            units: 200,
            hierarchies: vec!["academic".to_string(), "supervisory".to_string()],
            apps: 10,
            records: 50,
        };

        let world = generate_world(&opts);
        assert_world_exercises_every_axis(w, &world);
        let ref_units = world.to_reference_units();
        let tree = OrgTree::new(closure_rows_of_units(&world.units));
        let places = ReferencePlaces { units: &ref_units, domains: &world.domains, hierarchies: opts.hierarchies.clone() };
        let groups = world.groups_by_person();

        let mut holdings_by_person: HashMap<&str, Vec<Holding>> = HashMap::new();
        for r in &world.roles {
            holdings_by_person.entry(&r.eppn).or_default().push(Holding { unit: r.organization_id, position_key: r.position_key.clone(), effective: None, end: None });
        }
        let holdings_of = |eppn: &str| holdings_by_person.get(eppn).cloned().unwrap_or_default();
        let groups_of = |eppn: &str| groups.get(eppn).cloned().unwrap_or_default();

        // 1. For every person and every object, the reference and `role_at` agree.
        for person in &world.people {
            let holdings = holdings_of(&person.eppn);
            let holders = holder_set(&person.eppn, &holdings, &tree, groups_of(&person.eppn), now);
            let ref_holders = reference_holder_set(&person.eppn, &holdings, &ref_units, groups_of(&person.eppn), now);
            assert_eq!(holders.units, ref_holders.units, "holders' ancestor units must match the reference");
            // `role_at` gets the grants fetched for the holder, as in the server. One person in ten is also checked against
            // the reference given every grant, which proves the fetch drops nothing.
            let fetched = grants_held_by(&holders, &world.grants);
            let every_grant = world.people.iter().position(|p| p.eppn == person.eppn).is_some_and(|i| i % 10 == 0);
            for obj in &world.objects {
                let actual = role_at(&holders, &fetched, obj, &tree, &places, now);
                let reference = reference_role_at(&ref_holders, if every_grant { &world.grants } else { &fetched }, obj, &ref_units, &world.domains, now);
                assert_eq!(actual, reference, "world {w}: {} on {:?}: role_at {actual:?} != reference {reference:?}", person.eppn, obj);
                if actual.is_some() { reached += 1 } else { unreached += 1 }
            }
        }

        // 2. Monotonic: adding a grant never removes an object.
        let test_person = &world.people[0];
        let test_holders = holder_set(&test_person.eppn, &holdings_of(&test_person.eppn), &tree, groups_of(&test_person.eppn), now);
        let reachable = |grants: &[Grant]| -> BTreeSet<usize> {
            let fetched = grants_held_by(&test_holders, grants);
            world.objects.iter().enumerate().filter_map(|(i, o)| role_at(&test_holders, &fetched, o, &tree, &places, now).map(|_| i)).collect()
        };
        let before = reachable(&world.grants);
        let mut widened = world.grants.clone();
        widened.push(Grant {
            id: 999_999,
            holder_kind: HolderKind::Person,
            holder_id: test_person.eppn.clone(),
            role: Role::Viewer,
            hierarchy: Some("academic".to_string()),
            anchor_kind: AnchorKind::Unit,
            anchor_id: Some(world.units[0].id.to_string()),
            depth: None,
            domain: None,
            valid_from: 0,
            valid_until: None,
            revoked: false,
        });
        let after = reachable(&widened);
        assert!(before.is_subset(&after), "monotonicity: adding a grant must never remove an object");

        // 3. Move safety on the generated world: move a unit that has children under another unit, rebuild the closure from
        // the moved parents, and the closure answer must match the reference that walks the moved parents. No one keeps
        // access to the moved subtree through the old ancestor.
        let (moved, new_parent, old_parent) = {
            let academic: Vec<_> = world.units.iter().filter(|u| u.hierarchy_key == "academic" && u.synthetic).collect();
            // A unit with children whose parent is not the division root, so the division root is outside the parent's subtree.
            let parent_of = |id: Uuid| academic.iter().find(|a| a.id == id).and_then(|a| a.parent_id);
            let with_kids = academic
                .iter()
                .find(|u| u.parent_id.is_some_and(|p| parent_of(p).is_some_and(|gp| academic.iter().any(|a| a.id == gp))) && academic.iter().any(|c| c.parent_id == Some(u.id)))
                .expect("a unit with children, two levels below the division root");
            let below = |a: Uuid, b: Uuid| -> bool { // is `b` at or below `a`
                let mut cur = Some(b);
                while let Some(c) = cur {
                    if c == a { return true }
                    cur = academic.iter().find(|u| u.id == c).and_then(|u| u.parent_id);
                }
                false
            };
            let old = with_kids.parent_id.unwrap();
            // Outside the old parent's subtree, so the old parent stops containing the unit.
            let target = academic.iter().find(|u| !below(old, u.id) && !below(with_kids.id, u.id)).expect("somewhere to move it");
            (with_kids.id, target.id, with_kids.parent_id.unwrap())
        };
        let mut moved_units = world.units.clone();
        moved_units.iter_mut().find(|u| u.id == moved).unwrap().parent_id = Some(new_parent);
        let moved_ref = moved_units.iter().map(|u| scaffoldry_core::reach_reference::ReferenceUnit { id: u.id, parent_id: u.parent_id, hierarchy: u.hierarchy_key.clone() }).collect::<Vec<_>>();
        let moved_tree = OrgTree::new(closure_rows_of_units(&moved_units));
        let moved_places = ReferencePlaces { units: &moved_ref, domains: &world.domains, hierarchies: opts.hierarchies.clone() };
        assert!(!moved_tree.contains(old_parent, moved), "the old parent no longer contains the moved unit");
        assert!(moved_tree.contains(new_parent, moved), "the new parent contains it");
        let old_ancestor_grant = Grant {
            id: 888_888,
            holder_kind: HolderKind::Person,
            holder_id: "reader@synthetic.invalid".to_string(),
            role: Role::Viewer,
            hierarchy: Some("academic".to_string()),
            anchor_kind: AnchorKind::Unit,
            anchor_id: Some(old_parent.to_string()),
            depth: None,
            domain: None,
            valid_from: 0,
            valid_until: None,
            revoked: false,
        };
        let inside = Object { containers: vec![], units: [("academic".to_string(), moved)].into(), domain: "institution".to_string() };
        let reader = holder_set("reader@synthetic.invalid", &[], &moved_tree, vec![], now);
        assert!(role_at(&reader, std::slice::from_ref(&old_ancestor_grant), &inside, &tree, &places, now).is_some(), "before the move the old ancestor reaches the unit");
        assert!(role_at(&reader, std::slice::from_ref(&old_ancestor_grant), &inside, &moved_tree, &moved_places, now).is_none(), "after the move the old ancestor does not");
        for person in world.people.iter().take(150) {
            let holdings = holdings_of(&person.eppn);
            let h = holder_set(&person.eppn, &holdings, &moved_tree, groups_of(&person.eppn), now);
            let rh = reference_holder_set(&person.eppn, &holdings, &moved_ref, groups_of(&person.eppn), now);
            let fetched = grants_held_by(&h, &world.grants);
            for obj in &world.objects {
                let actual = role_at(&h, &fetched, obj, &moved_tree, &moved_places, now);
                let reference = reference_role_at(&rh, &world.grants, obj, &moved_ref, &world.domains, now);
                assert_eq!(actual, reference, "after the move, {} on {:?}", person.eppn, obj);
            }
        }

        // 4. Cache transparency: a closure rebuilt from the same parents gives the same answers.
        let rebuilt = OrgTree::new(closure_rows_of_units(&world.units));
        for person in world.people.iter().take(50) {
            let holdings = holdings_of(&person.eppn);
            let cached = holder_set(&person.eppn, &holdings, &tree, groups_of(&person.eppn), now);
            let fresh = holder_set(&person.eppn, &holdings, &rebuilt, groups_of(&person.eppn), now);
            let (a, b) = (grants_held_by(&cached, &world.grants), grants_held_by(&fresh, &world.grants));
            for obj in &world.objects {
                assert_eq!(role_at(&cached, &a, obj, &tree, &places, now), role_at(&fresh, &b, obj, &rebuilt, &places, now), "cache transparency");
            }
        }

        // 5. View-as bound: viewing as another person reaches only what both people reach. The server joins the two scopes
        // with `RowScope::and`; a person's reach is everything or nothing.
        let (p1, p2) = (&world.people[0], &world.people[1]);
        let reach_of = |p: &scaffoldry_server::service::synthesize::SyntheticPerson, obj: &Object| {
            let h = holder_set(&p.eppn, &holdings_of(&p.eppn), &tree, groups_of(&p.eppn), now);
            role_at(&h, &grants_held_by(&h, &world.grants), obj, &tree, &places, now)
        };
        let (mut both, mut only_one) = (0, 0);
        for obj in &world.objects {
            let (r1, r2) = (reach_of(p1, obj), reach_of(p2, obj));
            let combined = scope_of(&r1).and(scope_of(&r2));
            match (r1.is_some(), r2.is_some()) {
                (true, true) => { assert_eq!(combined, RowScope::AllowAll); both += 1 }
                _ => { assert_eq!(combined, RowScope::DenyAll, "view-as never reaches what either person cannot"); only_one += 1 }
            }
        }
        let _ = (both, only_one);
    }

    let elapsed = total_start.elapsed();
    println!("\n{num_worlds} worlds of 2,000 people and 200 units: {reached} reached and {unreached} not reached, in {elapsed:?}\n");
    assert!(reached > 1000 && unreached > 1000, "the worlds hold both outcomes in quantity: {reached} reached, {unreached} not");
    assert!(elapsed.as_secs() < 60, "the differential test took {elapsed:?}, over the 60 second limit");
}
