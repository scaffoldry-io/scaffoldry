//! Synthetic institution generator (access model phase 5).
//!
//! Builds a synthetic institution in its own subtree of each hierarchy,
//! from a fixed seed, with a small generator written in the code.
//! No external random-number crates are used.

use crate::repository::RepositoryError;
use crate::state::{OrganizationNode, RoleRow, WorkspaceRecord};
use scaffoldry_core::coordinates::ROOT_DOMAIN;
use scaffoldry_core::reach::{AnchorKind, Grant, HolderKind, Object, Role};
use scaffoldry_core::reach_reference::{ReferenceDomain, ReferenceUnit};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Options configuring the synthetic institution generator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesizeOptions {
    pub seed: u64,
    pub people: usize,
    pub units: usize,
    pub hierarchies: Vec<String>,
    pub apps: usize,
    pub records: usize,
}

impl Default for SynthesizeOptions {
    fn default() -> Self {
        Self {
            seed: 42,
            people: 2000,
            units: 200,
            hierarchies: vec!["academic".to_string(), "supervisory".to_string()],
            apps: 10,
            records: 100,
        }
    }
}

/// A lightweight, deterministic PRNG based on SplitMix64.
/// No external random-number crates are used.
#[derive(Debug, Clone)]
pub struct DeterministicPrng {
    state: u64,
}

impl DeterministicPrng {
    pub fn new(seed: u64) -> Self {
        let mut prng = Self { state: seed };
        // Warm up state
        prng.next_u64();
        prng
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    pub fn gen_range(&mut self, min: usize, max: usize) -> usize {
        if min >= max {
            return min;
        }
        min + (self.next_u64() as usize % (max - min))
    }

    pub fn gen_uuid(&mut self) -> Uuid {
        let mut bytes = [0u8; 16];
        let p1 = self.next_u64().to_le_bytes();
        let p2 = self.next_u64().to_le_bytes();
        bytes[..8].copy_from_slice(&p1);
        bytes[8..].copy_from_slice(&p2);
        // Set UUID version 4 and variant RFC 4122 deterministically
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Uuid::from_bytes(bytes)
    }

    /// Skewed index selector simulating a power-law / Pareto distribution (80/20 rule).
    pub fn gen_skewed_index(&mut self, len: usize) -> usize {
        if len <= 1 {
            return 0;
        }
        // With 75% probability, pick from the first 25% of items
        if self.gen_range(0, 100) < 75 {
            let top_slice = (len / 4).max(1);
            self.gen_range(0, top_slice)
        } else {
            self.gen_range(0, len)
        }
    }
}

/// One synthetic person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntheticPerson {
    pub id: Uuid,
    pub eppn: String,
    pub first_name: String,
    pub last_name: String,
    pub email: String,
    pub synthetic: bool,
}

/// One synthetic app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntheticApp {
    pub slug: String,
    pub workspace_id: String,
    pub name: String,
    pub domain: String,
    pub unit_id: Uuid,
}

/// One synthetic dataset record. `unit_id` is its unit in the first hierarchy. `units` holds its unit in every hierarchy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntheticRecord {
    pub id: Uuid,
    pub app_slug: String,
    pub table_name: String,
    pub unit_id: Uuid,
    pub units: BTreeMap<String, Uuid>,
    pub title: String,
}

/// One synthetic group and the addresses of its members.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntheticGroup {
    pub id: String,
    pub members: Vec<String>,
}

/// A complete synthetic institution world generated deterministically from a seed.
#[derive(Debug, Clone)]
pub struct SyntheticWorld {
    pub units: Vec<OrganizationNode>,
    pub people: Vec<SyntheticPerson>,
    pub roles: Vec<RoleRow>,
    pub workspaces: Vec<WorkspaceRecord>,
    pub apps: Vec<SyntheticApp>,
    pub records: Vec<SyntheticRecord>,
    pub grants: Vec<Grant>,
    pub groups: Vec<SyntheticGroup>,
    pub domains: Vec<ReferenceDomain>,
    pub objects: Vec<Object>,
}

impl SyntheticWorld {
    /// The group ids each person belongs to, by address.
    pub fn groups_by_person(&self) -> std::collections::HashMap<String, Vec<String>> {
        let mut out: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        for g in &self.groups {
            for m in &g.members {
                out.entry(m.clone()).or_default().push(g.id.clone());
            }
        }
        out
    }

    /// Converts units to `ReferenceUnit` for use with the reference evaluator.
    pub fn to_reference_units(&self) -> Vec<ReferenceUnit> {
        self.units
            .iter()
            .map(|u| ReferenceUnit {
                id: u.id,
                parent_id: u.parent_id,
                hierarchy: u.hierarchy_key.clone(),
            })
            .collect()
    }
}

const DEFAULT_ACADEMIC_ROOT: &str = "00000000-0000-0000-0000-000000000001";

/// Generates a synthetic institution world from the given options.
pub fn generate_world(opts: &SynthesizeOptions) -> SyntheticWorld {
    let mut prng = DeterministicPrng::new(opts.seed);
    let mut units = Vec::new();

    // The domains the platform seeds (migration 0058), so a persisted world behaves as the in-memory one does.
    let domains: Vec<ReferenceDomain> = [
        (ROOT_DOMAIN, None),
        ("learning-and-teaching", Some(ROOT_DOMAIN)),
        ("research", Some(ROOT_DOMAIN)),
        ("engagement", Some(ROOT_DOMAIN)),
        ("strategy-and-governance", Some(ROOT_DOMAIN)),
        ("enabling-capabilities", Some(ROOT_DOMAIN)),
        ("herm:1.2.3", Some("learning-and-teaching")),
        ("herm:2.2.3", Some("research")),
        ("herm:5.3.1", Some("enabling-capabilities")),
    ]
    .into_iter()
    .map(|(key, parent)| ReferenceDomain { key: key.to_string(), parent_key: parent.map(str::to_string) })
    .collect();

    let academic_root_id = Uuid::parse_str(DEFAULT_ACADEMIC_ROOT).unwrap();
    let num_hierarchies = opts.hierarchies.len().max(1);
    let units_per_hierarchy = (opts.units / num_hierarchies).max(1);

    // 1. Generate units for each hierarchy
    for h in &opts.hierarchies {
        let (root_parent, root_org_type) = if h == "academic" {
            (Some(academic_root_id), "College")
        } else {
            (None, "Institution")
        };

        let sub_root_id = prng.gen_uuid();
        let sub_root = OrganizationNode {
            id: sub_root_id,
            parent_id: root_parent,
            name: format!("Synthetic {} Division", h),
            code: format!("SYN-{}-ROOT", h.to_uppercase()),
            org_type: root_org_type.to_string(),
            hierarchy_key: h.clone(),
            synthetic: true,
        };
        units.push(sub_root);

        let mut hierarchy_unit_ids = vec![sub_root_id];
        for i in 1..units_per_hierarchy {
            let id = prng.gen_uuid();
            // Fanout: pick parent from existing units in this hierarchy, skewed toward earlier tiers
            let parent_idx = prng.gen_skewed_index(hierarchy_unit_ids.len());
            let parent_id = hierarchy_unit_ids[parent_idx];

            let org_type = match prng.gen_range(0, 3) {
                0 => "Department",
                1 => "Center",
                _ => "Program",
            };

            let node = OrganizationNode {
                id,
                parent_id: Some(parent_id),
                name: format!("Synthetic {} Unit {:04}", h, i),
                code: format!("SYN-{}-{:04}", h.to_uppercase(), i),
                org_type: org_type.to_string(),
                hierarchy_key: h.clone(),
                synthetic: true,
            };
            units.push(node);
            hierarchy_unit_ids.push(id);
        }
    }

    // 2. Generate people with skewed appointments and positions
    let mut people = Vec::with_capacity(opts.people);
    let mut roles = Vec::new();

    let all_unit_ids: Vec<Uuid> = units.iter().map(|u| u.id).collect();

    for i in 1..=opts.people {
        let person_id = prng.gen_uuid();
        let eppn = format!("synth_{i}@synthetic.invalid");
        let first_name = format!("Synth{i}");
        let last_name = "User".to_string();

        people.push(SyntheticPerson {
            id: person_id,
            eppn: eppn.clone(),
            first_name: first_name.clone(),
            last_name: last_name.clone(),
            email: eppn.clone(),
            synthetic: true,
        });

        // Determine number of appointments (skewed: most have 1, some have 2 or 3)
        let num_appointments = match prng.gen_range(0, 100) {
            0..=85 => 1,
            86..=96 => 2,
            _ => 3,
        };

        for _ in 0..num_appointments {
            let unit_idx = prng.gen_skewed_index(all_unit_ids.len());
            let unit_id = all_unit_ids[unit_idx];

            let (position_key, affiliation) = match prng.gen_range(0, 100) {
                0..=4 => (Some("chair".to_string()), "faculty"),
                5..=7 => (Some("dean".to_string()), "faculty"),
                8..=12 => (Some("director".to_string()), "staff"),
                13..=22 => (Some("supervisor".to_string()), "staff"),
                23..=60 => (None, "staff"),
                61..=85 => (None, "student"),
                _ => (None, "member"),
            };

            roles.push(RoleRow {
                id: prng.gen_uuid(),
                person_id,
                eppn: eppn.clone(),
                organization_id: unit_id,
                role_title: affiliation.to_string(),
                scoped_affiliation: affiliation.to_string(),
                is_primary: true,
                source: "scim".to_string(),
                position_key,
            });
        }
    }

    // Units of each hierarchy, so an object is placed only in a hierarchy that holds its unit.
    let mut hierarchy_units: BTreeMap<String, Vec<Uuid>> = BTreeMap::new();
    for u in &units {
        hierarchy_units.entry(u.hierarchy_key.clone()).or_default().push(u.id);
    }
    let first_hierarchy = opts.hierarchies.first().cloned().unwrap_or_else(|| "academic".to_string());

    // Groups: a few, each with a skewed membership.
    let group_count = (opts.people / 100).clamp(3, 40);
    let mut groups: Vec<SyntheticGroup> = (1..=group_count).map(|n| SyntheticGroup { id: format!("synth-group-{n}"), members: Vec::new() }).collect();
    for person in &people {
        let joins = match prng.gen_range(0, 100) {
            0..=69 => 0,
            70..=94 => 1,
            _ => 2,
        };
        for _ in 0..joins {
            let g = prng.gen_skewed_index(groups.len());
            if !groups[g].members.contains(&person.eppn) {
                groups[g].members.push(person.eppn.clone());
            }
        }
    }

    // 3. Generate workspaces and apps (skewed so a few departments hold most)
    let pick_unit = |prng: &mut DeterministicPrng, h: &str| -> Uuid {
        let list = &hierarchy_units[h];
        list[prng.gen_skewed_index(list.len())]
    };
    let num_workspaces = (opts.apps / 2).max(1);
    let mut workspaces = Vec::with_capacity(num_workspaces);
    let mut apps = Vec::with_capacity(opts.apps);

    for w in 1..=num_workspaces {
        let ws_id = format!("ws-synth-{w}");
        let domain = ["research", "learning-and-teaching", "enabling-capabilities", "herm:2.2.3"][prng.gen_range(0, 4)];
        let mut coords = scaffoldry_core::coordinates::Coordinates { domain: Some(domain.to_string()), ..Default::default() };
        for h in &opts.hierarchies {
            coords.units.insert(h.clone(), pick_unit(&mut prng, h).to_string());
        }
        let unit_id = Uuid::parse_str(&coords.units[&first_hierarchy]).unwrap();

        workspaces.push(WorkspaceRecord {
            id: ws_id.clone(),
            name: format!("Synthetic Workspace {w}"),
            code: format!("WS-SYNTH-{w}"),
            organization: "Synthetic Division".to_string(),
            department: "synthetic".to_string(),
            description: "Synthetic workspace".to_string(),
            icon: "🔬".to_string(),
            lead: "Synthetic Lead".to_string(),
            visibility: "departmental".to_string(),
            data_classification: "Internal".to_string(),
            created_at: "2026-10-20T00:00:00Z".to_string(),
            organization_id: Some(unit_id),
            coordinates: coords,
            synthetic: true,
        });
    }

    for a in 1..=opts.apps {
        let ws_idx = prng.gen_skewed_index(workspaces.len());
        let ws = &workspaces[ws_idx];
        apps.push(SyntheticApp {
            slug: format!("app-synth-{a}"),
            workspace_id: ws.id.clone(),
            name: format!("Synthetic App {a}"),
            domain: ws.coordinates.domain.clone().unwrap_or_else(|| ROOT_DOMAIN.to_string()),
            unit_id: ws.organization_id.unwrap_or(all_unit_ids[0]),
        });
    }

    // 4. Generate records, each placed at a unit in every hierarchy
    let mut records = Vec::with_capacity(opts.records);
    for r in 1..=opts.records {
        let app_idx = prng.gen_skewed_index(apps.len());
        let app = &apps[app_idx];
        let mut at = BTreeMap::new();
        for h in &opts.hierarchies {
            at.insert(h.clone(), pick_unit(&mut prng, h));
        }
        records.push(SyntheticRecord {
            id: prng.gen_uuid(),
            app_slug: app.slug.clone(),
            table_name: "records".to_string(),
            unit_id: at[&first_hierarchy],
            units: at,
            title: format!("Record {r}"),
        });
    }

    // 5. Generate grants of every anchor kind: Container, Unit, HolderUnit
    let mut grants = Vec::new();
    let mut grant_id = 1i64;
    // A moment in the far future: a grant that is valid now but ends later, or starts later.
    const FAR: i64 = 4_000_000_000;

    // A. HolderUnit grants
    for h in &opts.hierarchies {
        for (position, role, depth) in [("chair", Role::Editor, None), ("supervisor", Role::Viewer, Some(1))] {
            grants.push(Grant {
                id: grant_id,
                holder_kind: HolderKind::Position,
                holder_id: format!("{position}@*"),
                role,
                hierarchy: Some(h.clone()),
                anchor_kind: AnchorKind::HolderUnit,
                anchor_id: None,
                depth,
                domain: None,
                valid_from: 0,
                valid_until: None,
                revoked: false,
            });
            grant_id += 1;
        }
    }

    // B. Unit grants on synthetic units, then C. container grants on workspaces and apps
    for (i, unit) in units.iter().enumerate().take(10) {
        let person = &people[prng.gen_range(0, people.len())];
        grants.push(Grant {
            id: grant_id,
            holder_kind: HolderKind::Person,
            holder_id: person.eppn.clone(),
            role: if i % 2 == 0 { Role::Viewer } else { Role::Editor },
            hierarchy: Some(unit.hierarchy_key.clone()),
            anchor_kind: AnchorKind::Unit,
            anchor_id: Some(unit.id.to_string()),
            depth: if i % 3 == 0 { Some(1) } else { None },
            domain: None,
            valid_from: 0,
            valid_until: None,
            revoked: false,
        });
        grant_id += 1;
    }
    for (i, ws) in workspaces.iter().enumerate().take(5) {
        let person = &people[prng.gen_range(0, people.len())];
        grants.push(Grant {
            id: grant_id,
            holder_kind: HolderKind::Person,
            holder_id: person.eppn.clone(),
            role: if i == 0 { Role::Admin } else { Role::Editor },
            hierarchy: None,
            anchor_kind: AnchorKind::Container,
            anchor_id: Some(format!("workspace:{}", ws.id)),
            depth: None,
            domain: None,
            valid_from: 0,
            valid_until: None,
            revoked: false,
        });
        grant_id += 1;
    }

    // D. Random grants that exercise every axis: holder kind, depth, domain, and the validity window.
    let extra = (opts.people / 10).max(20);
    let domain_keys: Vec<String> = domains.iter().map(|d| d.key.clone()).collect();
    for _ in 0..extra {
        let role = match prng.gen_range(0, 100) {
            0..=44 => Role::Viewer,
            45..=64 => Role::Commenter,
            65..=84 => Role::Editor,
            85..=96 => Role::Admin,
            _ => Role::Owner,
        };
        let h = opts.hierarchies[prng.gen_range(0, opts.hierarchies.len())].clone();
        let anchor_unit = pick_unit(&mut prng, &h);
        let (holder_kind, holder_id) = match prng.gen_range(0, 100) {
            0..=49 => (HolderKind::Person, people[prng.gen_range(0, people.len())].eppn.clone()),
            50..=64 => (HolderKind::Unit, hierarchy_units[&h][prng.gen_skewed_index(hierarchy_units[&h].len())].to_string()),
            65..=79 => (HolderKind::Group, groups[prng.gen_range(0, groups.len())].id.clone()),
            _ => (HolderKind::Position, format!("{}@*", ["chair", "dean", "director", "supervisor"][prng.gen_range(0, 4)])),
        };
        let (anchor_kind, hierarchy, anchor_id) = if holder_kind == HolderKind::Position && prng.gen_range(0, 2) == 0 {
            (AnchorKind::HolderUnit, Some(h.clone()), None)
        } else if prng.gen_range(0, 100) < 25 {
            let ws = &workspaces[prng.gen_range(0, workspaces.len())];
            (AnchorKind::Container, None, Some(format!("workspace:{}", ws.id)))
        } else {
            (AnchorKind::Unit, Some(h.clone()), Some(anchor_unit.to_string()))
        };
        let depth = if anchor_kind == AnchorKind::Container || prng.gen_range(0, 2) == 0 { None } else { Some(prng.gen_range(0, 4) as i16) };
        let domain = (prng.gen_range(0, 100) < 35).then(|| domain_keys[prng.gen_range(0, domain_keys.len())].clone());
        let (valid_from, valid_until, revoked) = match prng.gen_range(0, 100) {
            0..=4 => (0, Some(500), false),
            5..=9 => (FAR, None, false),
            10..=14 => (0, None, true),
            15..=19 => (0, Some(FAR), false),
            _ => (0, None, false),
        };
        grants.push(Grant { id: grant_id, holder_kind, holder_id, role, hierarchy, anchor_kind, anchor_id, depth, domain, valid_from, valid_until, revoked });
        grant_id += 1;
    }

    // 6. Build evaluation objects: every workspace, app and record, each placed at a unit of each hierarchy
    let ws_units = |ws: &WorkspaceRecord| -> BTreeMap<String, Uuid> {
        ws.coordinates.units.iter().filter_map(|(h, u)| Uuid::parse_str(u).ok().map(|u| (h.clone(), u))).collect()
    };
    let mut objects = Vec::new();
    for ws in &workspaces {
        objects.push(Object {
            containers: vec![format!("workspace:{}", ws.id)],
            units: ws_units(ws),
            domain: ws.coordinates.domain.clone().unwrap_or_else(|| ROOT_DOMAIN.to_string()),
        });
    }
    for app in &apps {
        let ws = workspaces.iter().find(|w| w.id == app.workspace_id).expect("an app's workspace exists");
        objects.push(Object {
            containers: vec![format!("workspace:{}", app.workspace_id), format!("app:{}", app.slug)],
            units: ws_units(ws),
            domain: app.domain.clone(),
        });
    }
    for rec in &records {
        let app = apps.iter().find(|a| a.slug == rec.app_slug).expect("a record's app exists");
        objects.push(Object {
            containers: vec![format!("workspace:{}", app.workspace_id), format!("app:{}", app.slug)],
            units: rec.units.clone(),
            domain: app.domain.clone(),
        });
    }

    SyntheticWorld { units, people, roles, workspaces, apps, records, grants, groups, domains, objects }
}

/// A stable hex digest of every row the generator wrote, so two runs of one seed can be compared.
pub fn compute_world_hash(world: &SyntheticWorld) -> String {
    let mut hasher = Sha256::new();

    // 1. Units sorted by id
    let mut units = world.units.clone();
    units.sort_by_key(|a| a.id);
    for u in &units {
        hasher.update(u.id.as_bytes());
        hasher.update(u.name.as_bytes());
        hasher.update(u.code.as_bytes());
        hasher.update(u.hierarchy_key.as_bytes());
        if let Some(p) = u.parent_id {
            hasher.update(p.as_bytes());
        }
        hasher.update([u.synthetic as u8]);
    }

    // 2. People sorted by eppn
    let mut people = world.people.clone();
    people.sort_by(|a, b| a.eppn.cmp(&b.eppn));
    for p in &people {
        hasher.update(p.id.as_bytes());
        hasher.update(p.eppn.as_bytes());
        hasher.update([p.synthetic as u8]);
    }

    // 3. Roles sorted by id
    let mut roles = world.roles.clone();
    roles.sort_by_key(|a| a.id);
    for r in &roles {
        hasher.update(r.id.as_bytes());
        hasher.update(r.person_id.as_bytes());
        hasher.update(r.eppn.as_bytes());
        hasher.update(r.organization_id.as_bytes());
        hasher.update(r.role_title.as_bytes());
        if let Some(pos) = &r.position_key {
            hasher.update(pos.as_bytes());
        }
    }

    // 4. Workspaces sorted by id
    let mut ws = world.workspaces.clone();
    ws.sort_by(|a, b| a.id.cmp(&b.id));
    for w in &ws {
        hasher.update(w.id.as_bytes());
        hasher.update(w.name.as_bytes());
        hasher.update(w.code.as_bytes());
        hasher.update([w.synthetic as u8]);
    }

    // 5. Apps sorted by slug
    let mut apps = world.apps.clone();
    apps.sort_by(|a, b| a.slug.cmp(&b.slug));
    for a in &apps {
        hasher.update(a.slug.as_bytes());
        hasher.update(a.workspace_id.as_bytes());
        hasher.update(a.domain.as_bytes());
        hasher.update(a.unit_id.as_bytes());
    }

    // 6. Records sorted by id
    let mut records = world.records.clone();
    records.sort_by_key(|a| a.id);
    for r in &records {
        hasher.update(r.id.as_bytes());
        hasher.update(r.app_slug.as_bytes());
        hasher.update(r.unit_id.as_bytes());
        hasher.update(r.title.as_bytes());
    }

    // 7. Grants sorted by id
    let mut grants = world.grants.clone();
    grants.sort_by_key(|g| g.id);
    for g in &grants {
        hasher.update(g.id.to_le_bytes());
        hasher.update(g.holder_kind.as_str().as_bytes());
        hasher.update(g.holder_id.as_bytes());
        hasher.update(g.role.as_str().as_bytes());
        hasher.update(g.anchor_kind.as_str().as_bytes());
        if let Some(a) = &g.anchor_id {
            hasher.update(a.as_bytes());
        }
    }

    let hash = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for b in hash {
        use std::fmt::Write;
        let _ = write!(hex, "{:02x}", b);
    }
    hex
}

/// Persists the generated synthetic institution to the database.
pub fn persist_world(state: &crate::state::SharedState, world: &SyntheticWorld) -> Result<(), RepositoryError> {
    state.repository.with_client(|c| {
        c.execute(
            "INSERT INTO hierarchies (key, name, kind, source, created_by) \
             VALUES ('supervisory', 'Supervisory management', 'units', 'api', 'system') \
             ON CONFLICT (key) DO NOTHING",
            &[],
        )?;
        c.execute(
            "INSERT INTO position_types (key, name, description, org_types, max_holders, created_by) \
             VALUES \
                 ('chair', 'Department Chair', 'Head of an academic department', '[\"Department\"]'::jsonb, 1, 'system'), \
                 ('dean', 'College Dean', 'Dean of an academic college', '[\"College\"]'::jsonb, 1, 'system'), \
                 ('director', 'Program Director', 'Director of a center or program', '[\"Center\", \"Program\"]'::jsonb, 1, 'system'), \
                 ('supervisor', 'Supervisor', 'Supervisory manager', '[\"Department\", \"Institution\", \"College\"]'::jsonb, 10, 'system') \
             ON CONFLICT (key) DO NOTHING",
            &[],
        )?;
        Ok(())
    })?;

    // 1. Units
    for u in &world.units {
        state.organizations.write().unwrap().insert(u.id, u.clone());
        state.repository.upsert_organization(u)?;
    }

    // 2. People & SCIM Users
    for p in &world.people {
        let p = p.clone();
        state.repository.with_client(move |c| {
            c.execute(
                "INSERT INTO persons (id, first_name, last_name, email, eppn, synthetic) \
                 VALUES ($1, $2, $3, $4, $5, TRUE) \
                 ON CONFLICT (eppn) DO NOTHING",
                &[&p.id, &p.first_name, &p.last_name, &p.email, &p.eppn],
            )?;
            let payload = serde_json::json!({
                "id": p.id.to_string(),
                "userName": p.eppn,
                "name": { "givenName": p.first_name, "familyName": p.last_name },
                "emails": [{ "value": p.email, "primary": true }],
                "active": true,
            });
            c.execute(
                "INSERT INTO scim_users (id, user_name, active, payload, synthetic, updated_at) \
                 VALUES ($1, $2, TRUE, $3, TRUE, NOW()) \
                 ON CONFLICT (id) DO NOTHING",
                &[&p.id.to_string(), &p.eppn, &payload],
            )?;
            Ok(())
        })?;
    }

    // 3. Roles
    for r in &world.roles {
        state.repository.upsert_role(r)?;
    }

    // 4. Workspaces
    for ws in &world.workspaces {
        state.workspaces.write().unwrap().insert(ws.id.clone(), ws.clone());
        state.repository.upsert_workspace(ws)?;
    }

    // 5. Groups. A member is named by the person's id, as a SCIM group names it.
    let ids: BTreeMap<&str, String> = world.people.iter().map(|p| (p.eppn.as_str(), p.id.to_string())).collect();
    for g in &world.groups {
        let members: Vec<serde_json::Value> = g.members.iter().filter_map(|m| ids.get(m.as_str())).map(|id| serde_json::json!({ "value": id })).collect();
        state.repository.upsert_scim_group(&crate::state::ScimGroup { id: g.id.clone(), display_name: g.id.clone(), members })?;
    }

    // 6. Apps: one table whose rows are placed by a `Unit` field in the first hierarchy.
    let first = world.units.first().map(|u| u.hierarchy_key.clone()).unwrap_or_else(|| "academic".to_string());
    for app in &world.apps {
        let manifest: scaffoldry_engine::AppManifest = serde_json::from_value(serde_json::json!({
            "slug": app.slug, "title": app.name, "description": "Synthetic app", "organization_code": "SYNTH", "department": "synthetic",
            "workspace_id": app.workspace_id, "views": [], "pages": [],
            "tables": [{
                "id": "records", "name": "Records", "slug": "records",
                "fields": [
                    { "name": "title", "label": "Title", "field_type": "Text", "required": false, "ferpa_sensitive": false },
                    { "name": "unit", "label": "Unit", "field_type": "Unit", "hierarchy": first, "required": false, "ferpa_sensitive": false, "indexed": true }
                ],
                "row_place": { first.clone(): "unit" }
            }]
        }))
        .map_err(RepositoryError::Json)?;
        state.persist_app_manifest(manifest).map_err(|e| RepositoryError::Conflict(format!("the synthetic app could not be stored: {e}")))?;
    }

    // 7. Records, a thousand to a transaction, with their index rows.
    for app in &world.apps {
        let mine: Vec<&SyntheticRecord> = world.records.iter().filter(|r| r.app_slug == app.slug).collect();
        if mine.is_empty() {
            continue;
        }
        let probe = serde_json::json!({ "_table_id": "records" });
        let ctx = crate::service::history::ctx_for("synthetic@synthetic.invalid", crate::repository::ActorKind::System, &app.slug, &probe, state);
        for chunk in mine.chunks(1000) {
            let rows: Vec<crate::state::DatasetRecord> = chunk
                .iter()
                .map(|r| crate::state::DatasetRecord {
                    id: r.id.to_string(),
                    app_slug: r.app_slug.clone(),
                    data: serde_json::json!({ "_table_id": "records", "title": r.title, "unit": r.unit_id.to_string() }),
                    ceds_mapping: serde_json::json!({}),
                    is_ferpa_sensitive: false,
                    created_at: String::new(),
                    created_by: Some("synthetic@synthetic.invalid".to_string()),
                    version: 1,
                })
                .collect();
            let ctx = ctx.clone();
            state.repository.with_client(move |c| {
                let mut tx = c.transaction()?;
                for rec in &rows {
                    // A record is written once: a second run finds it by id and moves on.
                    if tx.query_opt("SELECT 1 FROM dataset_records WHERE id = $1", &[&rec.id])?.is_some() {
                        continue;
                    }
                    crate::repository::insert_record_in_tx(&mut tx, rec, &ctx)?;
                }
                tx.commit()?;
                Ok(())
            })?;
        }
    }

    // 8. Grants. The database assigns the ids, so a real grant made later never collides. A grant is written once: its
    // reason names the seed and the generator's id, and a second run finds it there.
    let grants = world.grants.clone();
    let seed_tag = world_tag(world);
    state.repository.with_client(move |c| {
        for g in &grants {
            let reason = format!("synthetic grant {seed_tag}:{}", g.id);
            c.execute(
                "INSERT INTO access_grants (holder_kind, holder_id, role, hierarchy_key, anchor_kind, anchor_id, depth, domain, \
                                           valid_from, valid_until, revoked_at, revoked_by, source, reason, created_by) \
                 SELECT $1, $2, $3, $4, $5, $6, $7, $8, to_timestamp($9::double precision), \
                        CASE WHEN $10::double precision IS NULL THEN NULL ELSE to_timestamp($10::double precision) END, \
                        CASE WHEN $11 THEN NOW() END, CASE WHEN $11 THEN 'system' END, 'rule', $12, 'system' \
                 WHERE NOT EXISTS (SELECT 1 FROM access_grants WHERE reason = $12)",
                &[
                    &g.holder_kind.as_str(),
                    &g.holder_id,
                    &g.role.as_str(),
                    &g.hierarchy,
                    &g.anchor_kind.as_str(),
                    &g.anchor_id,
                    &g.depth,
                    &g.domain,
                    &(g.valid_from as f64),
                    &g.valid_until.map(|u| u as f64),
                    &g.revoked,
                    &reason,
                ],
            )?;
        }
        Ok(())
    })?;

    Ok(())
}

/// A short tag for a world, from its first unit and grant count, that is the same for the same seed.
fn world_tag(world: &SyntheticWorld) -> String {
    let first = world.units.first().map(|u| u.id.simple().to_string()).unwrap_or_default();
    format!("{}-{}", &first[..first.len().min(8)], world.grants.len())
}

/// Executes synthesize generation and persistence, returning the world hash.
pub fn synthesize(state: &crate::state::SharedState, opts: &SynthesizeOptions) -> Result<String, RepositoryError> {
    let world = generate_world(opts);
    let hash = compute_world_hash(&world);
    persist_world(state, &world)?;
    Ok(hash)
}
