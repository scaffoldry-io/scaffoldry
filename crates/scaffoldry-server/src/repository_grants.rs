//! Access model phase 4: grants and the changes that wait for a second person. Grants are not cached. They are read
//! by holder on each request, with the index on `(holder_kind, holder_id)`.

use crate::repository::{PostgresRepository, RepositoryError};
use scaffoldry_core::reach::{AnchorKind, Grant, HolderKind, Holding, Role};
use serde_json::{json, Value};
use uuid::Uuid;

const GRANT_SELECT: &str = "SELECT id, holder_kind, holder_id, role, hierarchy_key, anchor_kind, anchor_id, depth, domain, clearance, \
    FLOOR(EXTRACT(EPOCH FROM valid_from))::bigint, FLOOR(EXTRACT(EPOCH FROM valid_until))::bigint, FLOOR(EXTRACT(EPOCH FROM revoked_at))::bigint, revoked_by, \
    source, reason, created_by, to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
    to_char(valid_from AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), to_char(valid_until AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
    to_char(revoked_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') FROM access_grants";

/// A stored grant with everything the API returns.
#[derive(Debug, Clone)]
pub struct GrantRow {
    pub grant: Grant,
    pub hierarchy: Option<String>,
    pub clearance: Option<String>,
    pub revoked_by: Option<String>,
    pub source: String,
    pub reason: String,
    pub created_by: String,
    pub created_at: String,
    pub valid_from: String,
    pub valid_until: Option<String>,
    pub revoked_at: Option<String>,
}

impl GrantRow {
    fn of(r: &postgres::Row) -> Option<GrantRow> {
        let grant = Grant {
            id: r.get(0),
            holder_kind: HolderKind::parse(&r.get::<_, String>(1))?,
            holder_id: r.get(2),
            role: Role::parse(&r.get::<_, String>(3))?,
            hierarchy: r.get(4),
            anchor_kind: AnchorKind::parse(&r.get::<_, String>(5))?,
            anchor_id: r.get(6),
            depth: r.get(7),
            domain: r.get(8),
            valid_from: r.get(10),
            valid_until: r.get(11),
            revoked: r.get::<_, Option<i64>>(12).is_some(),
            clearance: r.get::<_, Option<String>>(9).and_then(|c| scaffoldry_core::coordinates::sensitivity_rank(&c)),
        };
        Some(GrantRow {
            hierarchy: grant.hierarchy.clone(),
            clearance: r.get(9),
            revoked_by: r.get(13),
            source: r.get(14),
            reason: r.get(15),
            created_by: r.get(16),
            created_at: r.get(17),
            valid_from: r.get(18),
            valid_until: r.get(19),
            revoked_at: r.get(20),
            grant,
        })
    }

    /// The row as the schema `access-grant.schema.json` describes it.
    pub fn json(&self) -> Value {
        let g = &self.grant;
        json!({
            "id": g.id, "holder_kind": g.holder_kind.as_str(), "holder_id": g.holder_id, "role": g.role.as_str(),
            "hierarchy": self.hierarchy, "anchor_kind": g.anchor_kind.as_str(), "anchor_id": g.anchor_id, "depth": g.depth,
            "domain": g.domain, "clearance": self.clearance, "valid_from": self.valid_from, "valid_until": self.valid_until,
            "revoked_at": self.revoked_at, "revoked_by": self.revoked_by, "source": self.source, "reason": self.reason,
            "created_by": self.created_by, "created_at": self.created_at,
        })
    }
}

/// The filters of the grant list.
#[derive(Debug, Default, Clone)]
pub struct GrantFilter {
    pub holder: Option<String>,
    pub anchor: Option<String>,
    pub hierarchy: Option<String>,
    pub source: Option<String>,
    /// `active`, `revoked`, `expired`, or `all`.
    pub state: String,
    pub after: i64,
    pub limit: i64,
}

/// What `decide_access_change` found.
pub enum ChangeDecision {
    /// Decided. A confirmed grant change names the grant it made.
    Done { grant_id: Option<i64> },
    Missing,
    AlreadyDecided,
    OwnChange,
    /// An AI tool's draft that is more than seven days old.
    Expired,
}

pub enum RetireOutcome {
    Done,
    Missing,
    Protected,
    AlreadyRetired,
}

pub enum RevokeOutcome {
    Done,
    Missing,
    AlreadyRevoked,
}

impl PostgresRepository {
    /// Unrevoked grants held by any of these `(kind, id)` pairs.
    pub fn grants_held_by(&self, pairs: &[(String, String)]) -> Result<Vec<Grant>, RepositoryError> {
        if pairs.is_empty() {
            return Ok(Vec::new());
        }
        let (kinds, ids): (Vec<String>, Vec<String>) = pairs.iter().cloned().unzip();
        self.with_client(move |c| {
            // A grant in a retired hierarchy stays on record and reaches nothing.
            let sql = format!(
                "{GRANT_SELECT} WHERE revoked_at IS NULL AND (holder_kind, holder_id) IN (SELECT * FROM unnest($1::text[], $2::text[])) \
                 AND (hierarchy_key IS NULL OR hierarchy_key IN (SELECT key FROM hierarchies WHERE status = 'Active'))"
            );
            Ok(c.query(&sql, &[&kinds, &ids])?.iter().filter_map(GrantRow::of).map(|r| r.grant).collect())
        })
    }

    /// The roles a person holds with their dates and their scoped affiliation, for the holder set and for the grants an
    /// appointment implies.
    pub fn holdings_of(&self, eppn: &str) -> Result<Vec<(Holding, String)>, RepositoryError> {
        let eppn = eppn.to_string();
        self.with_client(move |c| {
            let rows = c.query(
                "SELECT r.organization_id, r.position_key, FLOOR(EXTRACT(EPOCH FROM r.effective_date::timestamptz))::bigint, \
                        FLOOR(EXTRACT(EPOCH FROM (r.end_date + 1)::timestamptz))::bigint - 1, r.scoped_affiliation \
                 FROM roles r JOIN persons p ON r.person_id = p.id WHERE lower(p.eppn) = lower($1)",
                &[&eppn],
            )?;
            Ok(rows.iter().map(|r| (Holding { unit: r.get(0), position_key: r.get(1), effective: r.get(2), end: r.get(3) }, r.get(4))).collect())
        })
    }

    /// The addresses, lowercase, of the people with an appointment at any of these units today. A hierarchy of people
    /// places a row by a person, so this turns the units a grant reaches into the people whose rows it reaches.
    pub fn appointed_addresses(&self, units: &[uuid::Uuid]) -> Result<Vec<String>, RepositoryError> {
        let units = units.to_vec();
        self.with_client(move |c| {
            let rows = c.query(
                "SELECT DISTINCT lower(p.eppn) FROM roles r JOIN persons p ON p.id = r.person_id \
                 WHERE r.organization_id = ANY($1) AND r.effective_date <= CURRENT_DATE AND (r.end_date IS NULL OR r.end_date >= CURRENT_DATE) \
                 ORDER BY 1",
                &[&units],
            )?;
            Ok(rows.iter().map(|r| r.get(0)).collect())
        })
    }

    /// The addresses, lowercase, of the people with this position today, at one unit or at any.
    pub fn appointed_to_position(&self, key: &str, unit: Option<uuid::Uuid>) -> Result<Vec<String>, RepositoryError> {
        let key = key.to_string();
        self.with_client(move |c| {
            let rows = c.query(
                "SELECT DISTINCT lower(p.eppn) FROM roles r JOIN persons p ON p.id = r.person_id \
                 WHERE r.position_key = $1 AND ($2::uuid IS NULL OR r.organization_id = $2) \
                 AND r.effective_date <= CURRENT_DATE AND (r.end_date IS NULL OR r.end_date >= CURRENT_DATE) ORDER BY 1",
                &[&key, &unit],
            )?;
            Ok(rows.iter().map(|r| r.get(0)).collect())
        })
    }

    pub fn list_grant_rows(&self, f: &GrantFilter) -> Result<Vec<GrantRow>, RepositoryError> {
        let f = f.clone();
        self.with_client(move |c| {
            let state = match f.state.as_str() {
                "revoked" => "revoked_at IS NOT NULL",
                "expired" => "revoked_at IS NULL AND valid_until IS NOT NULL AND valid_until <= NOW()",
                "all" => "TRUE",
                _ => "revoked_at IS NULL AND (valid_until IS NULL OR valid_until > NOW())",
            };
            let sql = format!(
                "{GRANT_SELECT} WHERE id > $1 AND {state} AND ($2::text IS NULL OR lower(holder_id) = lower($2)) AND ($3::text IS NULL OR anchor_id = $3) \
                 AND ($4::text IS NULL OR hierarchy_key = $4) AND ($5::text IS NULL OR source = $5) ORDER BY id LIMIT $6"
            );
            Ok(c.query(&sql, &[&f.after, &f.holder, &f.anchor, &f.hierarchy, &f.source, &f.limit])?.iter().filter_map(GrantRow::of).collect())
        })
    }

    /// Active grants older than `days` days that no receipt in the last `days` days names. Empty when no receipt exists at
    /// all, since nothing can then be said about use.
    pub fn unused_grant_ids(&self, days: i64) -> Result<Vec<i64>, RepositoryError> {
        self.with_client(move |c| {
            let days = days as i32;
            Ok(c.query(
                "SELECT g.id FROM access_grants g \
                 WHERE EXISTS (SELECT 1 FROM access_receipts) AND g.revoked_at IS NULL AND (g.valid_until IS NULL OR g.valid_until > NOW()) \
                   AND g.created_at < NOW() - make_interval(days => $1) \
                   AND NOT EXISTS (SELECT 1 FROM access_receipts r WHERE r.at > NOW() - make_interval(days => $1) AND g.id = ANY(r.grant_ids)) \
                 ORDER BY g.id LIMIT 500",
                &[&days],
            )?
            .iter()
            .map(|r| r.get(0))
            .collect())
        })
    }

    /// Active grants nobody has reviewed for `days` days, by id, a page after `after`.
    pub fn due_grant_rows(&self, days: i64, after: i64, limit: i64) -> Result<Vec<GrantRow>, RepositoryError> {
        let days = i32::try_from(days).unwrap_or(i32::MAX);
        self.with_client(move |c| {
            let sql = format!(
                "{GRANT_SELECT} WHERE id > $3 AND revoked_at IS NULL AND (valid_until IS NULL OR valid_until > NOW()) \
                 AND COALESCE(reviewed_at, created_at) < NOW() - make_interval(days => $1) ORDER BY id LIMIT $2"
            );
            Ok(c.query(&sql, &[&days, &limit, &after])?.iter().filter_map(GrantRow::of).collect())
        })
    }

    /// Retires a hierarchy. Its grants stop reaching, and its units stay. The academic hierarchy and one already retired are refused.
    pub fn retire_hierarchy(&self, key: &str) -> Result<RetireOutcome, RepositoryError> {
        let key = key.to_string();
        self.with_client(move |c| {
            let Some(row) = c.query_opt("SELECT status FROM hierarchies WHERE key = $1 FOR UPDATE", &[&key])? else { return Ok(RetireOutcome::Missing) };
            let status: String = row.get(0);
            if key == "academic" {
                return Ok(RetireOutcome::Protected);
            }
            if status != "Active" {
                return Ok(RetireOutcome::AlreadyRetired);
            }
            c.execute("UPDATE hierarchies SET status = 'Retired', retired_at = NOW() WHERE key = $1", &[&key])?;
            Ok(RetireOutcome::Done)
        })
    }

    /// Every domain a grant can be limited to, with its name.
    pub fn domain_names(&self) -> Result<Vec<(String, String)>, RepositoryError> {
        self.with_client(|c| {
            let rows = c.query("SELECT key, name FROM access_domains WHERE status = 'Active' ORDER BY name, key", &[])?;
            Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
        })
    }

    /// People whose address or name holds `q`, by address, a page after `after`.
    pub fn search_people(&self, q: &str, after: &str, limit: i64) -> Result<Vec<(String, String)>, RepositoryError> {
        let pattern = format!("%{}%", q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        let after = after.to_lowercase();
        self.with_client(move |c| {
            let rows = c.query(
                "SELECT lower(user_name), COALESCE(payload->'name'->>'formatted', user_name) FROM scim_users \
                 WHERE lower(user_name) > $2 AND active AND (user_name ILIKE $1 OR payload->'name'->>'formatted' ILIKE $1) ORDER BY lower(user_name) LIMIT $3",
                &[&pattern, &after, &limit],
            )?;
            Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
        })
    }

    /// Resets the review clock of an active grant. False when there is no such active grant.
    pub fn mark_grant_reviewed(&self, id: i64, by: &str) -> Result<bool, RepositoryError> {
        let by = by.to_string();
        self.with_client(move |c| Ok(c.execute("UPDATE access_grants SET reviewed_at = NOW(), reviewed_by = $2 WHERE id = $1 AND revoked_at IS NULL", &[&id, &by])? == 1))
    }

    /// The addresses, lowercase, of everyone who holds an active grant as a person.
    pub fn person_grant_holders(&self) -> Result<Vec<String>, RepositoryError> {
        self.with_client(|c| {
            let rows = c.query("SELECT DISTINCT lower(holder_id) FROM access_grants WHERE holder_kind = 'person' AND revoked_at IS NULL ORDER BY 1", &[])?;
            Ok(rows.iter().map(|r| r.get(0)).collect())
        })
    }

    pub fn grant_row(&self, id: i64) -> Result<Option<GrantRow>, RepositoryError> {
        self.with_client(move |c| Ok(c.query_opt(&format!("{GRANT_SELECT} WHERE id = $1"), &[&id])?.as_ref().and_then(GrantRow::of)))
    }

    /// Stores a change that waits for a second person.
    pub fn propose_access_change(&self, change: &Value, widens: bool, reason: &str, proposed_by: &str) -> Result<Uuid, RepositoryError> {
        let (change, reason, proposed_by) = (change.clone(), reason.to_string(), proposed_by.to_string());
        self.with_client(move |c| {
            let id = Uuid::new_v4();
            c.execute(
                "INSERT INTO access_changes (id, change, widens, reason, proposed_by) VALUES ($1, $2, $3, $4, $5)",
                &[&id, &change, &widens, &reason, &proposed_by],
            )?;
            Ok(id)
        })
    }

    /// Stores a change an AI tool drafted. It records the person it was drafted for, the tool and the task, and ends in
    /// seven days.
    pub fn propose_access_draft(&self, change: &Value, widens: bool, reason: &str, proposed_by: &str, client: Option<&str>, task: Option<&str>) -> Result<(Uuid, String), RepositoryError> {
        let (change, reason, proposed_by, client, task) = (change.clone(), reason.to_string(), proposed_by.to_string(), client.map(str::to_string), task.map(str::to_string));
        self.with_client(move |c| {
            let id = Uuid::new_v4();
            let row = c.query_one(
                "INSERT INTO access_changes (id, change, widens, reason, proposed_by, proposed_via, drafted_for, via_client, task, expires_at) \
                 VALUES ($1, $2, $3, $4, $5, 'agent', $5, $6, $7, NOW() + INTERVAL '7 days') \
                 RETURNING to_char(expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')",
                &[&id, &change, &widens, &reason, &proposed_by, &client, &task],
            )?;
            Ok((id, row.get(0)))
        })
    }

    /// How a change came about and whether it ended: `(proposed_via, widens, expired)`.
    pub fn access_change_flags(&self, id: Uuid) -> Result<Option<(String, bool, bool)>, RepositoryError> {
        self.with_client(move |c| Ok(c.query_opt("SELECT proposed_via, widens, COALESCE(expires_at < NOW(), false) FROM access_changes WHERE id = $1", &[&id])?.map(|r| (r.get(0), r.get(1), r.get(2)))))
    }

    /// A change by id: `(change, status, proposed_by, reason)`.
    pub fn access_change(&self, id: Uuid) -> Result<Option<(Value, String, String, String)>, RepositoryError> {
        self.with_client(move |c| {
            Ok(c.query_opt("SELECT change, status, proposed_by, reason FROM access_changes WHERE id = $1", &[&id])?
                .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3))))
        })
    }

    pub fn list_access_changes(&self, status: &str) -> Result<Vec<Value>, RepositoryError> {
        let status = status.to_string();
        self.with_client(move |c| {
            let rows = c.query(
                "SELECT id, change, widens, reason, status, proposed_by, to_char(proposed_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
                        proposed_via, drafted_for, via_client, task, to_char(expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
                        COALESCE(expires_at < NOW(), false) \
                 FROM access_changes WHERE status = $1 ORDER BY proposed_at, id",
                &[&status],
            )?;
            Ok(rows
                .iter()
                .map(|r| {
                    let id: Uuid = r.get(0);
                    let change: Value = r.get(1);
                    let kind = change["kind"].as_str().unwrap_or("grant").to_string();
                    json!({ "change_id": id, "kind": kind, "grant": change["grant"].clone(), "hierarchy": change["hierarchy"].clone(), "sentence": change["sentence"].clone(), "rule": change["rule"].clone(), "organization_id": change["organization_id"].clone(), "widens": r.get::<_, bool>(2), "reason": r.get::<_, String>(3), "status": r.get::<_, String>(4), "proposed_by": r.get::<_, String>(5), "proposed_at": r.get::<_, String>(6),
                        "proposed_via": r.get::<_, String>(7), "drafted_for": r.get::<_, Option<String>>(8), "via_client": r.get::<_, Option<String>>(9), "task": r.get::<_, Option<String>>(10),
                        "expires_at": r.get::<_, Option<String>>(11), "expired": r.get::<_, bool>(12) })
                })
                .collect())
        })
    }

    /// Decides a change. The row is locked, so two people deciding at once decide it once. A confirmed grant change
    /// writes the grant in the same transaction. The person who proposed a change cannot confirm it.
    pub fn decide_access_change(&self, id: Uuid, decider: &str, confirm: bool) -> Result<ChangeDecision, RepositoryError> {
        let decider = decider.to_string();
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(row) = tx.query_opt("SELECT change, status, proposed_by, proposed_via, widens, COALESCE(expires_at < NOW(), false) FROM access_changes WHERE id = $1 FOR UPDATE", &[&id])? else {
                return Ok(ChangeDecision::Missing);
            };
            let (change, status, proposed_by): (Value, String, String) = (row.get(0), row.get(1), row.get(2));
            let (via, widens, expired): (String, bool, bool) = (row.get(3), row.get(4), row.get(5));
            if status != "Pending" {
                return Ok(ChangeDecision::AlreadyDecided);
            }
            if confirm && expired {
                return Ok(ChangeDecision::Expired);
            }
            // A change that only narrows access, drafted by an AI tool, may be confirmed by the person it was drafted for.
            let narrowing_draft = via == "agent" && !widens;
            if confirm && proposed_by.eq_ignore_ascii_case(&decider) && !narrowing_draft {
                return Ok(ChangeDecision::OwnChange);
            }
            let mut grant_id = None;
            if confirm && change["kind"] == "hierarchy" {
                let h = &change["hierarchy"];
                let text = |k: &str| h[k].as_str().unwrap_or_default().to_string();
                let key = text("key");
                // The same lock every writer of the tree takes.
                tx.query_one("SELECT pg_advisory_xact_lock($1)", &[&0x5343_4f52_4754_5245_i64])?;
                if tx.query_opt("SELECT 1 FROM hierarchies WHERE key = $1", &[&key])?.is_some() {
                    return Ok(ChangeDecision::Missing);
                }
                tx.execute("INSERT INTO hierarchies (key, name, kind, source, created_by) VALUES ($1, $2, $3, $4, $5)", &[&key, &text("name"), &text("kind"), &text("source"), &proposed_by])?;
                // A hierarchy begins with its root unit, so it can be used the moment it exists.
                let root = Uuid::new_v4();
                tx.execute(
                    "INSERT INTO organizations (id, parent_id, name, code, org_type, hierarchy_key) VALUES ($1, NULL, $2, $3, 'Institution', $4)",
                    &[&root, &text("name"), &format!("ROOT-{}", key.to_uppercase()), &key],
                )?;
                tx.execute("INSERT INTO org_closure (hierarchy_key, ancestor, descendant, depth) VALUES ($1, $2, $2, 0)", &[&key, &root])?;
                crate::cache::bump(&mut tx, "organizations")?;
            } else if confirm && change["kind"] == "revoke" {
                let grant: i64 = change["grant_id"].as_i64().unwrap_or_default();
                if tx.execute("UPDATE access_grants SET revoked_at = NOW(), revoked_by = $2 WHERE id = $1 AND revoked_at IS NULL", &[&grant, &decider])? == 0 {
                    return Ok(ChangeDecision::Missing);
                }
            } else if confirm && change["kind"] == "request_rule" {
                let org: Uuid = change["organization_id"].as_str().and_then(|u| Uuid::parse_str(u).ok()).unwrap_or_default();
                if change["op"] == "retire" {
                    let rule: Uuid = change["rule_id"].as_str().and_then(|u| Uuid::parse_str(u).ok()).unwrap_or_default();
                    if tx.execute("UPDATE access_request_rules SET retired_at = NOW() WHERE id = $1 AND organization_id = $2 AND retired_at IS NULL", &[&rule, &org])? == 0 {
                        return Ok(ChangeDecision::Missing);
                    }
                } else {
                    tx.execute("INSERT INTO access_request_rules (id, organization_id, rule, created_by) VALUES ($1, $2, $3, $4)", &[&Uuid::new_v4(), &org, &change["rule"], &proposed_by])?;
                }
            } else if confirm {
                let g = &change["grant"];
                let text = |k: &str| g[k].as_str().map(str::to_string);
                let depth = g["depth"].as_i64().map(|d| d as i16);
                let valid_until = text("valid_until");
                let reason: String = text("reason").unwrap_or_default();
                let new_id: i64 = tx
                    .query_one(
                        "INSERT INTO access_grants (holder_kind, holder_id, role, hierarchy_key, anchor_kind, anchor_id, depth, domain, clearance, \
                            valid_until, source, reason, created_by) \
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::text::timestamptz, $11, $12, $13) RETURNING id",
                        &[&text("holder_kind"), &text("holder_id"), &text("role"), &text("hierarchy"), &text("anchor_kind"), &text("anchor_id"), &depth, &text("domain"), &text("clearance"), &valid_until, &text("source"), &reason, &proposed_by],
                    )?
                    .get(0);
                grant_id = Some(new_id);
            }
            let new_status = if confirm { "Confirmed" } else { "Rejected" };
            tx.execute("UPDATE access_changes SET status = $2, decided_by = $3, decided_at = NOW(), grant_id = $4 WHERE id = $1", &[&id, &new_status, &decider, &grant_id])?;
            tx.commit()?;
            Ok(ChangeDecision::Done { grant_id })
        })
    }

    /// Revokes a grant. A revoked grant stays in the table.
    pub fn revoke_grant(&self, id: i64, by: &str) -> Result<RevokeOutcome, RepositoryError> {
        let by = by.to_string();
        self.with_client(move |c| {
            let n = c.execute("UPDATE access_grants SET revoked_at = NOW(), revoked_by = $2 WHERE id = $1 AND revoked_at IS NULL", &[&id, &by])?;
            if n == 1 {
                return Ok(RevokeOutcome::Done);
            }
            let exists = c.query_opt("SELECT 1 FROM access_grants WHERE id = $1", &[&id])?.is_some();
            Ok(if exists { RevokeOutcome::AlreadyRevoked } else { RevokeOutcome::Missing })
        })
    }
}
