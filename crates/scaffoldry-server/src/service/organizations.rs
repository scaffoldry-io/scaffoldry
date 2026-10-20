use crate::state::{OrganizationNode, RoleRow, SharedState};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgCaller {
    pub eppn: String,
    pub affiliation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HierarchyTreeNode {
    pub id: Uuid,
    pub name: String,
    pub code: String,
    pub org_type: String,
    pub parent_id: Option<Uuid>,
    pub children: Vec<HierarchyTreeNode>,
}

pub fn is_platform_admin(caller: &OrgCaller, orgs: &[OrganizationNode], roles: &[RoleRow]) -> bool {
    // 1. central_admin is platform admin
    if caller.affiliation == "central_admin" {
        return true;
    }

    // 2. platform_admin role on the root unit (parent_id is None)
    roles.iter().any(|r| {
        r.eppn == caller.eppn
            && r.scoped_affiliation == "platform_admin"
            && orgs
                .iter()
                .any(|o| o.id == r.organization_id && o.parent_id.is_none())
    })
}

pub fn unit_in_reach(
    caller: &OrgCaller,
    org_id: Uuid,
    tree: &scaffoldry_core::tree::OrgTree,
    roles: &[RoleRow],
) -> bool {
    // 1. Platform admin is in reach everywhere
    if caller.affiliation == "central_admin"
        || roles.iter().any(|r| {
            r.eppn == caller.eppn
                && r.scoped_affiliation == "platform_admin"
                && tree.parent_of(r.organization_id).is_none()
        })
    {
        return true;
    }

    // 2. Caller has unit_admin role on org_id or an ancestor of org_id
    roles.iter().any(|r| {
        r.eppn == caller.eppn
            && r.scoped_affiliation == "unit_admin"
            && tree.contains(r.organization_id, org_id)
    })
}

pub fn build_hierarchy_tree(state: &SharedState, hierarchy_key: &str) -> Option<HierarchyTreeNode> {
    let orgs = state
        .organizations
        .read()
        .unwrap_or_else(|p| p.into_inner());
    let relevant: Vec<&OrganizationNode> = orgs
        .values()
        .filter(|o| o.hierarchy_key == hierarchy_key)
        .collect();
    if relevant.is_empty() {
        return None;
    }

    let root = *relevant.iter().find(|o| o.parent_id.is_none())?;

    fn build_node(node: &OrganizationNode, all: &[&OrganizationNode]) -> HierarchyTreeNode {
        let mut children: Vec<HierarchyTreeNode> = all
            .iter()
            .filter(|o| o.parent_id == Some(node.id))
            .map(|o| build_node(o, all))
            .collect();
        children.sort_by(|a, b| a.name.cmp(&b.name));
        HierarchyTreeNode {
            id: node.id,
            name: node.name.clone(),
            code: node.code.clone(),
            org_type: node.org_type.clone(),
            parent_id: node.parent_id,
            children,
        }
    }

    Some(build_node(root, &relevant))
}
