//! Decision authority phase 4: a large institution, simulated.
//!
//! Pure. No database, no clock, no random number crate. It calls `required` and `resolve_approvers` and nothing else.
//! The institution comes from a fixed seed and a small linear congruential generator written here.
//!
//! The report is `docs/measurements/authority-simulation.md`. Run with `SCAFFOLDRY_WRITE_REPORT=1` to write it. Without
//! that, the test reads the committed file and fails when its numbers are not the ones this code produces.

use scaffoldry_core::approver::{resolve_approvers, ApproverSpec, Delegation, Holding, NoApprover, Origin, Person, ResolveCtx, Unit};
use scaffoldry_core::authority::{load_summary, required, MatrixRow, Path, SignalRow, Tier, FLAGGED};
use std::collections::BTreeMap;
use std::time::Instant;
use uuid::Uuid;

const SEED: u64 = 20_261_027;
const PEOPLE: usize = 5_000;
const COLLEGES: usize = 9;
const DEPARTMENTS: usize = 40;
const PROGRAMS: usize = 150;
const UNITS: usize = 1 + COLLEGES + DEPARTMENTS + PROGRAMS;
const WORKSPACES: usize = 400;
const PROPOSALS: usize = 10_000;
const WEEKS: usize = 13;
const WEEK: i64 = 7 * 24 * 3600;
const ADMINS: usize = 8;
const REPORT: &str = "../../docs/measurements/authority-simulation.md";

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn eppn(n: usize) -> String {
    format!("p{n}@sim.edu")
}

fn unit_uuid(i: usize) -> Uuid {
    Uuid::from_u128(i as u128 + 1)
}

struct Workspace {
    unit: usize,
    owners: Vec<usize>,
    members: Vec<usize>,
}

struct Proposal {
    workspace: usize,
    proposer: usize,
    week: usize,
    raised: Vec<String>,
}

struct Institution {
    units: Vec<Unit>,
    people: Vec<Person>,
    /// Primary placements for everyone, then one row for each position held.
    holdings: Vec<Holding>,
    /// Who holds each position, for the report: eppn to a label.
    labels: BTreeMap<String, String>,
    holders: Vec<usize>,
    workspaces: Vec<Workspace>,
    proposals: Vec<Proposal>,
    first_member: usize,
}

fn parent_of(i: usize) -> Option<usize> {
    match i {
        0 => None,
        i if i <= COLLEGES => Some(0),
        i if i <= COLLEGES + DEPARTMENTS => Some((i - 1 - COLLEGES) % COLLEGES + 1),
        i => Some((i - 1 - COLLEGES - DEPARTMENTS) % DEPARTMENTS + 1 + COLLEGES),
    }
}

fn unit_path(i: usize) -> Vec<Uuid> {
    let mut path = vec![unit_uuid(i)];
    let mut at = i;
    while let Some(p) = parent_of(at) {
        path.push(unit_uuid(p));
        at = p;
    }
    path
}

fn build() -> Institution {
    let mut rng = Lcg(SEED);
    let level = |i: usize| match i {
        0 => "Institution",
        i if i <= COLLEGES => "College",
        i if i <= COLLEGES + DEPARTMENTS => "Department",
        _ => "Program",
    };
    let units: Vec<Unit> = (0..UNITS).map(|i| Unit { id: format!("u{i}"), parent_id: parent_of(i).map(|p| format!("u{p}")), org_type: level(i).into(), name: format!("Unit {i}") }).collect();

    // Positions first: a chair for each department, a dean for each college, a privacy officer and a records officer at the root.
    let mut positions: Vec<(&str, usize)> = Vec::new();
    for d in 0..DEPARTMENTS {
        positions.push(("chair", 1 + COLLEGES + d));
    }
    for c in 0..COLLEGES {
        positions.push(("dean", 1 + c));
    }
    positions.push(("privacy_officer", 0));
    positions.push(("records_officer", 0));
    let admin_end = positions.len() + ADMINS;

    let people: Vec<Person> = (0..PEOPLE).map(|n| Person { eppn: eppn(n), display_name: format!("Person {n}"), active: true, on_hold: false }).collect();
    let mut holdings = Vec::new();
    for n in 0..PEOPLE {
        let unit = if n < positions.len() { positions[n].1 } else { 1 + COLLEGES + DEPARTMENTS + rng.below(PROGRAMS) };
        holdings.push(Holding { eppn: eppn(n), unit_id: format!("u{unit}"), position_key: None, is_primary: true });
    }
    let mut labels = BTreeMap::new();
    let mut holders = Vec::new();
    for (n, (key, unit)) in positions.iter().enumerate() {
        holdings.push(Holding { eppn: eppn(n), unit_id: format!("u{unit}"), position_key: Some((*key).into()), is_primary: false });
        labels.insert(eppn(n), format!("{} of unit {unit}", key.replace('_', " ")));
        holders.push(n);
    }

    // Workspaces in departments and programs. People who hold a position are not drawn as owners or members.
    let first_member = admin_end;
    let pick = |rng: &mut Lcg| first_member + rng.below(PEOPLE - first_member);
    let workspaces: Vec<Workspace> = (0..WORKSPACES)
        .map(|_| {
            let unit = 1 + COLLEGES + rng.below(DEPARTMENTS + PROGRAMS);
            let owners = (0..1 + rng.below(4)).map(|_| pick(&mut rng)).collect();
            let members = (0..5 + rng.below(16)).map(|_| pick(&mut rng)).collect();
            Workspace { unit, owners, members }
        })
        .collect();

    // 70% raise nothing, 20% one flagged check, 10% two or more. 30% of proposers are owners.
    let proposals: Vec<Proposal> = (0..PROPOSALS)
        .map(|_| {
            let workspace = rng.below(WORKSPACES);
            let w = &workspaces[workspace];
            let proposer = if rng.below(100) < 30 { w.owners[rng.below(w.owners.len())] } else { w.members[rng.below(w.members.len())] };
            let how_many = match rng.below(100) {
                0..=69 => 0,
                70..=89 => 1,
                _ => 2 + rng.below(2),
            };
            let mut raised: Vec<String> = Vec::new();
            while raised.len() < how_many {
                let name = FLAGGED[rng.below(FLAGGED.len())].0.to_string();
                if !raised.contains(&name) {
                    raised.push(name);
                }
            }
            Proposal { workspace, proposer, week: rng.below(WEEKS), raised }
        })
        .collect();

    Institution { units, people, holdings, labels, holders, workspaces, proposals, first_member }
}

fn chair_spec() -> ApproverSpec {
    ApproverSpec::Position { key: "chair".into(), from: Origin::Workspace, walk_up: true }
}

fn privacy_spec() -> ApproverSpec {
    ApproverSpec::Position { key: "privacy_officer".into(), from: Origin::Workspace, walk_up: true }
}

struct Matrix {
    name: &'static str,
    what: &'static str,
    signals: Vec<SignalRow>,
    rows: Vec<MatrixRow>,
}

fn row(tier: Tier, spec: ApproverSpec) -> MatrixRow {
    MatrixRow { organization_id: unit_uuid(0), tier, path: Path::Approver, approver: Some(spec), control_id: None, library_policy_key: None, library_rule_key: None }
}

/// The checks that touch personal data or lose it. A matrix that sends High to a privacy officer needs some checks to be High.
fn high_signals() -> Vec<SignalRow> {
    ["sensitivity", "page_sensitivity", "data_loss"].iter().map(|c| SignalRow { organization_id: unit_uuid(0), check_name: (*c).into(), tier: Tier::High }).collect()
}

fn matrices() -> Vec<Matrix> {
    vec![
        Matrix { name: "Built-in defaults", what: "No rows and no signals. Every raised check is Moderate and needs a second owner or admin.", signals: vec![], rows: vec![] },
        Matrix { name: "Moderate to the department chair", what: "One row at the institution: a Moderate change needs the chair of the workspace's department, found by walking up from its unit.", signals: vec![], rows: vec![row(Tier::Moderate, chair_spec())] },
        Matrix {
            name: "High to the privacy officer",
            what: "Three checks (sensitivity, page_sensitivity, data_loss) count as High at the institution. A High change needs the privacy officer. Other raised checks stay Moderate and need a second person.",
            signals: high_signals(),
            rows: vec![row(Tier::High, privacy_spec())],
        },
        Matrix { name: "Both together", what: "The two rows above and the three High signals.", signals: high_signals(), rows: vec![row(Tier::Moderate, chair_spec()), row(Tier::High, privacy_spec())] },
    ]
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scenario {
    Present,
    Vacant,
    Away,
}

impl Scenario {
    const ALL: [Scenario; 3] = [Scenario::Present, Scenario::Vacant, Scenario::Away];

    fn name(self) -> &'static str {
        match self {
            Scenario::Present => "Everyone present",
            Scenario::Vacant => "5% of positions vacant",
            Scenario::Away => "10% of holders away, with a delegate, for two weeks",
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
struct Run {
    scenario: Scenario,
    total: u64,
    direct: u64,
    second_person: u64,
    approver: u64,
    /// Direct path, and the proposer is an owner: nobody else was involved.
    applied_alone: u64,
    no_approver: u64,
    /// No one resolves, and the reason is that the only holder is the proposer.
    only_proposer: u64,
    /// A raised flagged check on the direct path. The floor says this is zero.
    flagged_direct: u64,
    /// People who received at least one item by name.
    approvers: u64,
    /// The approver with the largest week: label, median, 95th percentile, largest.
    busiest: Option<(String, u64, u64, u64)>,
}

fn context(inst: &Institution, scenario: Scenario) -> ResolveCtx {
    let mut holdings = inst.holdings.clone();
    let mut delegations = Vec::new();
    let mut rng = Lcg(SEED ^ 0x5eed);
    let positions: Vec<usize> = inst.holders.clone();
    match scenario {
        Scenario::Present => {}
        Scenario::Vacant => {
            // 5% of the positions, rounded up, stand empty.
            let mut vacant: Vec<usize> = Vec::new();
            while vacant.len() < (positions.len() * 5).div_ceil(100) {
                let n = positions[rng.below(positions.len())];
                if !vacant.contains(&n) {
                    vacant.push(n);
                }
            }
            holdings.retain(|h| h.position_key.is_none() || !vacant.iter().any(|n| h.eppn == eppn(*n)));
        }
        Scenario::Away => {
            // 10% of the holders, rounded up, name a delegate for two weeks.
            let mut away: Vec<usize> = Vec::new();
            while away.len() < (positions.len() * 10).div_ceil(100) {
                let n = positions[rng.below(positions.len())];
                if !away.contains(&n) {
                    away.push(n);
                }
            }
            for n in away {
                let start = rng.below(WEEKS - 2) as i64;
                let delegate = inst.first_member + rng.below(PEOPLE - inst.first_member);
                delegations.push(Delegation { delegator: eppn(n), delegate: eppn(delegate), position_key: None, starts_at: start * WEEK, ends_at: (start + 2) * WEEK, revoked: false });
            }
        }
    }
    ResolveCtx { units: inst.units.clone(), holdings, people: inst.people.clone(), delegations, collaborators: vec![], now: 0, submitter: None, starter: None, workspace_unit: None }
}

fn simulate(inst: &Institution, matrix: &Matrix, scenario: Scenario) -> Run {
    let mut ctx = context(inst, scenario);
    let paths: Vec<Vec<Uuid>> = (0..UNITS).map(unit_path).collect();
    let admins: Vec<usize> = (inst.holders.len()..inst.holders.len() + ADMINS).collect();
    let mut run = Run { scenario, total: 0, direct: 0, second_person: 0, approver: 0, applied_alone: 0, no_approver: 0, only_proposer: 0, flagged_direct: 0, approvers: 0, busiest: None };
    let mut weekly: BTreeMap<String, [u64; WEEKS]> = BTreeMap::new();

    for p in &inst.proposals {
        let w = &inst.workspaces[p.workspace];
        let need = required(&p.raised, &paths[w.unit], &matrix.signals, &matrix.rows);
        run.total += 1;
        match need.path {
            Path::Direct => {
                run.direct += 1;
                if !p.raised.is_empty() {
                    run.flagged_direct += 1;
                }
                if w.owners.contains(&p.proposer) {
                    run.applied_alone += 1;
                }
            }
            Path::SecondPerson => {
                run.second_person += 1;
                // An owner of the workspace or a platform admin, other than the proposer.
                let other = w.owners.iter().chain(admins.iter()).any(|n| *n != p.proposer);
                if !other {
                    run.no_approver += 1;
                }
            }
            Path::Approver => {
                run.approver += 1;
                let spec = need.approver.as_ref().expect("an approver path names its spec");
                ctx.submitter = Some(eppn(p.proposer));
                ctx.workspace_unit = Some(format!("u{}", w.unit));
                ctx.now = p.week as i64 * WEEK + 1;
                let found = resolve_approvers(spec, &ctx);
                if found.approvers.is_empty() {
                    run.no_approver += 1;
                    if found.problem == Some(NoApprover::OnlySubmitter) {
                        run.only_proposer += 1;
                    }
                }
                for a in found.approvers {
                    weekly.entry(a.eppn).or_insert([0; WEEKS])[p.week] += 1;
                }
            }
        }
    }

    run.approvers = weekly.len() as u64;
    let mut best: Option<(String, u64, u64, u64)> = None;
    for (who, weeks) in &weekly {
        let s = load_summary(weeks);
        let label = inst.labels.get(who).cloned().unwrap_or_else(|| who.clone());
        if best.as_ref().is_none_or(|b| s.largest > b.3) {
            best = Some((label, s.median, s.p95, s.largest));
        }
    }
    run.busiest = best;
    run
}

fn all_runs(inst: &Institution) -> Vec<(&'static str, Vec<Run>)> {
    matrices().iter().map(|m| (m.name, Scenario::ALL.iter().map(|s| simulate(inst, m, *s)).collect())).collect()
}

fn pct(n: u64, total: u64) -> String {
    let tenths = (n * 1000 + total / 2) / total.max(1);
    format!("{}.{}%", tenths / 10, tenths % 10)
}

/// The numbers and sentences, without the date and the run time. Two runs with one seed give equal text.
fn render(runs: &[(&'static str, Vec<Run>)]) -> String {
    let mut bottlenecks: Vec<String> = Vec::new();
    for (name, list) in runs {
        for r in list {
            if let Some((label, _, _, largest)) = &r.busiest {
                if *largest > 50 {
                    bottlenecks.push(format!("Under \"{name}\" ({}), the {label} has a busiest week of {largest} items.", r.scenario.name().to_lowercase()));
                }
            }
        }
    }
    let mut out = String::new();
    out.push_str("# Decision authority simulation\n\n");
    if bottlenecks.is_empty() {
        out.push_str("No position had a week above 50 items under any of the four matrices.\n\n");
    } else {
        out.push_str("One position is a bottleneck under the matrices that send High changes to a single privacy officer. ");
        out.push_str(&bottlenecks.iter().take(1).cloned().collect::<Vec<_>>().join(" "));
        out.push_str(" A delegation to a deputy, or a row at each college that sends High changes to the college's own privacy contact, would relieve it. ");
        out.push_str(&format!("{} of the twelve runs have a week above 50 items.\n\n", bottlenecks.len()));
    }
    out.push_str("This simulation does not measure waiting time, and it does not measure whether people find the screens clear. Waiting time needs real decisions: see `GET /admin/authority/load`.\n\n");
    out.push_str(&format!("Seed {SEED}. {PEOPLE} people, {UNITS} units in four levels (1 institution, {COLLEGES} colleges, {DEPARTMENTS} departments, {PROGRAMS} programs), {WORKSPACES} workspaces with one to four owners each, {PROPOSALS} proposals over {WEEKS} weeks. ", ));
    out.push_str("70% of proposals raise no check, 20% raise one flagged check, 10% raise two or three. 30% of proposers are owners. People who hold a position are never drawn as owners or members. Eight platform admins can be the second person on any workspace.\n\n");
    out.push_str("Away is a delegation window, the only absence the resolver knows. The holder still resolves and the delegate is added, so a delegate adds a second approver and does not take the first one away. A vacant position resolves to no one at that unit, and the walk continues up.\n\n");
    out.push_str("Each approver's load is the number of proposals that resolve to that person in a week. A proposal that resolves to two people counts for both. The busiest column is the approver with the largest week.\n\n");
    for m in matrices() {
        let list = &runs.iter().find(|(n, _)| *n == m.name).expect("a matrix was run").1;
        out.push_str(&format!("## {}\n\n{}\n\n", m.name, m.what));
        out.push_str("| Scenario | Direct | Second person | Approver | Applied by the proposer alone | No approver | Only the proposer | Approvers with items | Busiest approver | Median / 95th / largest week |\n");
        out.push_str("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");
        for r in list {
            let (who, nums) = match &r.busiest {
                Some((label, a, b, c)) => (label.clone(), format!("{a} / {b} / {c}")),
                None => ("none".to_string(), "0 / 0 / 0".to_string()),
            };
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
                r.scenario.name(),
                pct(r.direct, r.total),
                pct(r.second_person, r.total),
                pct(r.approver, r.total),
                pct(r.applied_alone, r.total),
                r.no_approver,
                r.only_proposer,
                r.approvers,
                who,
                nums
            ));
        }
        out.push('\n');
    }
    out
}

fn today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let z = secs / 86_400 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// The report with its two changing lines, after the title.
fn with_header(stable: &str, seconds: f64) -> String {
    let (title, rest) = stable.split_once("\n\n").expect("the report has a title");
    format!("{title}\n\nDate: {}. Run time for all twelve runs on the machine that wrote this: {seconds:.2} seconds.\n\n{rest}", today())
}

fn without_header(text: &str) -> String {
    text.lines().filter(|l| !l.starts_with("Date: ")).collect::<Vec<_>>().join("\n")
}

fn timed() -> (Institution, Vec<(&'static str, Vec<Run>)>, f64) {
    let inst = build();
    let start = Instant::now();
    let runs = all_runs(&inst);
    (inst, runs, start.elapsed().as_secs_f64())
}

#[test]
fn twelve_runs_finish_in_under_five_seconds() {
    let (_, runs, seconds) = timed();
    assert_eq!(runs.iter().map(|(_, l)| l.len()).sum::<usize>(), 12);
    assert!(seconds < 5.0, "twelve runs took {seconds:.2} seconds");
}

#[test]
fn with_everyone_present_every_proposal_has_an_approver() {
    let (_, runs, _) = timed();
    for (name, list) in &runs {
        let present = list.iter().find(|r| r.scenario == Scenario::Present).unwrap();
        assert_eq!(present.no_approver, 0, "{name}: {present:?}");
        assert_eq!(present.total, PROPOSALS as u64);
    }
}

#[test]
fn under_the_defaults_most_proposals_need_no_second_person() {
    let (_, runs, _) = timed();
    for r in &runs[0].1 {
        assert!(r.direct * 100 >= r.total * 60, "{} direct of {} under {:?}", r.direct, r.total, r.scenario);
    }
}

#[test]
fn no_run_applies_a_raised_flagged_check_on_the_direct_path() {
    let (_, runs, _) = timed();
    for (name, list) in &runs {
        for r in list {
            assert_eq!(r.flagged_direct, 0, "{name}, {:?}", r.scenario);
        }
    }
}

#[test]
fn a_privacy_officer_load_is_reported_and_not_limited() {
    let (_, runs, _) = timed();
    let (_, list) = runs.iter().find(|(n, _)| *n == "High to the privacy officer").unwrap();
    let present = list.iter().find(|r| r.scenario == Scenario::Present).unwrap();
    let (label, _, _, largest) = present.busiest.clone().expect("someone received items");
    assert!(label.starts_with("privacy officer"), "{label}");
    let text = render(&runs);
    let first = text.split("\n\n").nth(1).unwrap();
    if largest > 50 {
        assert!(first.contains("bottleneck"), "{first}");
    }
}

#[test]
fn the_report_names_its_seed_and_a_second_run_gives_the_same_numbers() {
    let (inst, runs, seconds) = timed();
    let first = render(&runs);
    assert!(first.contains(&format!("Seed {SEED}")));
    assert_eq!(first, render(&all_runs(&inst)));
    assert_eq!(first, render(&all_runs(&build())));

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(REPORT);
    if std::env::var("SCAFFOLDRY_WRITE_REPORT").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, with_header(&first, seconds)).unwrap();
    }
    let written = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{REPORT} is missing: run with SCAFFOLDRY_WRITE_REPORT=1"));
    let kept: String = without_header(&written).lines().filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n");
    let made: String = without_header(&with_header(&first, seconds)).lines().filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n");
    assert_eq!(kept, made, "{REPORT} is out of date: run with SCAFFOLDRY_WRITE_REPORT=1");
}
