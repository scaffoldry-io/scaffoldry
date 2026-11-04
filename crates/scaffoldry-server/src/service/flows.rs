//! The flow runner (workflows phase 1). A flow acts as the service identity `flow:{rule_id}`, on behalf of the person
//! whose action started it, inside the grants stored with its version.
//!
//! It names records by address: the trigger record, the records it links to, and records earlier steps of the same run
//! created or found. A find matches one indexed field through the index and never scans. There is no step that removes
//! a record. Every effect has a dedupe key recorded in the transaction of its write, so a retry never writes twice.
//! Row and column rules do not apply to the records a flow addresses: the grants and the addressing are the limit.

use crate::repository::ActorKind;
use crate::state::{DatasetRecord, SharedState};
use scaffoldry_core::workflow::{ActionType, AutomationRule, Expr, FlowGrant, StepKind, Target};
use scaffoldry_core::{ProcessInstance, ProcessStatus};
use scaffoldry_engine::flow::{expr_refs, trigger_table};
use scaffoldry_engine::{AppManifest, HostRouter, LinkCardinality};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub use crate::repository_index::FIND_SQL;

/// The most steps a flow may hold, unless the setting `flows.max_steps` says otherwise.
pub const DEFAULT_MAX_STEPS: i64 = 50;
/// The most writes one run may make, unless the setting `flows.max_writes_per_run` says otherwise.
pub const DEFAULT_MAX_WRITES: i64 = 1_000;

fn setting(state: &SharedState, key: &str, default: i64) -> i64 {
    state.settings.read().ok().and_then(|s| s.get(key).and_then(Value::as_i64)).unwrap_or(default)
}

pub fn max_steps(state: &SharedState) -> i64 {
    setting(state, "flows.max_steps", DEFAULT_MAX_STEPS)
}

pub fn max_writes(state: &SharedState) -> i64 {
    setting(state, "flows.max_writes_per_run", DEFAULT_MAX_WRITES)
}

/// The identity a flow acts as.
pub fn actor_of(rule_id: &str) -> String {
    format!("flow:{rule_id}")
}

/// The manifests a flow names: its own app's and every app a step reaches into.
pub fn manifests_for(state: &SharedState, rule: &AutomationRule) -> Vec<AppManifest> {
    let mut slugs = vec![rule.app_slug.clone()];
    for s in &rule.steps {
        if let StepKind::Service { action: ActionType::FindRecord { app, .. } | ActionType::CreateRecord { app, .. } } = &s.kind {
            if !slugs.contains(app) {
                slugs.push(app.clone());
            }
        }
    }
    let Ok(eng) = state.engine.read() else { return Vec::new() };
    slugs.iter().filter_map(|s| eng.resolve_by_slug(s).cloned()).collect()
}

/// The manifests a flow names, with `manifest` standing in for its own app: the version under review, which may not be
/// live yet.
pub fn manifests_with(state: &SharedState, manifest: &AppManifest, rule: &AutomationRule) -> Vec<AppManifest> {
    let mut ms = manifests_for(state, rule);
    ms.retain(|m| m.slug != manifest.slug);
    ms.insert(0, manifest.clone());
    ms
}

/// Derives the rule's grants from its steps, as the version will store them. Every save and every proposal does this,
/// so a single-app rule gets its grants when its next version is saved.
pub fn derive(state: &SharedState, manifest: &AppManifest, rule: &mut AutomationRule) {
    rule.grants = scaffoldry_engine::flow::derive_grants(rule, &manifests_with(state, manifest, rule));
}

/// True when a rule holds a step that finds, creates, updates or links records.
pub fn is_flow(rule: &AutomationRule) -> bool {
    rule.steps.iter().any(|s| matches!(&s.kind, StepKind::Service { action } if action.is_flow()))
}

/// Up to `limit` records of a table whose indexed field equals `value`, case-insensitive as the index is. The match
/// reads the index only; only the matching records are then read.
pub fn find_records(state: &SharedState, app_slug: &str, table_id: &str, field: &str, value: &Value, limit: usize) -> Result<Vec<DatasetRecord>, String> {
    let spec = {
        let eng = state.engine.read().map_err(|e| e.to_string())?;
        eng.resolve_by_slug(app_slug)
            .and_then(|m| m.tables.iter().find(|t| t.id == table_id))
            .and_then(|t| t.fields.iter().find(|f| f.name == field && f.indexed))
            .cloned()
            .ok_or_else(|| format!("the field '{field}' of '{table_id}' is not indexed"))?
    };
    let (num, txt, _) = scaffoldry_engine::index_value(&spec, value);
    if num.is_none() && txt.is_none() {
        return Ok(Vec::new());
    }
    let limit = limit.min(scaffoldry_core::workflow::MAX_FIND as usize) as i64;
    let ids = state.repository.find_by_index(app_slug, table_id, field, txt, num, limit).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(rec) = state.find_record(app_slug, &id)? {
            out.push(rec);
        }
    }
    Ok(out)
}

/// Why a run stopped.
struct Stop(String);

fn exceeded(what: String) -> Stop {
    Stop(format!("flow_grant_exceeded: {what}, which is outside the flow's approved grants"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Reads one record a flow addresses. The flow's grants and addressing are its limit, not the reader's row rules, so
/// this reads by id with no scope; every field it then reads or writes is checked against the grants first.
fn load(state: &SharedState, app_slug: &str, id: &str) -> Result<Option<DatasetRecord>, Stop> {
    state.find_record(app_slug, id).map_err(Stop)
}

/// A run in progress: what it found and created, and how many writes it made.
struct Run<'a> {
    state: &'a SharedState,
    rule: &'a AutomationRule,
    run_id: &'a str,
    instance_id: String,
    started_by: &'a str,
    started_at: String,
    trigger: DatasetRecord,
    trigger_table: String,
    /// Each finding or creating step's records, and the table they are in.
    produced: HashMap<String, (String, String, Vec<DatasetRecord>)>,
    writes: i64,
    max_writes: i64,
    /// Whether the run has a trigger record. A scheduled or called flow has none.
    has_trigger: bool,
    /// The values a `Called` flow was started with, by input name.
    inputs: Map<String, Value>,
    /// The names of the flows in this chain, this one last.
    chain: Vec<String>,
}

impl Run<'_> {
    fn grant(&self, app: &str, table: &str) -> Option<&FlowGrant> {
        self.rule.grants.iter().find(|g| g.app_slug == app && g.table_id == table)
    }

    fn may_read(&self, app: &str, table: &str, field: &str) -> Result<(), Stop> {
        if field == "id" || self.grant(app, table).is_some_and(|g| g.read.iter().any(|f| f == field)) {
            Ok(())
        } else {
            Err(exceeded(format!("it reads {table}.{field}")))
        }
    }

    fn may_write(&self, app: &str, table: &str, field: &str) -> Result<(), Stop> {
        if self.grant(app, table).is_some_and(|g| g.write.iter().any(|f| f == field)) {
            Ok(())
        } else {
            Err(exceeded(format!("it sets {table}.{field}")))
        }
    }

    /// The value of an expression. Each field it names is checked against the grants before it is read.
    fn eval(&self, expr: &Expr) -> Result<Value, Stop> {
        let mut ctx = Map::new();
        for (scope, field) in expr_refs(expr) {
            let key = format!("{scope}.{field}");
            let value = match scope.as_str() {
                "run" => match field.as_str() {
                    "id" => Value::String(self.run_id.to_string()),
                    "started_at" => Value::String(self.started_at.clone()),
                    _ => Value::Null,
                },
                "trigger" if !self.has_trigger => return Err(Stop(format!("{{{key}}} names the trigger record, and this run has none"))),
                "trigger" => {
                    self.may_read(&self.trigger.app_slug, &self.trigger_table, &field)?;
                    value_of(&self.trigger, &field)
                }
                "input" => self.inputs.get(&field).cloned().unwrap_or(Value::Null),
                step => {
                    let Some((app, table, recs)) = self.produced.get(step) else { return Err(Stop(format!("{{{key}}} names a step that has not run"))) };
                    self.may_read(app, table, &field)?;
                    recs.first().map(|r| value_of(r, &field)).unwrap_or(Value::Null)
                }
            };
            ctx.insert(key, value);
        }
        Ok(scaffoldry_engine::evaluate_formula(expr, &Value::Object(ctx)))
    }

    /// The app, table and records a target names, read now.
    fn records(&self, t: &Target) -> Result<(String, String, Vec<DatasetRecord>), Stop> {
        match t {
            Target::Trigger => {
                let fresh = load(self.state, &self.trigger.app_slug, &self.trigger.id)?.unwrap_or_else(|| self.trigger.clone());
                Ok((self.trigger.app_slug.clone(), self.trigger_table.clone(), vec![fresh]))
            }
            Target::Created { step } | Target::Found { step } => {
                let (app, table, recs) = self.produced.get(step).cloned().ok_or_else(|| Stop(format!("step '{step}' has not run")))?;
                let mut fresh = Vec::with_capacity(recs.len());
                for r in recs {
                    if let Some(now) = load(self.state, &app, &r.id)? {
                        fresh.push(now);
                    }
                }
                Ok((app, table, fresh))
            }
            Target::Linked { link_field } => {
                self.may_read(&self.trigger.app_slug, &self.trigger_table, link_field)?;
                let (_, target) = link_target(self.state, &self.trigger.app_slug, &self.trigger_table, link_field).ok_or_else(|| Stop(format!("'{link_field}' is not a link field")))?;
                let mut out = Vec::new();
                for id in crate::repository_links::linked_ids(self.trigger.data.get(link_field)) {
                    if let Some(r) = load(self.state, &self.trigger.app_slug, &id)? {
                        out.push(r);
                    }
                }
                Ok((self.trigger.app_slug.clone(), target, out))
            }
        }
    }

    /// The dedupe key of one effect: the run, the step, the record, and the fields it sets.
    fn key(&self, step: &str, target: &str, fields: &[&str]) -> String {
        let mut h = Sha256::new();
        h.update(format!("{}\u{1f}{step}\u{1f}{target}\u{1f}{}", self.run_id, fields.join(",")));
        hex(&h.finalize())
    }

    fn count_write(&mut self) -> Result<(), Stop> {
        self.writes += 1;
        if self.writes > self.max_writes {
            return Err(Stop(format!("the run would make {} writes, and the limit is {} writes per run", self.writes, self.max_writes)));
        }
        Ok(())
    }

    fn ctx(&self, app: &str, data: &Value, key: String) -> crate::repository::WriteCtx {
        let mut ctx = crate::service::history::ctx_for(&actor_of(&self.rule.id), ActorKind::Automation, app, data, self.state);
        ctx.on_behalf_of = Some(self.started_by.to_string());
        ctx.batch_id = uuid::Uuid::from_bytes(Sha256::digest(self.run_id.as_bytes())[..16].try_into().unwrap_or([0; 16]));
        ctx.effect = Some((key, self.instance_id.clone()));
        ctx
    }

    fn step(&mut self, step: &str, action: &ActionType) -> Result<(), Stop> {
        match action {
            ActionType::FindRecord { id, app, table, field, equals, limit } => {
                self.may_read(app, table, field)?;
                let value = self.eval(equals)?;
                let found = find_records(self.state, app, table, field, &value, *limit as usize).map_err(Stop)?;
                self.produced.insert(id.clone(), (app.clone(), table.clone(), found));
            }
            ActionType::CreateRecord { id, app, table, fields } => {
                if !self.grant(app, table).is_some_and(|g| g.create) {
                    return Err(exceeded(format!("it creates a record in {table}")));
                }
                let mut data = Map::new();
                data.insert("_table_id".to_string(), Value::String(table.clone()));
                for (f, e) in fields {
                    self.may_write(app, table, f)?;
                    data.insert(f.clone(), self.eval(e)?);
                }
                // The record's id follows from the run and the step, so a retry names the same record.
                let names: Vec<&str> = fields.iter().map(|(f, _)| f.as_str()).collect();
                let key = self.key(step, "new", &names);
                let record_id = uuid::Uuid::from_bytes(Sha256::digest(key.as_bytes())[..16].try_into().unwrap_or([0; 16])).to_string();
                self.count_write()?;
                let data = Value::Object(data);
                let rec = DatasetRecord {
                    id: record_id.clone(),
                    app_slug: app.clone(),
                    data: data.clone(),
                    ceds_mapping: Value::Object(Map::new()),
                    is_ferpa_sensitive: false,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    created_by: Some(actor_of(&self.rule.id)),
                    version: 1,
                };
                let ctx = self.ctx(app, &data, key);
                self.state.insert_record(&rec, &ctx).map_err(|e| Stop(e.to_string()))?;
                let stored = load(self.state, app, &record_id)?.unwrap_or(rec);
                self.produced.insert(id.clone(), (app.clone(), table.clone(), vec![stored]));
            }
            ActionType::UpdateRecord { target, fields } => {
                let (app, table, recs) = self.records(target)?;
                for (f, _) in fields {
                    self.may_write(&app, &table, f)?;
                }
                let names: Vec<&str> = fields.iter().map(|(f, _)| f.as_str()).collect();
                for rec in recs {
                    let mut data = rec.data.clone();
                    for (f, e) in fields {
                        let v = self.eval(e)?;
                        if let Some(map) = data.as_object_mut() {
                            map.insert(f.clone(), v);
                        }
                    }
                    self.count_write()?;
                    let ctx = self.ctx(&app, &data, self.key(step, &rec.id, &names));
                    self.state.overwrite_record_data(&app, &rec.id, &data, &ctx).map_err(Stop)?;
                }
            }
            ActionType::SetLink { target, link_field, to } => {
                let (app, table, recs) = self.records(target)?;
                if !self.grant(&app, &table).is_some_and(|g| g.link.iter().any(|f| f == link_field)) {
                    return Err(exceeded(format!("it links {table}.{link_field}")));
                }
                let (_, _, others) = self.records(to)?;
                let ids: Vec<Value> = others.iter().map(|r| Value::String(r.id.clone())).collect();
                let one = link_target(self.state, &app, &table, link_field).is_some_and(|(c, _)| c == LinkCardinality::One);
                let value = if one { ids.first().cloned().unwrap_or(Value::Null) } else { Value::Array(ids) };
                for rec in recs {
                    let mut data = rec.data.clone();
                    if let Some(map) = data.as_object_mut() {
                        map.insert(link_field.clone(), value.clone());
                    }
                    self.count_write()?;
                    let ctx = self.ctx(&app, &data, self.key(step, &rec.id, &[link_field.as_str()]));
                    self.state.overwrite_record_data(&app, &rec.id, &data, &ctx).map_err(Stop)?;
                }
            }
            // Workflows phase 3: a new run of a called flow, one level deeper in the chain. It is not waited on: its outcome
            // is its own.
            ActionType::StartFlow { flow_id, inputs } => {
                let target = find_flow(self.state, flow_id).ok_or_else(|| Stop(format!("step '{step}' starts the flow '{flow_id}', which is not live")))?;
                let mut values = Map::new();
                for (name, e) in inputs {
                    values.insert(name.clone(), self.eval(e)?);
                }
                // The called flow's outcome, failed or not, is in its own instance. This run goes on.
                start_flow(self.state, &target, None, values, self.started_by, &uuid::Uuid::new_v4().to_string(), &self.chain);
            }
            // Notices, status updates and ledger notes run through the rule engine as before.
            _ => {}
        }
        Ok(())
    }
}

fn value_of(rec: &DatasetRecord, field: &str) -> Value {
    if field == "id" {
        return Value::String(rec.id.clone());
    }
    rec.data.get(field).cloned().unwrap_or(Value::Null)
}

/// A link field's cardinality and target table.
fn link_target(state: &SharedState, app: &str, table: &str, field: &str) -> Option<(LinkCardinality, String)> {
    let eng = state.engine.read().ok()?;
    let f = eng.resolve_by_slug(app)?.tables.iter().find(|t| t.id == table)?.fields.iter().find(|f| f.name == field)?.clone();
    Some((f.link_cardinality, f.link_target_table?))
}

/// Runs steps in order. Every condition reads the trigger record inside the grants, like any other read. A branch runs
/// its first arm whose conditions hold, or `otherwise`. Returns true when a decision step stopped the run: it waits in
/// its own instance, and this phase runs the record steps before it.
fn run_steps(run: &mut Run<'_>, inst: &mut ProcessInstance, steps: &[scaffoldry_core::ProcessStep]) -> Result<bool, Stop> {
    let holds = |run: &Run<'_>, when: &[scaffoldry_core::FieldPredicate]| -> Result<bool, Stop> {
        for p in when {
            run.may_read(&run.trigger.app_slug, &run.trigger_table, &p.field_name)?;
        }
        Ok(when.iter().all(|p| scaffoldry_engine::AutomationEngine::evaluate_predicate(p, &run.trigger.data)))
    };
    for s in steps {
        if !holds(run, &s.when)? {
            continue;
        }
        match &s.kind {
            StepKind::Service { action } => {
                run.step(&s.id, action)?;
                inst.log.push(format!("Step '{}' done", s.id));
            }
            StepKind::Branch { branches, otherwise } => {
                let mut chosen: Option<(usize, &[scaffoldry_core::ProcessStep])> = None;
                for (i, b) in branches.iter().enumerate() {
                    if holds(run, &b.when)? {
                        chosen = Some((i + 1, &b.steps));
                        break;
                    }
                }
                let (arm, next) = chosen.map_or(("otherwise".to_string(), otherwise.as_slice()), |(i, st)| (i.to_string(), st));
                inst.log.push(format!("Branch '{}' took {arm}", s.id));
                if run_steps(run, inst, next)? {
                    return Ok(true);
                }
            }
            StepKind::UserTask { .. } => return Ok(true),
        }
    }
    Ok(false)
}

/// Runs a flow for one trigger record, as one run with the id `run_id`. A run with an id that already ran is a retry:
/// effects it made before are skipped by their dedupe keys. The instance is stored `Completed`, or `Failed` with the
/// reason in its log, and returned.
pub fn run_flow(state: &SharedState, rule: &AutomationRule, trigger: &DatasetRecord, started_by: &str, run_id: &str) -> ProcessInstance {
    start_flow(state, rule, Some(trigger), Map::new(), started_by, run_id, &[])
}

/// The live flow with this id, in any app.
pub fn find_flow(state: &SharedState, flow_id: &str) -> Option<AutomationRule> {
    state.automations.read().ok()?.values().flatten().find(|r| r.id == flow_id).cloned()
}

/// How many failures in a row turn a flow off.
pub const BREAKER_FAILURES: i32 = 20;
/// The most runs one flow starts in a minute, unless the setting `flows.max_runs_per_minute` says otherwise.
pub const DEFAULT_MAX_RUNS_PER_MINUTE: i64 = 600;

/// A flow's failures in a row, and whether its breaker has turned it off.
pub fn breaker(state: &SharedState, flow_id: &str) -> (i32, bool) {
    state.repository.flow_breaker(flow_id).unwrap_or((0, false))
}

/// Starts one run of a flow. `trigger` is the record that started it, or `None` for a scheduled or called flow; `inputs`
/// are a called flow's values; `chain` names the flows that led here. The run is refused, and says why in its log, when
/// the breaker has turned the flow off, when it is past `flows.max_runs_per_minute`, or when the chain is too deep. A
/// failed run counts toward the breaker and a completed one clears it. When the run ends, the flows that wait for it
/// start.
pub fn start_flow(state: &SharedState, rule: &AutomationRule, trigger: Option<&DatasetRecord>, inputs: Map<String, Value>, started_by: &str, run_id: &str, chain: &[String]) -> ProcessInstance {
    let started_at = chrono::Utc::now().to_rfc3339();
    let none = DatasetRecord {
        id: String::new(),
        app_slug: rule.app_slug.clone(),
        data: Value::Object(Map::new()),
        ceds_mapping: Value::Object(Map::new()),
        is_ferpa_sensitive: false,
        created_at: started_at.clone(),
        created_by: None,
        version: 0,
    };
    let trigger_rec = trigger.cloned().unwrap_or(none);
    let mut inst = ProcessInstance {
        id: format!("{}:{run_id}", rule.id),
        rule_id: rule.id.clone(),
        app_slug: rule.app_slug.clone(),
        record_id: trigger_rec.id.clone(),
        status: ProcessStatus::Completed,
        waiting_step_id: None,
        role: None,
        prompt: None,
        log: Vec::new(),
        started_by: Some(started_by.to_string()),
        started_at: started_at.clone(),
        assigned_to: None,
        no_approver: None,
        rule_version: rule.version,
    };
    let mut names: Vec<String> = chain.to_vec();
    names.push(rule.name.clone());
    // Refusals that are not the flow's failure: nothing ran, and the breaker does not count them.
    let refused = if breaker(state, &rule.id).1 {
        Some(format!("The flow is turned off after {BREAKER_FAILURES} failed runs in a row. Save a new version of it to turn it on again"))
    } else if chain.len() >= scaffoldry_core::workflow::MAX_CHAIN_DEPTH {
        None
    } else if !state.repository.flow_rate_slot(&rule.id, max_runs_per_minute(state)).unwrap_or(true) {
        Some(format!("rate_limited: the flow started {} runs in the last minute, the most it may", max_runs_per_minute(state)))
    } else {
        None
    };
    if let Some(reason) = refused {
        inst.status = ProcessStatus::Failed;
        inst.log.push(reason);
        let _ = state.repository.upsert_process_instance(&inst);
        return inst;
    }
    let manifests = manifests_for(state, rule);
    let table = trigger_rec.data.get("_table_id").and_then(Value::as_str).map(str::to_string).or_else(|| trigger_table(rule, &manifests)).unwrap_or_default();
    let mut run = Run {
        state,
        rule,
        run_id,
        instance_id: inst.id.clone(),
        started_by,
        started_at,
        trigger: trigger_rec.clone(),
        trigger_table: table,
        produced: HashMap::new(),
        writes: 0,
        max_writes: max_writes(state),
        has_trigger: trigger.is_some(),
        inputs,
        chain: names.clone(),
    };
    let outcome = (|| {
        if chain.len() >= scaffoldry_core::workflow::MAX_CHAIN_DEPTH {
            return Err(Stop(format!(
                "the chain is too long: {}. A flow may start another at most {} levels deep",
                names.join(" > "),
                scaffoldry_core::workflow::MAX_CHAIN_DEPTH
            )));
        }
        let limit = max_steps(state);
        let count = scaffoldry_core::workflow::all_steps(&rule.steps).len();
        if count as i64 > limit {
            return Err(Stop(format!("the flow has {count} steps, and the limit is {limit}")));
        }
        run_steps(&mut run, &mut inst, &rule.steps).map(|_| ())
    })();
    if let Err(Stop(reason)) = outcome {
        inst.status = ProcessStatus::Failed;
        inst.log.push(reason);
    }
    inst.log.push(format!("{} writes", run.writes));
    if let Err(e) = state.repository.upsert_process_instance(&inst) {
        eprintln!("the flow run {} was not stored: {e}", inst.id);
    }
    // The breaker counts failures in a row. The twentieth turns the flow off and tells its owners.
    if inst.status == ProcessStatus::Failed {
        if let Ok((_, true)) = state.repository.flow_failed(&rule.id, BREAKER_FAILURES) {
            tell_tripped(state, rule, &run.trigger_table);
        }
    } else {
        let _ = state.repository.flow_succeeded(&rule.id);
    }
    crate::service::flow_chains::on_completed(state, rule, &inst, trigger, &names);
    inst
}

/// The most runs one flow starts in a minute.
pub fn max_runs_per_minute(state: &SharedState) -> i64 {
    setting(state, "flows.max_runs_per_minute", DEFAULT_MAX_RUNS_PER_MINUTE)
}

/// The owners and admins of the flow's workspace are told it was turned off. The notice names the app and table only.
fn tell_tripped(state: &SharedState, rule: &AutomationRule, table: &str) {
    let workspace = rule.home_workspace.clone().or_else(|| {
        let eng = state.engine.read().ok()?;
        eng.resolve_by_slug(&rule.app_slug).and_then(|m| m.workspace_id.clone())
    });
    let Some(workspace) = workspace else { return };
    let notice = crate::notifications::Notice::ProcessNotice { app: crate::notifications::AppRef(rule.app_slug.clone()), table: crate::notifications::TableRef(table.to_string()) };
    let to = crate::notifications::Recipients::Workspace { workspace, roles: vec!["owner".into(), "admin".into()] };
    let _ = crate::notifications::notify(state, &to, None, &notice);
}
