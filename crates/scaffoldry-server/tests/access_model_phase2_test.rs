mod common;

use scaffoldry_core::tree::OrgTree;
use scaffoldry_policy::entities::{AppCtx, PrincipalCtx, RecordCtx, Resource, WorkspaceCtx};
use scaffoldry_policy::{PolicyDecision, ScaffoldryPolicyEngine};
use scaffoldry_server::repository::OrgMove;
use scaffoldry_server::state::OrganizationNode;
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

// ---------------------------------------------------------------------------------------------
// Test 1: For a generated tree of 2,000 units, the closure equals a walk of parent_id for every pair
// ---------------------------------------------------------------------------------------------
#[test]
fn test_1_tree_of_2000_units_closure_equals_walk_of_parent_id() {
    let mut rng: u64 = 42;
    let mut next_rand = || {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        rng
    };

    let count = 2000;
    let mut ids = Vec::with_capacity(count);
    for _ in 0..count {
        ids.push(Uuid::new_v4());
    }

    let root = ids[0];
    let mut parent_map = HashMap::new();
    parent_map.insert(root, None);

    for i in 1..count {
        let parent_idx = (next_rand() as usize) % i;
        parent_map.insert(ids[i], Some(ids[parent_idx]));
    }

    let tree = OrgTree::from_parent_map("academic", &parent_map);

    let walk_ancestors = |start: Uuid| -> Vec<Uuid> {
        let mut curr = start;
        let mut path = vec![curr];
        while let Some(Some(p)) = parent_map.get(&curr) {
            curr = *p;
            path.push(curr);
        }
        path
    };

    for &node in &ids {
        let expected = walk_ancestors(node);
        let actual = tree.ancestors(node);
        assert_eq!(actual, expected, "ancestors mismatch for node {node}");
    }

    // Sample 5,000 pairs for contains check
    for _ in 0..5000 {
        let a = ids[(next_rand() as usize) % count];
        let b = ids[(next_rand() as usize) % count];

        let expected_contains = walk_ancestors(b).contains(&a);
        assert_eq!(
            tree.contains(a, b),
            expected_contains,
            "contains({a}, {b}) mismatch"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Test 2: Moving a subtree of 500 units rewrites only that subtree's closure rows and no record.
// Count both. Record the time.
// ---------------------------------------------------------------------------------------------
#[test]
fn test_2_moving_subtree_of_500_units_rewrites_only_subtree_closure_rows_and_no_record() {
    let db = common::TestDb::new();
    let state = db.state();
    let root = state
        .organizations
        .read()
        .unwrap()
        .values()
        .find(|o| o.parent_id.is_none())
        .map(|o| o.id)
        .unwrap();

    let target_a = Uuid::new_v4();
    let target_b = Uuid::new_v4();
    state
        .persist_organization(&OrganizationNode {
            id: target_a,
            parent_id: Some(root),
            name: "Target A".into(),
            code: "TGT-A".into(),
            org_type: "College".into(),
            hierarchy_key: "academic".into(),
        })
        .unwrap();
    state
        .persist_organization(&OrganizationNode {
            id: target_b,
            parent_id: Some(root),
            name: "Target B".into(),
            code: "TGT-B".into(),
            org_type: "College".into(),
            hierarchy_key: "academic".into(),
        })
        .unwrap();

    // Create a 500-unit subtree under target_a
    let subtree_root = Uuid::new_v4();
    state
        .persist_organization(&OrganizationNode {
            id: subtree_root,
            parent_id: Some(target_a),
            name: "Subtree Root".into(),
            code: "SUB-ROOT".into(),
            org_type: "Department".into(),
            hierarchy_key: "academic".into(),
        })
        .unwrap();

    let mut subtree_ids = vec![subtree_root];
    for i in 1..500 {
        let id = Uuid::new_v4();
        let parent = subtree_ids[i / 4];
        state
            .persist_organization(&OrganizationNode {
                id,
                parent_id: Some(parent),
                name: format!("Subunit {i}"),
                code: format!("SUB-{i:04}"),
                org_type: "Program".into(),
                hierarchy_key: "academic".into(),
            })
            .unwrap();
        subtree_ids.push(id);
    }

    let records_before = db.run(|c| {
        let row = c
            .query_one("SELECT COUNT(*) FROM dataset_records", &[])
            .unwrap();
        row.get::<_, i64>(0)
    });
    let non_subtree_closure_before = db.run({
        let ids = subtree_ids.clone();
        move |c| {
            let row = c
                .query_one(
                    "SELECT COUNT(*) FROM org_closure WHERE descendant != ALL($1)",
                    &[&ids],
                )
                .unwrap();
            row.get::<_, i64>(0)
        }
    });

    let start_time = Instant::now();
    let mut move_node = state
        .organizations
        .read()
        .unwrap()
        .get(&subtree_root)
        .unwrap()
        .clone();
    move_node.parent_id = Some(target_b);
    let move_res = state
        .repository
        .move_organization(&move_node)
        .expect("move organization");
    assert_eq!(move_res, OrgMove::Moved);
    let duration = start_time.elapsed();
    eprintln!("Moved 500-unit subtree in {:?}", duration);

    let records_after = db.run(|c| {
        let row = c
            .query_one("SELECT COUNT(*) FROM dataset_records", &[])
            .unwrap();
        row.get::<_, i64>(0)
    });
    assert_eq!(
        records_after, records_before,
        "No dataset_records may be touched or rewritten"
    );

    let non_subtree_closure_after = db.run({
        let ids = subtree_ids.clone();
        move |c| {
            let row = c
                .query_one(
                    "SELECT COUNT(*) FROM org_closure WHERE descendant != ALL($1)",
                    &[&ids],
                )
                .unwrap();
            row.get::<_, i64>(0)
        }
    });
    assert_eq!(
        non_subtree_closure_after, non_subtree_closure_before,
        "Non-subtree closure rows must be completely untouched"
    );

    scaffoldry_server::service::reload_after_write(&state).unwrap();
    let tree = state.tree.read().unwrap();
    assert!(tree.contains(target_b, subtree_root));
    assert!(!tree.contains(target_a, subtree_root));
    for &id in &subtree_ids {
        assert!(tree.contains(target_b, id));
        assert!(!tree.contains(target_a, id));
    }
}

// ---------------------------------------------------------------------------------------------
// Test 3: A cycle through a move is refused and the closure is unchanged
// ---------------------------------------------------------------------------------------------
#[test]
fn test_3_cycle_through_move_is_refused_and_closure_unchanged() {
    let db = common::TestDb::new();
    let state = db.state();
    let root = state
        .organizations
        .read()
        .unwrap()
        .values()
        .find(|o| o.parent_id.is_none())
        .map(|o| o.id)
        .unwrap();

    let node_a = Uuid::new_v4();
    let node_b = Uuid::new_v4();
    let node_c = Uuid::new_v4();

    state
        .persist_organization(&OrganizationNode {
            id: node_a,
            parent_id: Some(root),
            name: "A".into(),
            code: "CYC-A".into(),
            org_type: "College".into(),
            hierarchy_key: "academic".into(),
        })
        .unwrap();
    state
        .persist_organization(&OrganizationNode {
            id: node_b,
            parent_id: Some(node_a),
            name: "B".into(),
            code: "CYC-B".into(),
            org_type: "Department".into(),
            hierarchy_key: "academic".into(),
        })
        .unwrap();
    state
        .persist_organization(&OrganizationNode {
            id: node_c,
            parent_id: Some(node_b),
            name: "C".into(),
            code: "CYC-C".into(),
            org_type: "Program".into(),
            hierarchy_key: "academic".into(),
        })
        .unwrap();

    let closure_before = db.run(|c| {
        let rows = c
            .query(
                "SELECT ancestor, descendant, depth FROM org_closure ORDER BY ancestor, descendant",
                &[],
            )
            .unwrap();
        rows.into_iter()
            .map(|r| (r.get::<_, Uuid>(0), r.get::<_, Uuid>(1), r.get::<_, i16>(2)))
            .collect::<Vec<_>>()
    });

    let mut move_a_under_c = state
        .organizations
        .read()
        .unwrap()
        .get(&node_a)
        .unwrap()
        .clone();
    move_a_under_c.parent_id = Some(node_c);
    let res = state
        .repository
        .move_organization(&move_a_under_c)
        .expect("move call completes");
    assert_eq!(
        res,
        OrgMove::Cycle,
        "Move that would create a cycle must return OrgMove::Cycle"
    );

    let closure_after = db.run(|c| {
        let rows = c
            .query(
                "SELECT ancestor, descendant, depth FROM org_closure ORDER BY ancestor, descendant",
                &[],
            )
            .unwrap();
        rows.into_iter()
            .map(|r| (r.get::<_, Uuid>(0), r.get::<_, Uuid>(1), r.get::<_, i16>(2)))
            .collect::<Vec<_>>()
    });
    assert_eq!(
        closure_after, closure_before,
        "Closure must be completely unchanged after refused cycle move"
    );
}

// ---------------------------------------------------------------------------------------------
// Test 4: A second root in academic is refused by the unique index
// ---------------------------------------------------------------------------------------------
#[test]
fn test_4_second_root_in_academic_is_refused_by_unique_index() {
    let db = common::TestDb::new();
    let state = db.state();

    let second_root = OrganizationNode {
        id: Uuid::new_v4(),
        parent_id: None,
        name: "Second Root University".into(),
        code: "ROOT2".into(),
        org_type: "Institution".into(),
        hierarchy_key: "academic".into(),
    };

    let res = state.persist_organization(&second_root);
    assert!(
        res.is_err(),
        "Second root in academic must be refused by idx_one_root_per_hierarchy"
    );

    let sql_res = db.run(|c| {
        c.execute(
            "INSERT INTO organizations (id, name, code, org_type, hierarchy_key) VALUES ($1, $2, $3, $4, $5)",
            &[&Uuid::new_v4(), &"Root2", &"ROOT2", &"Institution", &"academic"],
        )
    });
    assert!(sql_res.is_err());
    let err = sql_res.unwrap_err();
    let db_err = err.as_db_error().expect("must be a database error");
    assert_eq!(
        db_err.constraint(),
        Some("idx_one_root_per_hierarchy"),
        "Constraint must be idx_one_root_per_hierarchy"
    );
}

// ---------------------------------------------------------------------------------------------
// Test 5: A grep of the server for the four old walker names prints nothing
// ---------------------------------------------------------------------------------------------
#[test]
fn test_5_grep_of_server_for_four_old_walker_names_prints_nothing() {
    let server_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let forbidden_names = [
        "unit_ids_for",
        "unit_in_scope",
        "fn within",
        "members::within",
        "fn walk_up",
    ];

    let mut found_violations = Vec::new();

    fn visit_dir(dir: &std::path::Path, forbidden: &[&str], violations: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit_dir(&path, forbidden, violations);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                for (line_no, line) in text.lines().enumerate() {
                    for &term in forbidden {
                        if line.contains(term) {
                            violations.push(format!(
                                "{}:{}: found forbidden walker {}",
                                path.display(),
                                line_no + 1,
                                term
                            ));
                        }
                    }
                }
            }
        }
    }

    visit_dir(&server_src, &forbidden_names, &mut found_violations);
    assert!(
        found_violations.is_empty(),
        "Old walker names found in scaffoldry-server/src:\n{}",
        found_violations.join("\n")
    );
}

// ---------------------------------------------------------------------------------------------
// Test 6: Each of the five rewritten policies allows a person in a descendant unit and denies a person outside
// ---------------------------------------------------------------------------------------------
#[test]
fn test_6_five_rewritten_policies_allow_descendant_and_deny_outside() {
    let engine = ScaffoldryPolicyEngine::default_institutional_engine().expect("engine");

    let college_id = "00000000-0000-0000-0000-000000000010";
    let physics_dept_id = "00000000-0000-0000-0000-000000000020";
    let biology_dept_id = "00000000-0000-0000-0000-000000000030";

    // Principal in Physics (child of College)
    let physics_principal = PrincipalCtx {
        eppn: "einstein@physics.state.edu".into(),
        name: "Albert Einstein".into(),
        scoped_affiliation: "faculty".into(),
        department: "physics".into(),
        unit_ids: vec![physics_dept_id.into(), college_id.into()],
        admin_unit_ids: vec![],
        is_platform_admin: false,
    };

    // Principal in Biology (outside Physics)
    let bio_principal = PrincipalCtx {
        eppn: "curie@biology.state.edu".into(),
        name: "Marie Curie".into(),
        scoped_affiliation: "faculty".into(),
        department: "biology".into(),
        unit_ids: vec![biology_dept_id.into()],
        admin_unit_ids: vec![],
        is_platform_admin: false,
    };

    // 1. Policy 1 (Read / Write Record)
    let record_res = Resource::Record(RecordCtx {
        app_slug: "physics-app".into(),
        department: physics_dept_id.into(),
        workspace_id: "ws-physics".into(),
        is_ferpa_sensitive: false,
        categories: Default::default(),
    });
    let dec_in =
        scaffoldry_policy::entities::authorize(&engine, &physics_principal, "read", &record_res)
            .unwrap();
    assert_eq!(
        dec_in.decision,
        PolicyDecision::Allow,
        "Policy 1 must allow descendant unit"
    );
    let dec_out =
        scaffoldry_policy::entities::authorize(&engine, &bio_principal, "read", &record_res)
            .unwrap();
    assert_eq!(
        dec_out.decision,
        PolicyDecision::Deny,
        "Policy 1 must deny outside unit"
    );

    // 2. Policy 3 (Export Record)
    let dec_in =
        scaffoldry_policy::entities::authorize(&engine, &physics_principal, "export", &record_res)
            .unwrap();
    assert_eq!(
        dec_in.decision,
        PolicyDecision::Allow,
        "Policy 3 must allow descendant unit"
    );
    let dec_out =
        scaffoldry_policy::entities::authorize(&engine, &bio_principal, "export", &record_res)
            .unwrap();
    assert_eq!(
        dec_out.decision,
        PolicyDecision::Deny,
        "Policy 3 must deny outside unit"
    );

    // 3. Policy 9 (Access Departmental Workspace)
    let ws_res = Resource::Workspace(WorkspaceCtx {
        workspace_id: "ws-physics".into(),
        department: physics_dept_id.into(),
        visibility: "departmental".into(),
        data_classification: "Internal".into(),
        member_role: "".into(),
        unit_id: physics_dept_id.into(),
        unit_path_ids: vec![physics_dept_id.into(), college_id.into()],
        is_member: false,
        is_ferpa_sensitive: false,
        categories: Default::default(),
        domain_path: Default::default(),
        sensitivity_rank: 2,
    });
    let dec_in = scaffoldry_policy::entities::authorize(
        &engine,
        &physics_principal,
        "access_workspace",
        &ws_res,
    )
    .unwrap();
    assert_eq!(
        dec_in.decision,
        PolicyDecision::Allow,
        "Policy 9 must allow descendant unit"
    );
    let dec_out = scaffoldry_policy::entities::authorize(
        &engine,
        &bio_principal,
        "access_workspace",
        &ws_res,
    )
    .unwrap();
    assert_eq!(
        dec_out.decision,
        PolicyDecision::Deny,
        "Policy 9 must deny outside unit"
    );

    // 4. Policy 14 (Create / Update / Publish App)
    let app_res = Resource::App(AppCtx {
        slug: "physics-app".into(),
        department: physics_dept_id.into(),
        workspace_id: "ws-physics".into(),
    });
    let dec_in = scaffoldry_policy::entities::authorize(
        &engine,
        &physics_principal,
        "publish_app",
        &app_res,
    )
    .unwrap();
    assert_eq!(
        dec_in.decision,
        PolicyDecision::Allow,
        "Policy 14 must allow descendant unit"
    );
    let dec_out =
        scaffoldry_policy::entities::authorize(&engine, &bio_principal, "publish_app", &app_res)
            .unwrap();
    assert_eq!(
        dec_out.decision,
        PolicyDecision::Deny,
        "Policy 14 must deny outside unit"
    );

    // 5. Policy 16 (Publish Dataset)
    let dec_in = scaffoldry_policy::entities::authorize(
        &engine,
        &physics_principal,
        "publish_dataset",
        &app_res,
    )
    .unwrap();
    assert_eq!(
        dec_in.decision,
        PolicyDecision::Allow,
        "Policy 16 must allow descendant unit"
    );
    let dec_out = scaffoldry_policy::entities::authorize(
        &engine,
        &bio_principal,
        "publish_dataset",
        &app_res,
    )
    .unwrap();
    assert_eq!(
        dec_out.decision,
        PolicyDecision::Deny,
        "Policy 16 must deny outside unit"
    );
}

// ---------------------------------------------------------------------------------------------
// Test 7: Rule 4 source scan passes
// ---------------------------------------------------------------------------------------------
#[test]
fn test_7_rule4_source_scan_passes() {
    // Rule 4: No hierarchy key, unit id, domain key, or position key is written in code or in a Cedar policy.
    let engine = scaffoldry_policy::ScaffoldryPolicyEngine::default_institutional_engine().unwrap();
    let cedar_policies = engine.source();
    assert!(
        !cedar_policies.contains("academic"),
        "No hierarchy key may be hardcoded in Cedar policy: found 'academic'"
    );
    assert!(
        !cedar_policies.contains("\"chair\"") && !cedar_policies.contains("\"dean\""),
        "No position key may be hardcoded in Cedar default institutional policy"
    );
    assert!(
        !cedar_policies.contains("00000000-0000"),
        "No unit UUID may be hardcoded in Cedar default policy"
    );
}
