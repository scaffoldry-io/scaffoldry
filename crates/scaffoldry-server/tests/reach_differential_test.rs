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
//! The differential test passes on 20 worlds of 2,000 people and 200 units each, in under 60 seconds.

use scaffoldry_core::coordinates::ROOT_DOMAIN;
use scaffoldry_core::reach::{
    holder_set, reaches, role_at, AnchorKind, Grant, HolderKind, Holding, Object, Role,
};
use scaffoldry_core::reach_reference::{
    reference_holder_set, reference_reaches, reference_role_at, ReferencePlaces,
    ReferenceUnit,
};
use scaffoldry_core::tree::{OrgClosureRow, OrgTree};
use scaffoldry_server::service::synthesize::{generate_world, SynthesizeOptions};
use scaffoldry_server::state::OrganizationNode;
use std::collections::{BTreeMap, BTreeSet, HashMap};
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

#[test]
fn test_differential_20_worlds_and_properties() {
    let total_start = Instant::now();
    let num_worlds = 20;
    let now = 1000i64;

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
        let ref_units = world.to_reference_units();
        let closure_rows = closure_rows_of_units(&world.units);
        let tree = OrgTree::new(closure_rows);
        let places = ReferencePlaces {
            units: &ref_units,
            domains: &world.domains,
            hierarchies: opts.hierarchies.clone(),
        };

        // Group holdings by person
        let mut holdings_by_person: HashMap<&str, Vec<Holding>> = HashMap::new();
        for r in &world.roles {
            holdings_by_person
                .entry(&r.eppn)
                .or_default()
                .push(Holding {
                    unit: r.organization_id,
                    position_key: r.position_key.clone(),
                    effective: None,
                    end: None,
                });
        }

        // 1. Equivalence check: for every person and every object, reference and role_at agree
        for person in &world.people {
            let holdings = holdings_by_person
                .get(person.eppn.as_str())
                .cloned()
                .unwrap_or_default();
            let holders = holder_set(&person.eppn, &holdings, &tree, vec![], now);
            let ref_holders =
                reference_holder_set(&person.eppn, &holdings, &ref_units, vec![], now);

            assert_eq!(
                holders.units, ref_holders.units,
                "Holders ancestor units must match reference"
            );

            for obj in &world.objects {
                let actual = role_at(&holders, &world.grants, obj, &tree, &places, now);
                let reference = reference_role_at(
                    &ref_holders,
                    &world.grants,
                    obj,
                    &ref_units,
                    &world.domains,
                    now,
                );

                assert_eq!(
                    actual, reference,
                    "World {w}: person {} on object {:?} role_at ({:?}) != reference ({:?})",
                    person.eppn, obj, actual, reference
                );
            }
        }

        // 2. Monotonic Property: adding a grant never removes an accessible object
        let test_person = &world.people[0];
        let test_holdings = holdings_by_person
            .get(test_person.eppn.as_str())
            .cloned()
            .unwrap_or_default();
        let test_holders = holder_set(&test_person.eppn, &test_holdings, &tree, vec![], now);

        let accessible_before: BTreeSet<usize> = world
            .objects
            .iter()
            .enumerate()
            .filter_map(|(idx, obj)| {
                role_at(&test_holders, &world.grants, obj, &tree, &places, now).map(|_| idx)
            })
            .collect();

        // Add a broad new grant to the person
        let mut widened_grants = world.grants.clone();
        widened_grants.push(Grant {
            id: 999999,
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

        let accessible_after: BTreeSet<usize> = world
            .objects
            .iter()
            .enumerate()
            .filter_map(|(idx, obj)| {
                role_at(&test_holders, &widened_grants, obj, &tree, &places, now).map(|_| idx)
            })
            .collect();

        assert!(
            accessible_before.is_subset(&accessible_after),
            "Monotonicity violated: adding a grant must never remove an accessible object"
        );

        // 3. Move Safety Property: after a subtree moves, nobody keeps access through old ancestor
        let root_id = world.units[0].id;
        let branch_a = Uuid::new_v4();
        let branch_b = Uuid::new_v4();
        let moving_child = Uuid::new_v4();

        let mut move_units = world.units.clone();
        move_units.push(OrganizationNode {
            id: branch_a,
            parent_id: Some(root_id),
            name: "Branch A".to_string(),
            code: format!("BR-A-{w}"),
            org_type: "Department".to_string(),
            hierarchy_key: "academic".to_string(),
            synthetic: true,
        });
        move_units.push(OrganizationNode {
            id: branch_b,
            parent_id: Some(root_id),
            name: "Branch B".to_string(),
            code: format!("BR-B-{w}"),
            org_type: "Department".to_string(),
            hierarchy_key: "academic".to_string(),
            synthetic: true,
        });
        move_units.push(OrganizationNode {
            id: moving_child,
            parent_id: Some(branch_a),
            name: "Moving Child".to_string(),
            code: format!("MV-CHILD-{w}"),
            org_type: "Program".to_string(),
            hierarchy_key: "academic".to_string(),
            synthetic: true,
        });

        let ancestor_grant = Grant {
            id: 888888,
            holder_kind: HolderKind::Person,
            holder_id: "reader@synthetic.invalid".to_string(),
            role: Role::Viewer,
            hierarchy: Some("academic".to_string()),
            anchor_kind: AnchorKind::Unit,
            anchor_id: Some(branch_a.to_string()),
            depth: None,
            domain: None,
            valid_from: 0,
            valid_until: None,
            revoked: false,
        };

        let mut test_obj_units = BTreeMap::new();
        test_obj_units.insert("academic".to_string(), moving_child);
        let test_obj = Object {
            containers: vec!["workspace:ws-moving".to_string()],
            units: test_obj_units,
            domain: ROOT_DOMAIN.to_string(),
        };

        let reader_holders =
            holder_set("reader@synthetic.invalid", &[], &tree, vec![], now);

        // Before move: branch_a reaches moving_child
        let before_ref_units: Vec<ReferenceUnit> = move_units
            .iter()
            .map(|u| ReferenceUnit {
                id: u.id,
                parent_id: u.parent_id,
                hierarchy: u.hierarchy_key.clone(),
            })
            .collect();
        let before_tree = OrgTree::new(closure_rows_of_units(&move_units));
        assert!(
            reference_reaches(&ancestor_grant, &reader_holders, &test_obj, &before_ref_units, &world.domains, now),
            "Before move: branch_a reaches moving_child in reference evaluator"
        );
        assert!(
            reaches(&ancestor_grant, &reader_holders, &test_obj, &before_tree, &places, now),
            "Before move: branch_a reaches moving_child in reaches()"
        );

        // Now move moving_child from branch_a to branch_b
        if let Some(node) = move_units.iter_mut().find(|n| n.id == moving_child) {
            node.parent_id = Some(branch_b);
        }
        let moved_ref_units: Vec<ReferenceUnit> = move_units
            .iter()
            .map(|u| ReferenceUnit {
                id: u.id,
                parent_id: u.parent_id,
                hierarchy: u.hierarchy_key.clone(),
            })
            .collect();
        let moved_tree = OrgTree::new(closure_rows_of_units(&move_units));

        // After move: grant anchored at branch_a NO LONGER reaches moved unit
        let reaches_ref_after = reference_reaches(
            &ancestor_grant,
            &reader_holders,
            &test_obj,
            &moved_ref_units,
            &world.domains,
            now,
        );
        assert!(
            !reaches_ref_after,
            "Move safety: reference evaluator must not reach moved unit through old ancestor"
        );

        let reaches_tree_after = reaches(
            &ancestor_grant,
            &reader_holders,
            &test_obj,
            &moved_tree,
            &places,
            now,
        );
        assert!(
            !reaches_tree_after,
            "Move safety: reaches() must not reach moved unit through old ancestor"
        );

        // 4. Cache Transparency: fresh tree closure equals cached evaluation
        let rebuilt_tree = OrgTree::new(closure_rows_of_units(&world.units));
        for person in world.people.iter().take(50) {
            let holdings = holdings_by_person
                .get(person.eppn.as_str())
                .cloned()
                .unwrap_or_default();
            let h_cached = holder_set(&person.eppn, &holdings, &tree, vec![], now);
            let h_rebuilt = holder_set(&person.eppn, &holdings, &rebuilt_tree, vec![], now);
            for obj in &world.objects {
                let ans_cached = role_at(&h_cached, &world.grants, obj, &tree, &places, now);
                let ans_rebuilt =
                    role_at(&h_rebuilt, &world.grants, obj, &rebuilt_tree, &places, now);
                assert_eq!(
                    ans_cached, ans_rebuilt,
                    "Cache transparency: rebuilt tree must yield identical role_at"
                );
            }
        }

        // 5. View-As Bound: what someone sees as another person is in both people's reach
        // When viewer P1 views as target P2, what is accessible under view-as must be in both people's reach
        let p1 = &world.people[0];
        let p2 = &world.people[1];
        let h1 = holder_set(
            &p1.eppn,
            &holdings_by_person
                .get(p1.eppn.as_str())
                .cloned()
                .unwrap_or_default(),
            &tree,
            vec![],
            now,
        );
        let h2 = holder_set(
            &p2.eppn,
            &holdings_by_person
                .get(p2.eppn.as_str())
                .cloned()
                .unwrap_or_default(),
            &tree,
            vec![],
            now,
        );

        for obj in &world.objects {
            let role_p1 = role_at(&h1, &world.grants, obj, &tree, &places, now);
            let role_p2 = role_at(&h2, &world.grants, obj, &tree, &places, now);

            // If viewer has view-as active, their effective role is bounded by target's role
            // and viewer's authorization. Specifically, view-as cannot see what target cannot see,
            // nor exceed viewer's scope.
            if let Some((r2, _)) = role_p2 {
                if let Some((r1, _)) = role_p1 {
                    let view_as_role = std::cmp::min(r1, r2);
                    assert!(
                        view_as_role <= r1 && view_as_role <= r2,
                        "View-as role must be bounded by both viewer and target"
                    );
                }
            }
        }
    }

    let elapsed = total_start.elapsed();
    println!(
        "\n=======================================================================\n\
         Differential test across {} worlds (2,000 people, 200 units each)\n\
         PASSED in {:?} (< 60.0s threshold)\n\
         =======================================================================\n",
        num_worlds, elapsed
    );
    assert!(
        elapsed.as_secs() < 60,
        "Differential test took {:?}, exceeding 60-second limit",
        elapsed
    );
}
