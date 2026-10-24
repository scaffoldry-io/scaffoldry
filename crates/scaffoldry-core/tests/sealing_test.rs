//! Access model phase 7: sealing, and the trace that explains a reach.
//!
//! A sealed container stops every grant anchored above it. A grant anchored at the sealed container, or inside it, still
//! reaches. `trace` is what `reaches` is built on: it says which grants reached an object, along what path, and for each
//! that did not, the first axis that failed.

use scaffoldry_core::coordinates::{Coordinates, Places};
use scaffoldry_core::reach::{holder_set, reaches, role_at, row_contributions, trace, AnchorKind, Grant, HolderKind, Miss, Object, Role, RowReach};
use scaffoldry_core::reach_reference::{reference_reaches, ReferenceDomain, ReferenceUnit};
use scaffoldry_core::tree::OrgTree;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use uuid::Uuid;

const NOW: i64 = 1_800_000_000;
const DAY: i64 = 86_400;

struct Domains;
impl Places for Domains {
    fn unit_within(&self, _: &str, _: &str, _: &str) -> bool {
        false
    }
    fn domain_within(&self, domain: &str, container: &str) -> bool {
        domain == container || container == "institution"
    }
    fn hierarchies(&self) -> Vec<String> {
        vec!["academic".to_string()]
    }
    fn root_unit(&self, _: &str) -> Option<String> {
        None
    }
}

/// root > science (the college) > physics (the department) > lab.
struct World {
    tree: OrgTree,
    root: Uuid,
    science: Uuid,
    physics: Uuid,
    lab: Uuid,
}

fn world() -> World {
    let ids: Vec<Uuid> = (0..4).map(|_| Uuid::new_v4()).collect();
    let [root, science, physics, lab] = ids[..] else { unreachable!() };
    let parents: HashMap<Uuid, Option<Uuid>> = HashMap::from([(root, None), (science, Some(root)), (physics, Some(science)), (lab, Some(physics))]);
    World { tree: OrgTree::from_parent_map("academic", &parents), root, science, physics, lab }
}

fn grant(id: i64, who: &str, role: Role, anchor: (AnchorKind, Option<String>)) -> Grant {
    Grant {
        id,
        holder_kind: HolderKind::Person,
        holder_id: who.to_string(),
        role,
        hierarchy: if anchor.0 == AnchorKind::Container { None } else { Some("academic".to_string()) },
        anchor_kind: anchor.0,
        anchor_id: anchor.1,
        depth: None,
        domain: None,
        valid_from: NOW - DAY,
        valid_until: None,
        revoked: false,
        clearance: None,
    }
}

fn container(id: &str) -> (AnchorKind, Option<String>) {
    (AnchorKind::Container, Some(id.to_string()))
}

/// A table `grants` of app `lab-app` in workspace `ws`, placed in the physics department. `sealed` names the containers
/// that are sealed.
fn table(w: &World, sealed: &[&str]) -> Object {
    Object {
        containers: vec!["workspace:ws".to_string(), "app:lab-app".to_string(), "table:lab-app/grants".to_string()],
        units: BTreeMap::from([("academic".to_string(), w.physics)]),
        domain: "institution".to_string(),
        sealed: sealed.iter().map(|s| s.to_string()).collect(),
    }
}

fn ada() -> scaffoldry_core::reach::HolderSet {
    holder_set("ada@x.edu", &[], &world().tree, [], NOW)
}

// 1. A sealed table hides itself from the grants above it. A grant at the table, or inside it, still reaches.
#[test]
fn a_sealed_table_stops_the_grants_above_it_and_keeps_the_ones_at_it() {
    let w = world();
    let holders = ada();
    let dean = grant(1, "ada@x.edu", Role::Admin, (AnchorKind::Unit, Some(w.science.to_string())));
    let editor = grant(2, "ada@x.edu", Role::Editor, container("workspace:ws"));
    let app_owner = grant(3, "ada@x.edu", Role::Owner, container("app:lab-app"));
    let at_table = grant(4, "ada@x.edu", Role::Viewer, container("table:lab-app/grants"));
    let open = table(&w, &[]);
    let sealed = table(&w, &["table:lab-app/grants"]);
    for g in [&dean, &editor, &app_owner, &at_table] {
        assert!(reaches(g, &holders, &open, &w.tree, &Domains, NOW), "grant {} reaches an unsealed table", g.id);
    }
    assert!(!reaches(&dean, &holders, &sealed, &w.tree, &Domains, NOW), "a college grant (unit anchor) is above the table");
    assert!(!reaches(&editor, &holders, &sealed, &w.tree, &Domains, NOW), "so is a workspace grant");
    assert!(!reaches(&app_owner, &holders, &sealed, &w.tree, &Domains, NOW), "and an app grant");
    assert!(reaches(&at_table, &holders, &sealed, &w.tree, &Domains, NOW), "a grant anchored at the sealed table reaches it");
    let all = [dean, editor, app_owner, at_table];
    assert_eq!(role_at(&holders, &all, &sealed, &w.tree, &Domains, NOW), Some((Role::Viewer, vec![4])), "only the grant at the table gives a role");

    // Sealing the app instead: the workspace grant is above it, the app grant is at it, and both reach the table inside.
    let app_sealed = table(&w, &["app:lab-app"]);
    assert!(!reaches(&all[1], &holders, &app_sealed, &w.tree, &Domains, NOW), "a workspace grant is above a sealed app");
    assert!(reaches(&all[2], &holders, &app_sealed, &w.tree, &Domains, NOW), "an app grant is at the sealed app");
    assert!(reaches(&all[3], &holders, &app_sealed, &w.tree, &Domains, NOW), "a table grant is inside it");
    assert!(!reaches(&all[0], &holders, &app_sealed, &w.tree, &Domains, NOW), "a unit grant is above any sealed container");
    // A sealed workspace stops only what is above a workspace: unit grants.
    let ws_sealed = table(&w, &["workspace:ws"]);
    assert!(reaches(&all[1], &holders, &ws_sealed, &w.tree, &Domains, NOW));
    assert!(!reaches(&all[0], &holders, &ws_sealed, &w.tree, &Domains, NOW));
}

// The rows of a sealed table follow the same rule.
#[test]
fn the_rows_of_a_sealed_table_are_reached_only_by_a_grant_at_or_inside_it() {
    let w = world();
    let holders = ada();
    let dean = grant(1, "ada@x.edu", Role::Admin, (AnchorKind::Unit, Some(w.science.to_string())));
    let editor = grant(2, "ada@x.edu", Role::Editor, container("workspace:ws"));
    let at_table = grant(4, "ada@x.edu", Role::Viewer, container("table:lab-app/grants"));
    let grants = [dean, editor, at_table];
    let rows = BTreeSet::new();
    let open: Vec<i64> = row_contributions(&holders, &grants, &table(&w, &[]), &rows, &w.tree, &Domains, NOW).into_iter().map(|c| c.grant_id).collect();
    assert_eq!(open, vec![1, 2, 4]);
    let sealed = row_contributions(&holders, &grants, &table(&w, &["table:lab-app/grants"]), &rows, &w.tree, &Domains, NOW);
    assert_eq!(sealed.iter().map(|c| c.grant_id).collect::<Vec<_>>(), vec![4], "only the grant at the table gives any row");
    assert_eq!(sealed[0].reach, RowReach::All);
}

// The reference evaluator, written apart from `reaches`, agrees on every combination.
#[test]
fn the_reference_evaluator_agrees_with_reaches_on_sealed_objects() {
    let w = world();
    let holders = ada();
    let units: Vec<ReferenceUnit> = [(w.root, None), (w.science, Some(w.root)), (w.physics, Some(w.science)), (w.lab, Some(w.physics))]
        .into_iter()
        .map(|(id, parent_id)| ReferenceUnit { id, parent_id, hierarchy: "academic".to_string() })
        .collect();
    let domains = vec![ReferenceDomain { key: "institution".to_string(), parent_key: None }];
    let anchors = [
        (AnchorKind::Unit, Some(w.science.to_string())),
        (AnchorKind::Unit, Some(w.physics.to_string())),
        container("workspace:ws"),
        container("app:lab-app"),
        container("table:lab-app/grants"),
    ];
    let seals: [&[&str]; 6] = [&[], &["workspace:ws"], &["app:lab-app"], &["table:lab-app/grants"], &["workspace:ws", "table:lab-app/grants"], &["app:lab-app", "table:lab-app/grants"]];
    for (i, anchor) in anchors.iter().enumerate() {
        let g = grant(i as i64 + 1, "ada@x.edu", Role::Editor, anchor.clone());
        for s in seals {
            let object = table(&w, s);
            assert_eq!(
                reaches(&g, &holders, &object, &w.tree, &Domains, NOW),
                reference_reaches(&g, &holders, &object, &units, &domains, NOW),
                "anchor {anchor:?} with seals {s:?}"
            );
        }
    }
}

// 3. The trace says which axis failed first, and the path of a grant that reached.
#[test]
fn the_trace_names_the_first_axis_that_failed_and_the_path_of_a_grant_that_reached() {
    let w = world();
    let holders = ada();
    let object = table(&w, &["table:lab-app/grants"]);
    let t = |g: &Grant, o: &Object| trace(g, &holders, o, &w.tree, &Domains, NOW);

    let dean = grant(1, "ada@x.edu", Role::Admin, (AnchorKind::Unit, Some(w.science.to_string())));
    assert_eq!(t(&dean, &table(&w, &[])).missed, None);
    assert_eq!(t(&dean, &table(&w, &[])).path, vec![w.science.to_string(), w.physics.to_string()], "from the anchor down to the object's unit");
    assert_eq!(t(&dean, &object).missed, Some(Miss::Sealed("table:lab-app/grants".to_string())), "stopped by the sealed table");

    let at_table = grant(2, "ada@x.edu", Role::Viewer, container("table:lab-app/grants"));
    assert_eq!(t(&at_table, &object).missed, None);
    assert_eq!(t(&at_table, &object).path, vec!["table:lab-app/grants".to_string()]);
    let workspace = grant(3, "ada@x.edu", Role::Editor, container("workspace:ws"));
    assert_eq!(t(&workspace, &table(&w, &[])).path, vec!["workspace:ws".to_string(), "app:lab-app".to_string(), "table:lab-app/grants".to_string()], "the containers from the anchor down");

    // The other axes, in order: the grant's life, its holder, its domain, its anchor, its depth.
    assert_eq!(t(&Grant { revoked: true, ..dean.clone() }, &object).missed, Some(Miss::Revoked));
    assert_eq!(t(&Grant { valid_from: NOW + DAY, ..dean.clone() }, &object).missed, Some(Miss::NotYet));
    assert_eq!(t(&Grant { valid_until: Some(NOW - 1), ..dean.clone() }, &object).missed, Some(Miss::Expired));
    assert_eq!(t(&Grant { holder_id: "someone.else@x.edu".to_string(), ..dean.clone() }, &object).missed, Some(Miss::Holder));
    assert_eq!(t(&Grant { domain: Some("finance".to_string()), ..dean.clone() }, &object).missed, Some(Miss::Domain));
    let elsewhere = grant(4, "ada@x.edu", Role::Viewer, (AnchorKind::Unit, Some(Uuid::new_v4().to_string())));
    assert_eq!(t(&elsewhere, &table(&w, &[])).missed, Some(Miss::Anchor));
    assert_eq!(t(&grant(5, "ada@x.edu", Role::Viewer, container("workspace:other")), &table(&w, &[])).missed, Some(Miss::Anchor));
    let shallow = Grant { depth: Some(0), ..dean.clone() };
    assert_eq!(t(&shallow, &table(&w, &[])).missed, Some(Miss::Depth), "inside the anchor's subtree, but below the depth");
    assert_eq!(t(&dean, &table(&w, &[])).missed.is_none(), reaches(&dean, &holders, &table(&w, &[]), &w.tree, &Domains, NOW), "reaches is the trace with no miss");
    assert!(Miss::Sealed("x".into()).axis() == "sealed" && Miss::Depth.axis() == "depth" && Miss::Holder.axis() == "holder");
}

// The sealed axis is stored with the other axes of a workspace.
#[test]
fn a_workspace_stores_sealed_as_one_more_axis() {
    let sealed = Coordinates { sealed: Some(true), ..Coordinates::default() };
    assert!(!sealed.is_empty(), "a workspace that only says it is sealed still has coordinates");
    assert_eq!(sealed.to_axes(), vec![("sealed".to_string(), "true".to_string())]);
    assert_eq!(Coordinates::from_axes(sealed.to_axes()), sealed, "it comes back as it went in");
    let open = Coordinates { sealed: Some(false), ..Coordinates::default() };
    assert_eq!(Coordinates::from_axes(open.to_axes()).sealed, Some(false));
    assert_eq!(Coordinates::default().to_axes(), vec![], "nothing named, no row");
    assert_eq!(Coordinates::from_axes([("sealed".to_string(), "banana".to_string())]).sealed, Some(false), "anything but true is not sealed");
}
