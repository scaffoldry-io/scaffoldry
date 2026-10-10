use std::collections::HashSet;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::state::{OrganizationNode, RoleRow};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgCaller {
    pub eppn: String,
    pub affiliation: String,
}

pub fn is_platform_admin(
    caller: &OrgCaller,
    orgs: &[OrganizationNode],
    roles: &[RoleRow],
) -> bool {
    // 1. central_admin is platform admin
    if caller.affiliation == "central_admin" {
        return true;
    }

    // 2. platform_admin role on the root unit (parent_id is None)
    roles.iter().any(|r| {
        r.eppn == caller.eppn
            && r.scoped_affiliation == "platform_admin"
            && orgs.iter().any(|o| o.id == r.organization_id && o.parent_id.is_none())
    })
}

pub fn unit_in_scope(
    caller: &OrgCaller,
    org_id: Uuid,
    orgs: &[OrganizationNode],
    roles: &[RoleRow],
) -> bool {
    // 1. Platform admin is in scope everywhere
    if is_platform_admin(caller, orgs, roles) {
        return true;
    }

    // 2. Walk parent_id from org_id up to the root, checking for unit_admin role on any ancestor node.
    let mut current_id = org_id;
    let mut visited = HashSet::new();
    let mut steps = 0;

    while steps < 32 {
        steps += 1;

        if !visited.insert(current_id) {
            // Cycle detected! Stop and return false.
            return false;
        }

        // Check if caller has unit_admin role for current_id
        if roles.iter().any(|r| {
            r.eppn == caller.eppn
                && r.organization_id == current_id
                && r.scoped_affiliation == "unit_admin"
        }) {
            return true;
        }

        // Find current organization
        if let Some(node) = orgs.iter().find(|o| o.id == current_id) {
            if let Some(parent_id) = node.parent_id {
                current_id = parent_id;
            } else {
                // Reached root without finding a unit_admin role
                return false;
            }
        } else {
            // Node not found in orgs
            return false;
        }
    }

    false
}
