//! Access model phase 6: what one decision costs on a large institution.
//!
//! A synthetic institution of 50,000 people, 400 units and three hierarchies. For one person and one object,
//! `role_at` plus the Cedar decision takes under 1 ms at the median, and the slowest of 1,000 takes under 10 ms.
//!
//! Marked `#[ignore]` because the limits are for an optimized build. CI runs it in release mode:
//! `cargo test --release -p scaffoldry-server --test decide_benchmark_test -- --ignored --nocapture`.
//! If a limit fails and the work is sound, the limit does not move. The timings go into the record and the work stops.

mod common;

use scaffoldry_core::coordinates::Places;
use scaffoldry_core::reach::{holder_set, role_at, Grant, HolderKind, Holding};
use scaffoldry_core::reach_reference::{ReferenceDomain, ReferencePlaces, ReferenceUnit};
use scaffoldry_core::tree::{OrgClosureRow, OrgTree};
use scaffoldry_policy::entities::{PrincipalCtx, Resource, WorkspaceCtx};
use scaffoldry_server::service::access::decide_for;
use scaffoldry_server::service::synthesize::{generate_world, SynthesizeOptions};
use scaffoldry_server::state::{OrganizationNode, ServerState};
use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

const MEDIAN_LIMIT: Duration = Duration::from_millis(1);
const SLOWEST_LIMIT: Duration = Duration::from_millis(10);
const DECISIONS: usize = 1_000;

fn closure_rows(units: &[OrganizationNode]) -> Vec<OrgClosureRow> {
    let parent: HashMap<uuid::Uuid, Option<uuid::Uuid>> = units.iter().map(|u| (u.id, u.parent_id)).collect();
    let mut rows = Vec::new();
    for u in units {
        let (mut curr, mut depth) = (Some(u.id), 0i16);
        while let Some(ancestor) = curr {
            rows.push(OrgClosureRow { hierarchy_key: u.hierarchy_key.clone(), ancestor, descendant: u.id, depth });
            curr = parent.get(&ancestor).copied().flatten();
            depth += 1;
        }
    }
    rows
}

/// The grants the server would fetch for this holder, by holder kind and id.
fn fetched(index: &HashMap<(&'static str, String), Vec<Grant>>, holders: &scaffoldry_core::reach::HolderSet) -> Vec<Grant> {
    let mut keys: BTreeSet<(&'static str, String)> = BTreeSet::new();
    keys.insert(("person", holders.person.clone()));
    for (key, unit) in &holders.positions {
        keys.insert(("position", format!("{key}@{unit}")));
        keys.insert(("position", format!("{key}@*")));
    }
    keys.extend(holders.units.iter().map(|u| ("unit", u.to_string())));
    keys.extend(holders.groups.iter().map(|g| ("group", g.clone())));
    keys.iter().filter_map(|k| index.get(k)).flatten().cloned().collect()
}

#[test]
#[ignore = "the limits are for an optimized build. CI runs it in release mode"]
fn decide_benchmark() {
    let _db = common::TestDb::new();
    let state = std::sync::Arc::new(ServerState::new().expect("state"));
    let opts = SynthesizeOptions { seed: 20261023, people: 50_000, units: 400, hierarchies: vec!["academic".into(), "supervisory".into(), "regional".into()], apps: 20, records: 200 };
    let world = generate_world(&opts);
    assert_eq!(world.people.len(), 50_000);
    let tree = OrgTree::new(closure_rows(&world.units));
    let ref_units: Vec<ReferenceUnit> = world.units.iter().map(|u| ReferenceUnit { id: u.id, parent_id: u.parent_id, hierarchy: u.hierarchy_key.clone() }).collect();
    let domains: Vec<ReferenceDomain> = world.domains.clone();
    let places = ReferencePlaces { units: &ref_units, domains: &domains, hierarchies: opts.hierarchies.clone() };
    let _: &dyn Places = &places;

    let mut index: HashMap<(&'static str, String), Vec<Grant>> = HashMap::new();
    for g in &world.grants {
        let id = if g.holder_kind == HolderKind::Person { g.holder_id.to_lowercase() } else { g.holder_id.clone() };
        index.entry((g.holder_kind.as_str(), id)).or_default().push(g.clone());
    }
    let mut holdings: HashMap<&str, Vec<Holding>> = HashMap::new();
    for r in &world.roles {
        holdings.entry(&r.eppn).or_default().push(Holding { unit: r.organization_id, position_key: r.position_key.clone(), effective: None, end: None });
    }
    let groups = world.groups_by_person();
    let now = chrono::Utc::now().timestamp();

    // A small generator of the same sequence every run.
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move |n: usize| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x % n as u64) as usize
    };

    let (mut times, mut reached, mut allowed, mut holder_times) = (Vec::with_capacity(DECISIONS), 0, 0, Vec::with_capacity(DECISIONS));
    for _ in 0..DECISIONS {
        let person = &world.people[next(world.people.len())];
        let ws = &world.workspaces[next(world.workspaces.len())];
        let object = &world.objects[world.workspaces.iter().position(|w| w.id == ws.id).expect("a workspace object")];
        let started = Instant::now();
        let holders = holder_set(&person.eppn, holdings.get(person.eppn.as_str()).map(Vec::as_slice).unwrap_or_default(), &tree, groups.get(&person.eppn).cloned().unwrap_or_default(), now);
        let grants = fetched(&index, &holders);
        holder_times.push(started.elapsed());

        let started = Instant::now();
        let role = role_at(&holders, &grants, object, &tree, &places, now);
        let principal = PrincipalCtx {
            eppn: person.eppn.clone(),
            name: person.eppn.clone(),
            scoped_affiliation: "faculty".to_string(),
            department: ws.department.clone(),
            unit_ids: holders.units.iter().map(|u| u.to_string()).collect(),
            is_platform_admin: false,
        };
        let resource = Resource::Workspace(WorkspaceCtx {
            workspace_id: ws.id.clone(),
            department: ws.department.clone(),
            visibility: ws.visibility.clone(),
            data_classification: ws.data_classification.clone(),
            member_role: role.as_ref().map(|(r, _)| r.as_str().to_string()).unwrap_or_default(),
            unit_id: ws.organization_id.map(|u| u.to_string()).unwrap_or_default(),
            unit_path_ids: ws.organization_id.map(|u| tree.ancestors(u).iter().map(|a| a.to_string()).collect()).unwrap_or_default(),
            is_member: role.is_some(),
            is_ferpa_sensitive: false,
            categories: Default::default(),
            domain_path: object_domain_path(&domains, &object.domain),
            sensitivity_rank: 2,
        });
        let decision = decide_for(&state, &principal, "access_workspace", &resource);
        times.push(started.elapsed());
        assert!(decision.error.is_none(), "{:?}", decision.error);
        reached += role.is_some() as usize;
        allowed += decision.allowed as usize;
    }

    let mut sorted = times.clone();
    sorted.sort();
    let median = sorted[DECISIONS / 2];
    let p99 = sorted[DECISIONS * 99 / 100];
    let slowest = *sorted.last().unwrap();
    holder_times.sort();
    println!("\ndecide_benchmark: 50,000 people, 400 units, 3 hierarchies, {} grants. {DECISIONS} decisions.", world.grants.len());
    println!("| Step | Median | p99 | Slowest | Limit |");
    println!("| --- | --- | --- | --- | --- |");
    println!("| role_at plus Cedar | {:?} | {:?} | {:?} | {:?} median, {:?} slowest |", median, p99, slowest, MEDIAN_LIMIT, SLOWEST_LIMIT);
    println!("| holder set and grant fetch (not in the limit) | {:?} | {:?} | {:?} | |", holder_times[DECISIONS / 2], holder_times[DECISIONS * 99 / 100], holder_times.last().unwrap());
    println!("decide_benchmark: {reached} of {DECISIONS} reached the workspace, {allowed} allowed by Cedar\n");
    assert!(reached > 50 && reached < DECISIONS - 50, "the sample holds both outcomes: {reached} reached");
    assert!(median < MEDIAN_LIMIT, "the median {median:?} passed {MEDIAN_LIMIT:?}. The limit does not move. Paste the timings and stop");
    assert!(slowest < SLOWEST_LIMIT, "the slowest {slowest:?} passed {SLOWEST_LIMIT:?}. The limit does not move. Paste the timings and stop");
}

/// The domain and every domain above it.
fn object_domain_path(domains: &[ReferenceDomain], domain: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut current = Some(domain.to_string());
    while let Some(d) = current {
        current = domains.iter().find(|x| x.key == d).and_then(|x| x.parent_key.clone());
        out.insert(d);
    }
    out
}
