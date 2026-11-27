//! Data layer phase 3b: the reads and decisions of audience fields, and the conditions on a person's record.
//!
//! Every statement on `record_audience` is in `scaffoldry_data::audience`. This file runs them, and the one that accepts a unit's name
//! runs in a transaction with its ledger entry, so it does all of it or none. A person's conditions are a small map on `persons`.

use crate::repository::{append_ledger_in_tx, AppendDecisionParams, PostgresRepository, RepositoryError};
use scaffoldry_core::reach::Holding;
use scaffoldry_data::audience::{self, Waiting};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use uuid::Uuid;

/// A person's holdings with their dates, and their conditions: each condition's last day.
pub type Standing = (Vec<(Holding, String)>, BTreeMap<String, String>);

/// What an acceptance did.
pub enum Accepted {
    /// The record does not name the unit, or is gone.
    NotNamed,
    /// The unit's name does not wait: it was accepted already, or the policy never asked.
    NothingWaits,
    /// The fields it accepted.
    Done(u64),
}

/// What the sizing of an audience found.
pub struct AudienceSize {
    /// Whether any accepted row names the reader's principals in the audience fields the statement may use.
    pub named: bool,
    /// Whether the rules and the audience together reach few rows.
    pub few: bool,
}

/// A person's condition map and where each came from, as stored.
#[derive(Debug, Clone, Default)]
pub struct Conditions {
    pub until: BTreeMap<String, String>,
    pub sources: Value,
}

fn map_of(v: Value) -> BTreeMap<String, String> {
    v.as_object().map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|d| (k.clone(), d.to_string()))).collect()).unwrap_or_default()
}

impl PostgresRepository {
    /// The records, among these, that name any of the principals, with the field and the table: `(record id, field, table id)`.
    pub fn audience_named(&self, app: &str, ids: Vec<String>, principals: Vec<String>) -> Result<Vec<(String, String, String)>, RepositoryError> {
        let app = app.to_string();
        self.with_client(move |c| Ok(audience::named_in(c, &app, &ids, &principals)?))
    }

    /// How many rows the reader's audience names, and whether rules and audience together are few (`FEW_ROWS`). Two small index reads, each
    /// with a limit. `None` when the plan has no audience. A rule of a shape that cannot be counted this way is not few.
    pub fn size_audience(&self, plan: &crate::repository::ScopePlan, app: &str, table: &str) -> Result<Option<AudienceSize>, RepositoryError> {
        let Some((sql, params, countable)) = plan.sizing(app, table) else { return Ok(None) };
        self.with_client(move |c| {
            let boxed = crate::repository::to_sql_params(params);
            let refs: Vec<&(dyn postgres::types::ToSql + Sync)> = boxed.iter().map(|b| b.as_ref() as &(dyn postgres::types::ToSql + Sync)).collect();
            let row = c.query_one(&sql, &refs)?;
            let (named, together): (i64, i64) = (row.get(0), row.get(1));
            Ok(Some(AudienceSize { named: named > 0, few: countable && together <= crate::repository::FEW_ROWS }))
        })
    }

    /// The unit values of a table that wait for their unit to accept.
    pub fn audience_waiting(&self, app: &str, table: &str, limit: i64) -> Result<Vec<Waiting>, RepositoryError> {
        let (app, table) = (app.to_string(), table.to_string());
        self.with_client(move |c| Ok(audience::waiting(c, &app, &table, limit)?))
    }

    /// Accepts a unit's name in one record. The acceptance and its ledger entry commit together.
    pub fn accept_audience(&self, app: &str, table: &str, record_id: &str, unit: Uuid, by: &str, ledger: impl FnOnce(&[String]) -> AppendDecisionParams + Send + 'static) -> Result<Accepted, RepositoryError> {
        let (app, table, record_id, by) = (app.to_string(), table.to_string(), record_id.to_string(), by.to_string());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            if !audience::names_unit(&mut tx, &app, &table, &record_id, unit)? {
                return Ok(Accepted::NotNamed);
            }
            let waiting = audience::waiting_fields(&mut tx, &record_id, unit)?;
            if waiting.is_empty() {
                return Ok(Accepted::NothingWaits);
            }
            let n = audience::accept(&mut tx, &record_id, unit, &by)?;
            append_ledger_in_tx(&mut tx, ledger(&waiting))?;
            tx.commit()?;
            Ok(Accepted::Done(n))
        })
    }

    /// The positions a person holds with their dates, and their conditions, in one read. A person who is not in the directory has neither.
    pub fn standing_of(&self, eppn: &str) -> Result<Standing, RepositoryError> {
        let eppn = eppn.to_string();
        self.with_client(move |c| {
            let rows = c.query(
                "SELECT r.organization_id, r.position_key, FLOOR(EXTRACT(EPOCH FROM r.effective_date::timestamptz))::bigint, \
                        FLOOR(EXTRACT(EPOCH FROM (r.end_date + 1)::timestamptz))::bigint - 1, r.scoped_affiliation, p.conditions \
                 FROM persons p LEFT JOIN roles r ON r.person_id = p.id WHERE lower(p.eppn) = lower($1)",
                &[&eppn],
            )?;
            let conditions = rows.first().map(|r| map_of(r.get::<_, Value>(5))).unwrap_or_default();
            let holdings = rows
                .iter()
                .filter_map(|r| Some((Holding { unit: r.get::<_, Option<Uuid>>(0)?, position_key: r.get(1), effective: r.get(2), end: r.get(3) }, r.get::<_, Option<String>>(4).unwrap_or_default())))
                .collect();
            Ok((holdings, conditions))
        })
    }

    /// The conditions on a person's record, or `None` when the person is not in the directory.
    pub fn conditions_of(&self, eppn: &str) -> Result<Option<Conditions>, RepositoryError> {
        let eppn = eppn.to_string();
        self.with_client(move |c| {
            Ok(c.query_opt("SELECT conditions, condition_sources FROM persons WHERE lower(eppn) = lower($1)", &[&eppn])?.map(|r| Conditions { until: map_of(r.get(0)), sources: r.get(1) }))
        })
    }

    /// Sets a condition's last day, or clears it with `None`, and records who, the owner and the source beside it. The change and its ledger
    /// entry commit together. `None` when the person is not in the directory. Returns the conditions as they are now.
    pub fn set_condition(&self, eppn: &str, id: &str, until: Option<&str>, source: Value, ledger: impl FnOnce(Option<&str>) -> AppendDecisionParams + Send + 'static) -> Result<Option<Conditions>, RepositoryError> {
        let (eppn, id, until) = (eppn.to_string(), id.to_string(), until.map(str::to_string));
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(row) = tx.query_opt("SELECT id, conditions FROM persons WHERE lower(eppn) = lower($1) FOR UPDATE", &[&eppn])? else { return Ok(None) };
            let (person, was): (Uuid, Value) = (row.get(0), row.get(1));
            let was = was.get(&id).and_then(Value::as_str).map(str::to_string);
            match &until {
                Some(day) => {
                    tx.execute(
                        "UPDATE persons SET conditions = conditions || jsonb_build_object($2::text, $3::text), condition_sources = condition_sources || jsonb_build_object($2::text, $4::jsonb) WHERE id = $1",
                        &[&person, &id, day, &source],
                    )?;
                }
                None => {
                    tx.execute("UPDATE persons SET conditions = conditions - $2::text, condition_sources = condition_sources - $2::text WHERE id = $1", &[&person, &id])?;
                }
            }
            append_ledger_in_tx(&mut tx, ledger(was.as_deref()))?;
            let now = tx.query_one("SELECT conditions, condition_sources FROM persons WHERE id = $1", &[&person])?;
            let out = Conditions { until: map_of(now.get(0)), sources: now.get(1) };
            tx.commit()?;
            Ok(Some(out))
        })
    }
}

/// How a condition is recorded beside its date.
pub fn source_of(owner: &str, source: &str, set_by: &str, at: &str) -> Value {
    json!({ "owner": owner, "source": source, "set_by": set_by, "set_at": at })
}
