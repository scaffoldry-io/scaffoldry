use uuid::Uuid;
use scaffoldry_server::service::organizations::{unit_in_scope, OrgCaller};
use scaffoldry_server::state::{OrganizationNode, RoleRow};

#[test]
fn test_organization_scope_rules() {
    let root_id = Uuid::new_v4();
    let college_id = Uuid::new_v4();
    let dept_physics_id = Uuid::new_v4();
    let dept_biology_id = Uuid::new_v4();

    let orgs = vec![
        OrganizationNode {
            id: root_id,
            parent_id: None,
            name: "State University".to_string(),
            code: "INST".to_string(),
            org_type: "Institution".to_string(),
        },
        OrganizationNode {
            id: college_id,
            parent_id: Some(root_id),
            name: "College of Sciences".to_string(),
            code: "SCI".to_string(),
            org_type: "College".to_string(),
        },
        OrganizationNode {
            id: dept_physics_id,
            parent_id: Some(college_id),
            name: "Department of Physics".to_string(),
            code: "PHYS".to_string(),
            org_type: "Department".to_string(),
        },
        OrganizationNode {
            id: dept_biology_id,
            parent_id: Some(college_id),
            name: "Department of Biology".to_string(),
            code: "BIO".to_string(),
            org_type: "Department".to_string(),
        },
    ];

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

    // 1. College admin is in scope for a child department
    assert!(unit_in_scope(&dean_caller, dept_physics_id, &orgs, &roles));
    assert!(unit_in_scope(&dean_caller, dept_biology_id, &orgs, &roles));
    assert!(unit_in_scope(&dean_caller, college_id, &orgs, &roles));
    assert!(!unit_in_scope(&dean_caller, root_id, &orgs, &roles));

    // 2. Department admin is not in scope for a sibling department or for the college parent
    assert!(unit_in_scope(&chair_caller, dept_physics_id, &orgs, &roles));
    assert!(!unit_in_scope(&chair_caller, dept_biology_id, &orgs, &roles));
    assert!(!unit_in_scope(&chair_caller, college_id, &orgs, &roles));
    assert!(!unit_in_scope(&chair_caller, root_id, &orgs, &roles));

    // 3. central_admin is in scope for every node
    assert!(unit_in_scope(&central_admin_caller, root_id, &orgs, &roles));
    assert!(unit_in_scope(&central_admin_caller, college_id, &orgs, &roles));
    assert!(unit_in_scope(&central_admin_caller, dept_physics_id, &orgs, &roles));
    assert!(unit_in_scope(&central_admin_caller, dept_biology_id, &orgs, &roles));

    // 4. A cycle in the fixture (A.parent = B, B.parent = A) returns false and does not loop. Cap the walk at 32 steps.
    let cycle_a_id = Uuid::new_v4();
    let cycle_b_id = Uuid::new_v4();
    let cycle_orgs = vec![
        OrganizationNode {
            id: cycle_a_id,
            parent_id: Some(cycle_b_id),
            name: "Cycle A".to_string(),
            code: "CYC-A".to_string(),
            org_type: "Department".to_string(),
        },
        OrganizationNode {
            id: cycle_b_id,
            parent_id: Some(cycle_a_id),
            name: "Cycle B".to_string(),
            code: "CYC-B".to_string(),
            org_type: "Department".to_string(),
        },
    ];

    assert!(!unit_in_scope(&chair_caller, cycle_a_id, &cycle_orgs, &roles));
    assert!(!unit_in_scope(&chair_caller, cycle_b_id, &cycle_orgs, &roles));
}
