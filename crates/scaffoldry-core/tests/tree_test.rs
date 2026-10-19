use scaffoldry_core::tree::OrgTree;
use std::collections::HashMap;
use uuid::Uuid;

#[test]
fn test_tree_ancestors_descendants_contains() {
    let root = Uuid::new_v4();
    let college = Uuid::new_v4();
    let dept = Uuid::new_v4();
    let lab = Uuid::new_v4();

    // root <- college <- dept <- lab
    let mut parent_map = HashMap::new();
    parent_map.insert(root, None);
    parent_map.insert(college, Some(root));
    parent_map.insert(dept, Some(college));
    parent_map.insert(lab, Some(dept));

    let tree = OrgTree::from_parent_map("academic", &parent_map);

    // 1. Ancestors
    let lab_ancestors = tree.ancestors(lab);
    assert_eq!(lab_ancestors, vec![lab, dept, college, root]);

    let dept_ancestors = tree.ancestors(dept);
    assert_eq!(dept_ancestors, vec![dept, college, root]);

    let root_ancestors = tree.ancestors(root);
    assert_eq!(root_ancestors, vec![root]);

    // 2. Descendants (unbounded)
    let root_descendants = tree.descendants(root, None);
    assert!(root_descendants.contains(&root));
    assert!(root_descendants.contains(&college));
    assert!(root_descendants.contains(&dept));
    assert!(root_descendants.contains(&lab));
    assert_eq!(root_descendants.len(), 4);

    // 3. Descendants with depth bound
    let root_depth_1 = tree.descendants(root, Some(1));
    assert_eq!(root_depth_1, vec![root, college]);

    let root_depth_2 = tree.descendants(root, Some(2));
    assert_eq!(root_depth_2, vec![root, college, dept]);

    // 4. Contains
    assert!(tree.contains(root, lab));
    assert!(tree.contains(college, lab));
    assert!(tree.contains(dept, lab));
    assert!(tree.contains(lab, lab));
    assert!(!tree.contains(lab, root));
    assert!(!tree.contains(dept, college));

    // 5. Parent of
    assert_eq!(tree.parent_of(lab), Some(dept));
    assert_eq!(tree.parent_of(dept), Some(college));
    assert_eq!(tree.parent_of(college), Some(root));
    assert_eq!(tree.parent_of(root), None);

    // 6. Hierarchy
    assert_eq!(tree.hierarchy_of(lab), Some("academic"));
}

#[test]
fn test_tree_cycle_safety() {
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();

    // A -> B -> A cycle
    let mut parent_map = HashMap::new();
    parent_map.insert(a, Some(b));
    parent_map.insert(b, Some(a));

    let tree = OrgTree::from_parent_map("academic", &parent_map);
    // Cycle in bad data must terminate safely
    assert!(tree.ancestors(a).len() <= 2);
    assert!(tree.ancestors(b).len() <= 2);
}
