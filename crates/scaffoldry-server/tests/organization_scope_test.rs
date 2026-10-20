mod common;

use scaffoldry_server::service::organizations::{unit_in_reach, OrgCaller};
use scaffoldry_server::state::{OrganizationNode, RoleRow};
use uuid::Uuid;

#[test]
fn test_organization_scope_rules() {
    let _db = common::TestDb::new();
    let root_id = Uuid::new_v4();
    let college_id = Uuid::new_v4();
    let dept_physics_id = Uuid::new_v4();
    let dept_biology_id = Uuid::new_v4();

    let orgs = [
        OrganizationNode {
            id: root_id,
            parent_id: None,
            name: "State University".to_string(),
            code: "INST".to_string(),
            org_type: "Institution".to_string(),
            hierarchy_key: "academic".to_string(),
        },
        OrganizationNode {
            id: college_id,
            parent_id: Some(root_id),
            name: "College of Sciences".to_string(),
            code: "SCI".to_string(),
            org_type: "College".to_string(),
            hierarchy_key: "academic".to_string(),
        },
        OrganizationNode {
            id: dept_physics_id,
            parent_id: Some(college_id),
            name: "Department of Physics".to_string(),
            code: "PHYS".to_string(),
            org_type: "Department".to_string(),
            hierarchy_key: "academic".to_string(),
        },
        OrganizationNode {
            id: dept_biology_id,
            parent_id: Some(college_id),
            name: "Department of Biology".to_string(),
            code: "BIO".to_string(),
            org_type: "Department".to_string(),
            hierarchy_key: "academic".to_string(),
        },
    ];

    let parent_map: std::collections::HashMap<Uuid, Option<Uuid>> =
        orgs.iter().map(|o| (o.id, o.parent_id)).collect();
    let tree = scaffoldry_core::tree::OrgTree::from_parent_map("academic", &parent_map);

    let roles = vec![
        RoleRow {
            id: Uuid::new_v4(),
            person_id: Uuid::new_v4(),
            eppn: "dean.sciences@state.edu".to_string(),
            organization_id: college_id,
            role_title: "Dean of Sciences".to_string(),
            scoped_affiliation: "unit_admin".to_string(),
            is_primary: true,
            source: "api".to_string(),
            position_key: None,
        },
        RoleRow {
            id: Uuid::new_v4(),
            person_id: Uuid::new_v4(),
            eppn: "chair.physics@state.edu".to_string(),
            organization_id: dept_physics_id,
            role_title: "Physics Department Chair".to_string(),
            scoped_affiliation: "unit_admin".to_string(),
            is_primary: true,
            source: "api".to_string(),
            position_key: None,
        },
    ];

    let dean_caller = OrgCaller {
        eppn: "dean.sciences@state.edu".to_string(),
        affiliation: "faculty".to_string(),
    };
    let chair_caller = OrgCaller {
        eppn: "chair.physics@state.edu".to_string(),
        affiliation: "faculty".to_string(),
    };
    let central_admin_caller = OrgCaller {
        eppn: "jordan.lee@state.edu".to_string(),
        affiliation: "central_admin".to_string(),
    };

    // 1. College admin is in reach for a child department
    assert!(unit_in_reach(&dean_caller, dept_physics_id, &tree, &roles));
    assert!(unit_in_reach(&dean_caller, dept_biology_id, &tree, &roles));
    assert!(unit_in_reach(&dean_caller, college_id, &tree, &roles));
    assert!(!unit_in_reach(&dean_caller, root_id, &tree, &roles));

    // 2. Department admin is not in reach for a sibling department or for the college parent
    assert!(unit_in_reach(&chair_caller, dept_physics_id, &tree, &roles));
    assert!(!unit_in_reach(
        &chair_caller,
        dept_biology_id,
        &tree,
        &roles
    ));
    assert!(!unit_in_reach(&chair_caller, college_id, &tree, &roles));
    assert!(!unit_in_reach(&chair_caller, root_id, &tree, &roles));

    // 3. central_admin is in reach for every node
    assert!(unit_in_reach(&central_admin_caller, root_id, &tree, &roles));
    assert!(unit_in_reach(
        &central_admin_caller,
        college_id,
        &tree,
        &roles
    ));
    assert!(unit_in_reach(
        &central_admin_caller,
        dept_physics_id,
        &tree,
        &roles
    ));
    assert!(unit_in_reach(
        &central_admin_caller,
        dept_biology_id,
        &tree,
        &roles
    ));

    // 4. A cycle in the fixture (A.parent = B, B.parent = A) returns false and does not loop.
    let cycle_a_id = Uuid::new_v4();
    let cycle_b_id = Uuid::new_v4();
    let cycle_orgs = [
        OrganizationNode {
            id: cycle_a_id,
            parent_id: Some(cycle_b_id),
            name: "Cycle A".to_string(),
            code: "CYC-A".to_string(),
            org_type: "Department".to_string(),
            hierarchy_key: "academic".to_string(),
        },
        OrganizationNode {
            id: cycle_b_id,
            parent_id: Some(cycle_a_id),
            name: "Cycle B".to_string(),
            code: "CYC-B".to_string(),
            org_type: "Department".to_string(),
            hierarchy_key: "academic".to_string(),
        },
    ];

    let cycle_parent_map: std::collections::HashMap<Uuid, Option<Uuid>> =
        cycle_orgs.iter().map(|o| (o.id, o.parent_id)).collect();
    let cycle_tree = scaffoldry_core::tree::OrgTree::from_parent_map("academic", &cycle_parent_map);

    assert!(!unit_in_reach(
        &chair_caller,
        cycle_a_id,
        &cycle_tree,
        &roles
    ));
    assert!(!unit_in_reach(
        &chair_caller,
        cycle_b_id,
        &cycle_tree,
        &roles
    ));
}
