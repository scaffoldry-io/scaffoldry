//! Policy library phase 1: the rows. Every function here runs inside a service function that has already written the
//! ledger entry, so a row never exists without its entry.

use crate::repository::{PostgresRepository, RepositoryError};
use serde_json::{json, Value};
use uuid::Uuid;

const TS: &str = "'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'";

fn ts(col: &str) -> String {
    format!("to_char({col} AT TIME ZONE 'UTC', {TS})")
}

fn policy_of(r: &postgres::Row) -> Value {
    json!({
        "id": r.get::<_, Uuid>(0), "key": r.get::<_, String>(1), "title": r.get::<_, String>(2), "summary": r.get::<_, String>(3),
        "organization_id": r.get::<_, Uuid>(4), "owner": r.get::<_, Value>(5), "review_every_days": r.get::<_, i32>(6),
        "last_reviewed_at": r.get::<_, Option<String>>(7), "status": r.get::<_, String>(8), "created_by": r.get::<_, String>(9), "created_at": r.get::<_, String>(10),
    })
}

fn policy_select() -> String {
    format!("SELECT id, key, title, summary, organization_id, owner, review_every_days, {}, status, created_by, {} FROM library_policies", ts("last_reviewed_at"), ts("created_at"))
}

/// A new policy.
pub struct NewPolicy {
    pub key: String,
    pub title: String,
    pub summary: String,
    pub organization_id: Uuid,
    pub owner: Value,
    pub review_every_days: i32,
}

/// One citation of a new rule, already checked against its snapshot.
pub struct NewCitation {
    pub snapshot_id: Uuid,
    pub passage: String,
    pub locator: String,
}

impl PostgresRepository {
    pub fn library_create_policy(&self, p: NewPolicy, by: &str) -> Result<Value, RepositoryError> {
        let by = by.to_string();
        self.with_client(move |c| {
            let id = Uuid::new_v4();
            c.execute(
                "INSERT INTO library_policies (id, key, title, summary, organization_id, owner, review_every_days, created_by) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[&id, &p.key, &p.title, &p.summary, &p.organization_id, &p.owner, &p.review_every_days, &by],
            )?;
            Ok(policy_of(&c.query_one(&format!("{} WHERE id = $1", policy_select()), &[&id])?))
        })
    }

    pub fn library_policy(&self, id: Uuid) -> Result<Option<Value>, RepositoryError> {
        self.with_client(move |c| {
            Ok(
                c.query_opt(&format!("{} WHERE id = $1", policy_select()), &[&id])?
                    .as_ref()
                    .map(policy_of),
            )
        })
    }

    pub fn library_policy_key_exists(&self, key: &str) -> Result<bool, RepositoryError> {
        let key = key.to_string();
        self.with_client(move |c| {
            Ok(
                c.query_opt("SELECT 1 FROM library_policies WHERE key = $1", &[&key])?
                    .is_some(),
            )
        })
    }

    pub fn library_policies(&self) -> Result<Vec<Value>, RepositoryError> {
        self.with_client(|c| {
            Ok(
                c.query(&format!("{} ORDER BY title, key", policy_select()), &[])?
                    .iter()
                    .map(policy_of)
                    .collect(),
            )
        })
    }

    /// Changes the fields a patch names. `patch` holds only fields already checked.
    pub fn library_update_policy(
        &self,
        id: Uuid,
        patch: &Value,
    ) -> Result<Option<Value>, RepositoryError> {
        let patch = patch.clone();
        self.with_client(move |c| {
            let text = |k: &str| patch.get(k).and_then(Value::as_str).map(str::to_string);
            let days = patch.get("review_every_days").and_then(Value::as_i64).map(|d| d as i32);
            let owner = patch.get("owner").cloned();
            c.execute(
                "UPDATE library_policies SET title = COALESCE($2, title), summary = COALESCE($3, summary), owner = COALESCE($4, owner), \
                 review_every_days = COALESCE($5, review_every_days), status = COALESCE($6, status) WHERE id = $1",
                &[&id, &text("title"), &text("summary"), &owner, &days, &text("status")],
            )?;
            Ok(c.query_opt(&format!("{} WHERE id = $1", policy_select()), &[&id])?.as_ref().map(policy_of))
        })
    }

    pub fn library_source_count(&self, policy: Uuid) -> Result<i64, RepositoryError> {
        self.with_client(move |c| {
            Ok(c.query_one(
                "SELECT COUNT(*) FROM library_sources WHERE policy_id = $1",
                &[&policy],
            )?
            .get(0))
        })
    }

    pub fn library_add_source(
        &self,
        policy: Uuid,
        title: &str,
        kind: &str,
        href: Option<&str>,
        by: &str,
    ) -> Result<Value, RepositoryError> {
        let (title, kind, href, by) = (
            title.to_string(),
            kind.to_string(),
            href.map(str::to_string),
            by.to_string(),
        );
        self.with_client(move |c| {
            let id = Uuid::new_v4();
            c.execute("INSERT INTO library_sources (id, policy_id, title, kind, href, created_by) VALUES ($1, $2, $3, $4, $5, $6)", &[&id, &policy, &title, &kind, &href, &by])?;
            Ok(json!({ "id": id, "policy_id": policy, "title": title, "kind": kind, "href": href, "created_by": by }))
        })
    }

    /// A source: `(policy id, kind)`.
    pub fn library_source(&self, id: Uuid) -> Result<Option<(Uuid, String)>, RepositoryError> {
        self.with_client(move |c| {
            Ok(c.query_opt(
                "SELECT policy_id, kind FROM library_sources WHERE id = $1",
                &[&id],
            )?
            .map(|r| (r.get(0), r.get(1))))
        })
    }

    /// The sources of a policy, each with its latest snapshot and a plain state.
    pub fn library_sources_of(&self, policy: Uuid) -> Result<Vec<Value>, RepositoryError> {
        self.with_client(move |c| {
            let rows = c.query(
                &format!(
                    "SELECT s.id, s.title, s.kind, s.href, n.id, {}, n.text_origin, n.text_sha256 FROM library_sources s \
                     LEFT JOIN LATERAL (SELECT id, taken_at, text_origin, text_sha256 FROM library_snapshots WHERE source_id = s.id ORDER BY taken_at DESC, id LIMIT 1) n ON TRUE \
                     WHERE s.policy_id = $1 ORDER BY s.created_at, s.id",
                    ts("n.taken_at")
                ),
                &[&policy],
            )?;
            Ok(rows
                .iter()
                .map(|r| {
                    let snapshot: Option<Uuid> = r.get(4);
                    json!({
                        "id": r.get::<_, Uuid>(0), "title": r.get::<_, String>(1), "kind": r.get::<_, String>(2), "href": r.get::<_, Option<String>>(3),
                        "state": if snapshot.is_some() { "Stored" } else if r.get::<_, String>(2) == "url" { "Not yet fetched" } else { "No text yet" },
                        "latest_snapshot": snapshot.map(|id| json!({ "id": id, "taken_at": r.get::<_, Option<String>>(5), "text_origin": r.get::<_, Option<String>>(6), "text_sha256": r.get::<_, Option<String>>(7) })),
                    })
                })
                .collect())
        })
    }

    /// The latest snapshot of a source: `(id, text_sha256)`.
    pub fn library_latest_snapshot(
        &self,
        source: Uuid,
    ) -> Result<Option<(Uuid, String)>, RepositoryError> {
        self.with_client(move |c| Ok(c.query_opt("SELECT id, text_sha256 FROM library_snapshots WHERE source_id = $1 ORDER BY taken_at DESC, id LIMIT 1", &[&source])?.map(|r| (r.get(0), r.get::<_, String>(1).trim().to_string()))))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn library_add_snapshot(
        &self,
        source: Uuid,
        media_type: &str,
        content_sha256: &str,
        text: &str,
        text_sha256: &str,
        origin: &str,
        by: &str,
    ) -> Result<Value, RepositoryError> {
        let (media_type, content_sha256, text, text_sha256, origin, by) = (
            media_type.to_string(),
            content_sha256.to_string(),
            text.to_string(),
            text_sha256.to_string(),
            origin.to_string(),
            by.to_string(),
        );
        self.with_client(move |c| {
            let id = Uuid::new_v4();
            c.execute(
                "INSERT INTO library_snapshots (id, source_id, media_type, content_sha256, text, text_sha256, text_origin, taken_by) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[&id, &source, &media_type, &content_sha256, &text, &text_sha256, &origin, &by],
            )?;
            Ok(json!({ "id": id, "source_id": source, "media_type": media_type, "text_sha256": text_sha256, "text_origin": origin, "taken_by": by }))
        })
    }

    /// A snapshot with its text and the policy it belongs to.
    pub fn library_snapshot(&self, id: Uuid) -> Result<Option<(Uuid, Value)>, RepositoryError> {
        self.with_client(move |c| {
            Ok(c.query_opt(
                &format!(
                    "SELECT s.policy_id, n.id, n.source_id, {}, n.media_type, n.content_sha256, n.text, n.text_sha256, n.text_origin, n.taken_by, s.title FROM library_snapshots n JOIN library_sources s ON s.id = n.source_id WHERE n.id = $1",
                    ts("n.taken_at")
                ),
                &[&id],
            )?
            .map(|r| {
                (
                    r.get(0),
                    json!({ "id": r.get::<_, Uuid>(1), "source_id": r.get::<_, Uuid>(2), "source_title": r.get::<_, String>(10), "taken_at": r.get::<_, String>(3), "media_type": r.get::<_, String>(4),
                            "content_sha256": r.get::<_, String>(5).trim(), "text": r.get::<_, String>(6), "text_sha256": r.get::<_, String>(7).trim(), "text_origin": r.get::<_, String>(8), "taken_by": r.get::<_, String>(9) }),
                )
            }))
        })
    }

    pub fn library_rule_counts(&self, policy: Uuid) -> Result<Value, RepositoryError> {
        self.with_client(move |c| {
            let rows = c.query(
                "SELECT status, COUNT(*) FROM library_rules WHERE policy_id = $1 GROUP BY status",
                &[&policy],
            )?;
            Ok(Value::Object(
                rows.iter()
                    .map(|r| (r.get::<_, String>(0), json!(r.get::<_, i64>(1))))
                    .collect(),
            ))
        })
    }

    pub fn library_rule_key_count(&self, policy: Uuid) -> Result<i64, RepositoryError> {
        self.with_client(move |c| {
            Ok(c.query_one(
                "SELECT COUNT(DISTINCT key) FROM library_rules WHERE policy_id = $1",
                &[&policy],
            )?
            .get(0))
        })
    }

    pub fn library_open_questions(&self, policy: Uuid) -> Result<Vec<Value>, RepositoryError> {
        self.with_client(move |c| {
            let rows = c.query(
                &format!("SELECT id, question, context_passage, snapshot_id, asked_by, asked_via, {} FROM library_questions WHERE policy_id = $1 AND answer IS NULL ORDER BY asked_at, id", ts("asked_at")),
                &[&policy],
            )?;
            Ok(rows
                .iter()
                .map(|r| json!({ "id": r.get::<_, Uuid>(0), "question": r.get::<_, String>(1), "context_passage": r.get::<_, String>(2), "snapshot_id": r.get::<_, Option<Uuid>>(3), "asked_by": r.get::<_, String>(4), "asked_via": r.get::<_, String>(5), "asked_at": r.get::<_, String>(6) }))
                .collect())
        })
    }

    /// Stores a rule as the next version of its key, with its citations, as a draft. One transaction.
    #[allow(clippy::too_many_arguments)]
    pub fn library_draft_rule(
        &self,
        policy: Uuid,
        key: &str,
        statement: &str,
        applies_to: &str,
        by: &str,
        via: &str,
        citations: Vec<NewCitation>,
    ) -> Result<Value, RepositoryError> {
        let (key, statement, applies_to, by, via) = (
            key.to_string(),
            statement.to_string(),
            applies_to.to_string(),
            by.to_string(),
            via.to_string(),
        );
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            // Two drafts of one key at once take their versions in turn.
            tx.query_one("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))", &[&format!("library-rule:{policy}:{key}")])?;
            let version: i32 = tx.query_one("SELECT COALESCE(MAX(version), 0) + 1 FROM library_rules WHERE policy_id = $1 AND key = $2", &[&policy, &key])?.get(0);
            let id = Uuid::new_v4();
            tx.execute(
                "INSERT INTO library_rules (id, policy_id, key, version, statement, applies_to, status, submitted_by, submitted_via) VALUES ($1, $2, $3, $4, $5, $6, 'Draft', $7, $8)",
                &[&id, &policy, &key, &version, &statement, &applies_to, &by, &via],
            )?;
            for cite in &citations {
                tx.execute("INSERT INTO library_citations (id, rule_id, snapshot_id, passage, locator) VALUES ($1, $2, $3, $4, $5)", &[&Uuid::new_v4(), &id, &cite.snapshot_id, &cite.passage, &cite.locator])?;
            }
            tx.commit()?;
            Ok(json!({ "id": id, "policy_id": policy, "key": key, "version": version, "statement": statement, "applies_to": applies_to, "status": "Draft", "submitted_by": by, "submitted_via": via,
                       "citations": citations.iter().map(|c| json!({ "snapshot_id": c.snapshot_id, "passage": c.passage, "locator": c.locator })).collect::<Vec<_>>() }))
        })
    }

    pub fn library_add_question(
        &self,
        policy: Uuid,
        question: &str,
        context_passage: &str,
        snapshot: Option<Uuid>,
        by: &str,
        via: &str,
    ) -> Result<Value, RepositoryError> {
        let (question, context_passage, by, via) = (
            question.to_string(),
            context_passage.to_string(),
            by.to_string(),
            via.to_string(),
        );
        self.with_client(move |c| {
            let id = Uuid::new_v4();
            c.execute(
                "INSERT INTO library_questions (id, policy_id, question, context_passage, snapshot_id, asked_by, asked_via) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                &[&id, &policy, &question, &context_passage, &snapshot, &by, &via],
            )?;
            Ok(json!({ "id": id, "policy_id": policy, "question": question, "asked_by": by, "asked_via": via }))
        })
    }

    /// Every open question, with the policy it belongs to.
    pub fn library_open_questions_all(&self) -> Result<Vec<Value>, RepositoryError> {
        self.with_client(|c| {
            let rows = c.query(
                &format!("SELECT q.id, q.policy_id, p.title, q.question, q.context_passage, q.snapshot_id, q.asked_by, q.asked_via, {} FROM library_questions q JOIN library_policies p ON p.id = q.policy_id WHERE q.answer IS NULL ORDER BY q.asked_at, q.id", ts("q.asked_at")),
                &[],
            )?;
            Ok(rows
                .iter()
                .map(|r| json!({ "id": r.get::<_, Uuid>(0), "policy_id": r.get::<_, Uuid>(1), "policy_title": r.get::<_, String>(2), "question": r.get::<_, String>(3), "context_passage": r.get::<_, String>(4), "snapshot_id": r.get::<_, Option<Uuid>>(5), "asked_by": r.get::<_, String>(6), "asked_via": r.get::<_, String>(7), "asked_at": r.get::<_, String>(8) }))
                .collect())
        })
    }

    /// Published rules whose statement or `applies_to` contains the query, and whose `applies_to` contains the second filter.
    /// A rule in any other status is never returned.
    pub fn library_published_rules(
        &self,
        query: &str,
        applies_to: &str,
    ) -> Result<Vec<Value>, RepositoryError> {
        let like = |t: &str| {
            format!(
                "%{}%",
                t.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
            )
        };
        let (query, applies_to) = (like(query), like(applies_to));
        self.with_client(move |c| {
            let rows = c.query(
                "SELECT r.id, r.key, r.statement, r.applies_to, p.id, p.key, p.title FROM library_rules r JOIN library_policies p ON p.id = r.policy_id WHERE r.status = 'Published' AND (r.statement ILIKE $1 OR r.applies_to ILIKE $1) AND r.applies_to ILIKE $2 ORDER BY p.title, r.key LIMIT 50",
                &[&query, &applies_to],
            )?;
            Ok(rows
                .iter()
                .map(|r| json!({ "id": r.get::<_, Uuid>(0), "key": r.get::<_, String>(1), "statement": r.get::<_, String>(2), "applies_to": r.get::<_, String>(3), "policy_id": r.get::<_, Uuid>(4), "policy_key": r.get::<_, String>(5), "policy_title": r.get::<_, String>(6) }))
                .collect())
        })
    }
}
