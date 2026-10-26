//! Access model phase 13: requests for access and the rules that approve some of them alone.
//!
//! An approval is one transaction: the grant (when there is one), the request's new state, and the session it widens
//! (when it is a `self` request). The service has already written the ledger entry.

use crate::repository::{PostgresRepository, RepositoryError};
use serde_json::{json, Value};
use uuid::Uuid;

const TS: &str = "'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'";

/// The id of the rule the migration creates at the institution's root.
pub const DEFAULT_RULE_ID: &str = "00000000-0000-0000-0000-0000000000a1";

#[derive(Debug, Clone)]
pub struct RequestRow {
    pub id: Uuid,
    pub requester: String,
    pub filed_by_client: Option<String>,
    pub session_id: Option<Uuid>,
    pub task: Option<String>,
    pub object_kind: String,
    pub object_id: String,
    pub role: String,
    pub clearance: Option<String>,
    pub reason: String,
    pub days: i16,
    pub route: String,
    pub rule_id: Option<Uuid>,
    pub status: String,
    pub decided_by: Option<String>,
    pub decided_at: Option<String>,
    pub grant_id: Option<i64>,
    pub created_at: String,
    /// The grant's end, when the request has one. A request whose grant has ended reads as `Expired`.
    pub grant_ends: Option<String>,
    pub grant_ended: bool,
}

impl RequestRow {
    /// The status a person reads: an approved request whose grant has ended is `Expired`.
    pub fn shown_status(&self) -> &str {
        if self.status == "Approved" && self.grant_ended {
            "Expired"
        } else {
            &self.status
        }
    }

    pub fn json(&self) -> Value {
        json!({
            "id": self.id, "requester": self.requester, "filed_by_client": self.filed_by_client, "task": self.task,
            "object_kind": self.object_kind, "object_id": self.object_id, "role": self.role, "clearance": self.clearance,
            "reason": self.reason, "days": self.days, "route": self.route, "rule_id": self.rule_id, "status": self.shown_status(),
            "decided_by": self.decided_by, "decided_at": self.decided_at, "grant_id": self.grant_id, "created_at": self.created_at,
            "ends_at": self.grant_ends,
        })
    }
}

#[derive(Debug, Clone)]
pub struct RuleRow {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub rule: Value,
    pub created_by: String,
    pub created_at: String,
    pub retired_at: Option<String>,
}

fn ts(col: &str) -> String {
    format!("to_char({col} AT TIME ZONE 'UTC', {TS})")
}

fn columns() -> String {
    format!(
        "r.id, r.requester, r.filed_by_client, r.session_id, r.task, r.object_kind, r.object_id, r.role, r.clearance, r.reason, r.days, r.route, \
         r.rule_id, r.status, r.decided_by, {}, r.grant_id, {}, {}, COALESCE(g.valid_until <= NOW(), false)",
        ts("r.decided_at"),
        ts("r.created_at"),
        ts("g.valid_until")
    )
}

const FROM: &str = "FROM access_requests r LEFT JOIN access_grants g ON g.id = r.grant_id";

fn row_of(r: &postgres::Row) -> RequestRow {
    RequestRow {
        id: r.get(0),
        requester: r.get(1),
        filed_by_client: r.get(2),
        session_id: r.get(3),
        task: r.get(4),
        object_kind: r.get(5),
        object_id: r.get(6),
        role: r.get(7),
        clearance: r.get(8),
        reason: r.get(9),
        days: r.get(10),
        route: r.get(11),
        rule_id: r.get(12),
        status: r.get(13),
        decided_by: r.get(14),
        decided_at: r.get(15),
        grant_id: r.get(16),
        created_at: r.get(17),
        grant_ends: r.get(18),
        grant_ended: r.get(19),
    }
}

/// A grant a request makes, to be stored with the approval.
pub struct RequestGrant {
    pub holder: String,
    pub role: String,
    pub anchor_id: String,
    pub clearance: Option<String>,
    pub days: i16,
    pub reason: String,
    pub created_by: String,
}

/// What an approval changes besides the request: a grant it makes, or the session it widens to a level.
#[derive(Default)]
pub struct Effects<'a> {
    pub grant: Option<&'a RequestGrant>,
    pub widen: Option<(Uuid, &'a str)>,
}

/// What an approval did.
pub enum Approved {
    Done { grant_id: Option<i64> },
    /// The request was not in a state that allows it.
    Not(String),
    Missing,
}

impl PostgresRepository {
    /// Makes the default rule when the institution's root exists and the rule has never been made.
    pub fn ensure_default_request_rule(&self) -> Result<(), RepositoryError> {
        self.with_client(|c| {
            c.execute(
                "INSERT INTO access_request_rules (id, organization_id, rule, created_by) \
                 SELECT $1::uuid, id, '{\"max_role\":\"viewer\",\"max_sensitivity\":\"Internal\",\"visibilities\":[\"departmental\",\"institutional\"],\"own_unit\":true,\"max_days\":30}'::jsonb, 'migration' \
                 FROM organizations WHERE id = '00000000-0000-0000-0000-000000000001' ON CONFLICT (id) DO NOTHING",
                &[&Uuid::parse_str(DEFAULT_RULE_ID).unwrap_or_default()],
            )?;
            Ok(())
        })
    }

    pub fn request_rules(&self) -> Result<Vec<RuleRow>, RepositoryError> {
        self.with_client(|c| {
            Ok(c.query(&format!("SELECT id, organization_id, rule, created_by, {}, {} FROM access_request_rules ORDER BY created_at, id", ts("created_at"), ts("retired_at")), &[])?
                .iter()
                .map(|r| RuleRow { id: r.get(0), organization_id: r.get(1), rule: r.get(2), created_by: r.get(3), created_at: r.get(4), retired_at: r.get(5) })
                .collect())
        })
    }

    pub fn insert_request(&self, r: &RequestRow) -> Result<(), RepositoryError> {
        let r = r.clone();
        self.with_client(move |c| {
            c.execute(
                "INSERT INTO access_requests (id, requester, filed_by_client, session_id, task, object_kind, object_id, role, clearance, reason, days, route, rule_id, status) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
                &[&r.id, &r.requester, &r.filed_by_client, &r.session_id, &r.task, &r.object_kind, &r.object_id, &r.role, &r.clearance, &r.reason, &r.days, &r.route, &r.rule_id, &r.status],
            )?;
            Ok(())
        })
    }

    pub fn get_request(&self, id: Uuid) -> Result<Option<RequestRow>, RepositoryError> {
        self.with_client(move |c| Ok(c.query_opt(&format!("SELECT {} {FROM} WHERE r.id = $1", columns()), &[&id])?.map(|r| row_of(&r))))
    }

    pub fn requests_of(&self, eppn: &str) -> Result<Vec<RequestRow>, RepositoryError> {
        let eppn = eppn.to_string();
        self.with_client(move |c| Ok(c.query(&format!("SELECT {} {FROM} WHERE lower(r.requester) = lower($1) ORDER BY r.created_at DESC, r.id LIMIT 200", columns()), &[&eppn])?.iter().map(row_of).collect()))
    }

    pub fn pending_requests(&self, limit: i64) -> Result<Vec<RequestRow>, RepositoryError> {
        self.with_client(move |c| Ok(c.query(&format!("SELECT {} {FROM} WHERE r.status = 'Pending' ORDER BY r.created_at, r.id LIMIT $1", columns()), &[&limit])?.iter().map(row_of).collect()))
    }

    /// The client of an AI tool session, and the task it named.
    pub fn session_client_and_task(&self, session: Uuid) -> Result<Option<(String, Option<String>)>, RepositoryError> {
        self.with_client(move |c| Ok(c.query_opt("SELECT client_id, task FROM oauth_sessions WHERE session_id = $1", &[&session])?.map(|r| (r.get(0), r.get(1)))))
    }

    /// The person confirms a request an AI tool filed. It leaves `Waiting for person` with the route the server has now
    /// worked out. False when it was not waiting.
    pub fn confirm_request(&self, id: Uuid, route: &str, status: &str) -> Result<bool, RepositoryError> {
        let (route, status) = (route.to_string(), status.to_string());
        self.with_client(move |c| Ok(c.execute("UPDATE access_requests SET route = $2, status = $3 WHERE id = $1 AND status = 'Waiting for person'", &[&id, &route, &status])? == 1))
    }

    /// Ends a request that is not approved. `from` names the states it may leave.
    pub fn close_request(&self, id: Uuid, from: &[&str], to: &str, by: &str) -> Result<bool, RepositoryError> {
        let (from, to, by): (Vec<String>, String, String) = (from.iter().map(|s| s.to_string()).collect(), to.to_string(), by.to_string());
        self.with_client(move |c| Ok(c.execute("UPDATE access_requests SET status = $3, decided_by = $4, decided_at = NOW() WHERE id = $1 AND status = ANY($2)", &[&id, &from, &to, &by])? == 1))
    }

    /// Approves a request in one transaction. With a grant, the grant is stored and named. With a session, that session is
    /// widened to `level`: its record and every token of it. Nothing else is.
    pub fn approve_request(&self, id: Uuid, from: &[&str], route: &str, rule: Option<Uuid>, by: &str, effects: Effects<'_>) -> Result<Approved, RepositoryError> {
        let (from, route, by): (Vec<String>, String, String) = (from.iter().map(|s| s.to_string()).collect(), route.to_string(), by.to_string());
        let grant = effects.grant.map(|g| (g.holder.clone(), g.role.clone(), g.anchor_id.clone(), g.clearance.clone(), g.days, g.reason.clone(), g.created_by.clone()));
        let widen = effects.widen.map(|(s, l)| (s, l.to_string()));
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(row) = tx.query_opt("SELECT status FROM access_requests WHERE id = $1 FOR UPDATE", &[&id])? else { return Ok(Approved::Missing) };
            let status: String = row.get(0);
            if !from.contains(&status) {
                return Ok(Approved::Not(status));
            }
            let mut grant_id = None;
            if let Some((holder, role, anchor, clearance, days, reason, created_by)) = &grant {
                let new_id: i64 = tx
                    .query_one(
                        "INSERT INTO access_grants (holder_kind, holder_id, role, anchor_kind, anchor_id, clearance, valid_until, source, reason, created_by) \
                         VALUES ('person', $1, $2, 'container', $3, $4, NOW() + make_interval(days => $5::int), 'request', $6, $7) RETURNING id",
                        &[holder, role, anchor, clearance, &i32::from(*days), reason, created_by],
                    )?
                    .get(0);
                grant_id = Some(new_id);
            }
            if let Some((session, level)) = &widen {
                tx.execute("UPDATE oauth_sessions SET access = $2 WHERE session_id = $1 AND revoked_at IS NULL", &[session, level])?;
                tx.execute(
                    "UPDATE api_tokens SET scope = jsonb_set(scope, '{level}', to_jsonb($2::text)) \
                     WHERE scope IS NOT NULL AND jsonb_exists(scope, 'level') AND token_hash IN (SELECT token_hash FROM oauth_tokens WHERE session_id = $1)",
                    &[session, level],
                )?;
            }
            tx.execute(
                "UPDATE access_requests SET status = 'Approved', route = $2, rule_id = $3, decided_by = $4, decided_at = NOW(), grant_id = $5 WHERE id = $1",
                &[&id, &route, &rule, &by, &grant_id],
            )?;
            tx.commit()?;
            Ok(Approved::Done { grant_id })
        })
    }
}
