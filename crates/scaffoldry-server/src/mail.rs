//! Notifications phase 2: email.
//!
//! A notice that should be mailed adds a row to `email_outbox` in the transaction that makes the notice (`queue`). The
//! `send_email` job sends the due rows every 15 seconds (`send_due`) through `net::smtp`, which reaches only a host on
//! `egress.allow`. Nothing is queued or sent while `mail.enabled` is false.
//!
//! A mail is plain text: the notice's title, its body, and `Open it: {platform.base_url}/n/{id}`. Following the link
//! needs a session. There is no field value in it, no token, and no attachment.
//!
//! Limits: 50 mails to one address an hour, after which a notice stays in the inbox and waits for the digest, and 600
//! a minute for the whole server. A failure is tried again after 1 minute, 10 minutes, 1 hour and 6 hours, and then the
//! row is `failed`. The relay's password is read from the secrets store when a run starts and is never logged.

use crate::net::egress::{Allow, Resolver};
use crate::net::smtp::{Relay, Security, Session};
use crate::repository::{PostgresRepository, RepositoryError};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

/// How long a failed row waits before each next try. After the last, the row is `failed`.
pub const RETRY_AFTER_SECS: [i64; 4] = [60, 600, 3_600, 21_600];
/// Tries in all: the first and one after each wait.
pub const MAX_ATTEMPTS: i32 = RETRY_AFTER_SECS.len() as i32 + 1;
/// Mails to one address in an hour. Past this, a notice waits for the digest.
pub const PER_RECIPIENT_PER_HOUR: i64 = 50;
/// Mails the server sends in a minute.
pub const PER_MINUTE: i64 = 600;
/// Rows one run claims.
pub const BATCH: i64 = 20;
/// How long a claimed row is held by the run that claimed it. A run that stops part way leaves its rows to the next.
const CLAIM_SECS: f64 = 300.0;

/// The relay, from the setting `mail.smtp`.
#[derive(Debug, Clone)]
pub struct Smtp {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub from: lettre::message::Mailbox,
    pub username: Option<String>,
    /// The name of the secret that holds the password.
    pub secret: Option<String>,
}

impl Smtp {
    /// Reads and checks `{ host, port, security: "starttls" | "tls", from, username, secret }`. A username needs a
    /// secret, and a secret a username.
    pub fn parse(value: &Value) -> Result<Smtp, String> {
        let obj = value.as_object().ok_or("mail.smtp must be an object")?;
        if let Some(k) = obj.keys().find(|k| !["host", "port", "security", "from", "username", "secret"].contains(&k.as_str())) {
            return Err(format!("mail.smtp has no field {k}"));
        }
        let text = |k: &str| obj.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
        let host = text("host").ok_or("mail.smtp needs a host")?.to_ascii_lowercase();
        if host.len() > 253 || host.contains(|c: char| c.is_whitespace() || c == '/' || c == '@' || c == ':') {
            return Err("mail.smtp.host must be a host name".to_string());
        }
        let port = obj.get("port").and_then(Value::as_u64).filter(|p| (1..=65535).contains(p)).ok_or("mail.smtp.port must be from 1 to 65535")? as u16;
        let security = match text("security") {
            Some("tls") => Security::Tls,
            Some("starttls") => Security::StartTls,
            _ => return Err("mail.smtp.security must be starttls or tls. Mail is never sent in the clear".to_string()),
        };
        let from = text("from").and_then(|f| f.parse::<lettre::message::Mailbox>().ok()).ok_or("mail.smtp.from must be an email address")?;
        let (username, secret) = (text("username").map(str::to_string), text("secret").map(str::to_string));
        if username.is_some() != secret.is_some() {
            return Err("mail.smtp needs both a username and a secret, or neither".to_string());
        }
        if secret.as_deref().is_some_and(|s| !crate::secrets::valid_name(s)) {
            return Err("mail.smtp.secret must name a secret".to_string());
        }
        Ok(Smtp { host, port, security, from, username, secret })
    }
}

/// Checks `platform.base_url`: an https address with no query, fragment or user part. `http` is taken only for
/// `localhost`, for a desk on a developer's machine.
pub fn valid_base_url(value: &Value) -> bool {
    let Some(s) = value.as_str() else { return false };
    let rest = match (s.strip_prefix("https://"), s.strip_prefix("http://")) {
        (Some(r), _) => r,
        (None, Some(r)) if r.split(['/', ':']).next() == Some("localhost") => r,
        _ => return false,
    };
    let host = rest.split('/').next().unwrap_or("");
    s.len() <= 255 && !host.is_empty() && !rest.contains(|c: char| c.is_whitespace() || c == '?' || c == '#' || c == '@' || c == '\\')
}

/// Whether mail is on and complete. The answer is the address links start with.
pub fn mailing(settings: &HashMap<String, Value>) -> Result<String, String> {
    if settings.get("mail.enabled").and_then(Value::as_bool) != Some(true) {
        return Err("Mail is off. Set mail.enabled to send it".to_string());
    }
    Smtp::parse(settings.get("mail.smtp").unwrap_or(&Value::Null))?;
    settings
        .get("platform.base_url")
        .filter(|v| valid_base_url(v))
        .and_then(Value::as_str)
        .map(|s| s.trim_end_matches('/').to_string())
        .ok_or_else(|| "Mail needs platform.base_url, the address links in mail start with".to_string())
}

/// A notice's mail text: its title, its body, and the link.
pub fn compose(title: &str, body: &str, base_url: &str, notification: Uuid) -> String {
    format!("{title}\n\n{body}\n\nOpen it: {base_url}/n/{notification}\n")
}

/// Queues one mail in the caller's transaction, or holds it for the digest when the address has had its hourly
/// share. A held notice is marked `digest`. Returns whether a mail was queued.
pub fn queue(tx: &mut postgres::Transaction<'_>, notification: Option<Uuid>, to: &str, subject: &str, body: &str) -> Result<bool, postgres::Error> {
    // One writer at a time per address, so two notices at once cannot both take the 50th place.
    tx.execute("SELECT pg_advisory_xact_lock(hashtext(lower($1)))", &[&to])?;
    let recent: i64 = tx
        .query_one("SELECT count(*) FROM email_outbox WHERE lower(to_address) = lower($1) AND created_at > NOW() - INTERVAL '1 hour'", &[&to])?
        .get(0);
    let queued = recent < PER_RECIPIENT_PER_HOUR;
    if queued {
        tx.execute(
            "INSERT INTO email_outbox (id, to_address, subject, body, notification_id) VALUES ($1, $2, $3, $4, $5)",
            &[&Uuid::new_v4(), &to, &subject, &body, &notification],
        )?;
    }
    if let Some(id) = notification {
        let state = if queued { "queued" } else { "digest" };
        tx.execute("UPDATE notifications SET email_state = $2 WHERE id = $1", &[&id, &state])?;
    }
    Ok(queued)
}

/// The address mail for a person goes to: their directory address, else their sign-in name when it is an address.
pub fn address_of(c: &mut postgres::Transaction<'_>, eppn: &str, scim_email: Option<&str>) -> Result<Option<String>, postgres::Error> {
    if let Some(e) = scim_email.filter(|e| e.contains('@')) {
        return Ok(Some(e.to_string()));
    }
    let person: Option<String> = c.query_opt("SELECT email FROM persons WHERE lower(eppn) = lower($1)", &[&eppn])?.map(|r| r.get(0));
    Ok(person.or_else(|| eppn.contains('@').then(|| eppn.to_string())).filter(|a| a.len() <= 320))
}

struct Claimed {
    id: Uuid,
    to: String,
    subject: String,
    body: String,
    attempts: i32,
    notification: Option<Uuid>,
}

fn storage(e: RepositoryError) -> String {
    format!("The database could not be reached: {e}")
}

/// Claims up to 20 due rows, within the server's minute budget. Each is held for five minutes, so another server
/// skips it, and a run that stops part way leaves it to the next.
fn claim(repo: &PostgresRepository) -> Result<Vec<Claimed>, String> {
    repo.with_client(|c| {
        let mut tx = c.transaction()?;
        let sent: i64 = tx.query_one("SELECT count(*) FROM email_outbox WHERE sent_at > NOW() - INTERVAL '1 minute'", &[])?.get(0);
        let room = BATCH.min(PER_MINUTE - sent);
        if room <= 0 {
            return Ok(Vec::new());
        }
        let rows = tx.query(
            "UPDATE email_outbox SET next_attempt_at = NOW() + make_interval(secs => $2) WHERE id IN ( \
                SELECT id FROM email_outbox WHERE state = 'queued' AND next_attempt_at <= NOW() \
                ORDER BY next_attempt_at LIMIT $1 FOR UPDATE SKIP LOCKED) \
             RETURNING id, to_address, subject, body, attempts, notification_id",
            &[&room, &CLAIM_SECS],
        )?;
        tx.commit()?;
        Ok(rows.iter().map(|r| Claimed { id: r.get(0), to: r.get(1), subject: r.get(2), body: r.get(3), attempts: r.get(4), notification: r.get(5) }).collect())
    })
    .map_err(storage)
}

enum Outcome {
    Sent,
    Retry(String),
    Failed(String),
}

fn record(repo: &PostgresRepository, results: Vec<(Uuid, Option<Uuid>, i32, Outcome)>) -> Result<(), String> {
    repo.with_client(move |c| {
        let mut tx = c.transaction()?;
        for (id, notification, attempts, outcome) in &results {
            let tries = attempts + 1;
            let (state, error, wait) = match outcome {
                Outcome::Sent => ("sent", None, 0.0),
                Outcome::Retry(e) if tries < MAX_ATTEMPTS => ("queued", Some(e.as_str()), RETRY_AFTER_SECS[(tries - 1) as usize] as f64),
                Outcome::Retry(e) | Outcome::Failed(e) => ("failed", Some(e.as_str()), 0.0),
            };
            tx.execute(
                "UPDATE email_outbox SET state = $2::varchar, attempts = $3, last_error = $4, \
                 next_attempt_at = CASE WHEN $2::varchar = 'queued' THEN NOW() + make_interval(secs => $5) ELSE next_attempt_at END, \
                 sent_at = CASE WHEN $2::varchar = 'sent' THEN NOW() ELSE sent_at END WHERE id = $1",
                &[id, &state, &tries, &error, &wait],
            )?;
            if state != "queued" {
                tx.execute("UPDATE notifications SET email_state = $2 WHERE id = $1", &[notification, &state])?;
            }
        }
        tx.commit()?;
        Ok(())
    })
    .map_err(storage)
}

/// One run of `send_email`: sends what is due and records each result. Returns how many were sent, will be tried
/// again, and failed for good. The report never holds an address or a password.
pub fn send_due(repo: &PostgresRepository, key_file: Option<&Path>, resolver: &dyn Resolver) -> Result<Value, String> {
    let settings = repo.get_platform_settings().map_err(storage)?;
    if let Err(why) = mailing(&settings) {
        return Ok(json!({ "sent": 0, "retried": 0, "failed": 0, "idle": why }));
    }
    let smtp = Smtp::parse(settings.get("mail.smtp").unwrap_or(&Value::Null))?;
    let claimed = claim(repo)?;
    if claimed.is_empty() {
        return Ok(json!({ "sent": 0, "retried": 0, "failed": 0 }));
    }
    let allow = Allow::from_settings(&settings);
    let password = smtp.secret.as_ref().map(|name| crate::secrets::get_in(repo, key_file, name).map_err(|e| format!("The mail password could not be read: {e}")));
    let hello = smtp.from.email.domain().to_string();
    let mut session: Option<Session> = None;
    // The reason the relay cannot be reached in this run, once known, for every row after.
    let mut unreachable: Option<String> = None;
    let mut results = Vec::with_capacity(claimed.len());
    for row in &claimed {
        let message = lettre::Message::builder()
            .from(smtp.from.clone())
            .to(match row.to.parse() {
                Ok(m) => m,
                Err(_) => {
                    results.push((row.id, row.notification, row.attempts, Outcome::Failed("The address is not an email address".to_string())));
                    continue;
                }
            })
            .subject(row.subject.clone())
            .header(lettre::message::header::ContentType::TEXT_PLAIN)
            .body(row.body.clone());
        let message = match message {
            Ok(m) => m,
            Err(_) => {
                results.push((row.id, row.notification, row.attempts, Outcome::Failed("The mail could not be built".to_string())));
                continue;
            }
        };
        if let Some(why) = &unreachable {
            results.push((row.id, row.notification, row.attempts, Outcome::Retry(why.clone())));
            continue;
        }
        if session.is_none() {
            let opened = match &password {
                Some(Err(e)) => Err(e.clone()),
                Some(Ok(p)) => Session::open(&relay(&smtp, &hello, Some(p.expose())), &allow, resolver),
                None => Session::open(&relay(&smtp, &hello, None), &allow, resolver),
            };
            match opened {
                Ok(s) => session = Some(s),
                Err(why) => {
                    unreachable = Some(why.clone());
                    results.push((row.id, row.notification, row.attempts, Outcome::Retry(why)));
                    continue;
                }
            }
        }
        let sent = session.as_mut().map(|s| s.send(&message)).unwrap_or_else(|| Err("No connection".to_string()));
        match sent {
            Ok(()) => results.push((row.id, row.notification, row.attempts, Outcome::Sent)),
            Err(why) => {
                // A refused message may leave the conversation in any state. The next one starts a new one.
                session = None;
                results.push((row.id, row.notification, row.attempts, Outcome::Retry(why)));
            }
        }
    }
    drop(session);
    let count = |f: fn(&Outcome, i32) -> bool| results.iter().filter(|(_, _, a, o)| f(o, *a)).count();
    let sent = count(|o, _| matches!(o, Outcome::Sent));
    let retried = count(|o, a| matches!(o, Outcome::Retry(_)) && a + 1 < MAX_ATTEMPTS);
    let failed = results.len() - sent - retried;
    record(repo, results)?;
    Ok(json!({ "sent": sent, "retried": retried, "failed": failed }))
}

fn relay<'a>(smtp: &'a Smtp, hello: &'a str, password: Option<&'a [u8]>) -> Relay<'a> {
    Relay {
        host: &smtp.host,
        port: smtp.port,
        security: smtp.security,
        hello,
        credentials: smtp.username.as_deref().zip(password),
    }
}

/// The `send_email` job, every 15 seconds.
pub fn send_email_job(ctx: &crate::jobs::JobCtx, _payload: Value) -> Result<Value, crate::jobs::JobError> {
    let key = crate::secrets::key_file_from_env();
    send_due(ctx.repository(), key.as_deref(), &crate::net::egress::SystemResolver).map_err(crate::jobs::JobError::Retry)
}

/// The domain of an address, for showing where mail went without saying to whom.
pub fn domain_of(address: &str) -> String {
    address.rsplit_once('@').map(|(_, d)| d.to_ascii_lowercase()).unwrap_or_default()
}

/// What `GET /admin/mail` shows: whether mail is on, the queue, and the last failures by the recipient's domain.
pub fn status(repo: &PostgresRepository) -> Result<Value, RepositoryError> {
    let settings = repo.get_platform_settings()?;
    let enabled = settings.get("mail.enabled").and_then(Value::as_bool) == Some(true);
    let problem = mailing(&settings).err().filter(|_| enabled);
    let relay = settings.get("mail.smtp").and_then(|v| Smtp::parse(v).ok()).map(|s| {
        json!({ "host": s.host, "port": s.port, "security": if s.security == Security::Tls { "tls" } else { "starttls" } })
    });
    repo.with_client(move |c| {
        let q = c.query_one(
            "SELECT count(*) FILTER (WHERE state = 'queued'), min(created_at) FILTER (WHERE state = 'queued'), \
                    count(*) FILTER (WHERE sent_at > NOW() - INTERVAL '1 hour'), count(*) FILTER (WHERE state = 'failed') \
             FROM email_outbox",
            &[],
        )?;
        let oldest: Option<chrono::DateTime<chrono::Utc>> = q.get(1);
        let failures: Vec<Value> = c
            .query("SELECT to_address, last_error, attempts, created_at FROM email_outbox WHERE state = 'failed' ORDER BY created_at DESC LIMIT 20", &[])?
            .iter()
            .map(|r| {
                let at: chrono::DateTime<chrono::Utc> = r.get(3);
                json!({ "domain": domain_of(&r.get::<_, String>(0)), "error": r.get::<_, Option<String>>(1), "attempts": r.get::<_, i32>(2), "created_at": at.to_rfc3339() })
            })
            .collect();
        Ok(json!({
            "enabled": enabled,
            "problem": problem,
            "relay": relay,
            "queued": q.get::<_, i64>(0),
            "oldest_queued_at": oldest.map(|t| t.to_rfc3339()),
            "sent_last_hour": q.get::<_, i64>(2),
            "failed": q.get::<_, i64>(3),
            "failures": failures,
        }))
    })
}

/// A notice waiting for a digest: id, recipient, app slug, app title, notice title, notice body.
type Pending = (Uuid, String, Option<String>, Option<String>, String, String);

/// When the digest goes out, in `platform.timezone`.
pub const DIGEST_AT: &str = "07:00:00";

/// Schedules `send_digests` for 07:00 in the institution's zone. Called at start and when the zone changes.
pub fn register_digest_schedule(repo: &PostgresRepository) -> Result<(), String> {
    let zone = repo
        .get_platform_settings()
        .map_err(storage)?
        .get("platform.timezone")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| "UTC".to_string());
    crate::jobs::register_daily_schedule(repo, "send_digests", &zone, DIGEST_AT).map_err(|e| e.to_string())
}

/// Notifications phase 3: one mail per person with a line per pending notice, grouped by app. Pending means unread
/// and held for the digest, by the person's choice or by the hourly limit. Each notice goes in one digest only. The
/// lines are the notices' own titles and bodies, which `render` built from names and counts, and a link.
pub fn send_digests(repo: &PostgresRepository) -> Result<Value, String> {
    let settings = repo.get_platform_settings().map_err(storage)?;
    let base = match mailing(&settings) {
        Ok(b) => b,
        Err(why) => return Ok(json!({ "digests": 0, "idle": why })),
    };
    let pending: Vec<Pending> = repo
        .with_client(|c| {
            Ok(c.query(
                "SELECT n.id, n.recipient, n.app_slug, m.title, n.title, n.body FROM notifications n LEFT JOIN app_manifests m ON m.slug = n.app_slug \
                 WHERE n.email_state = 'digest' AND n.read_at IS NULL ORDER BY lower(n.recipient), m.title NULLS LAST, n.created_at",
                &[],
            )?
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4), r.get(5)))
            .collect())
        })
        .map_err(storage)?;
    let mut people: Vec<String> = pending.iter().map(|p| p.1.to_lowercase()).collect();
    people.dedup();
    let users = repo.users_by_names(&people).map_err(storage)?;
    let mut digests = 0;
    for who in people {
        let mine: Vec<&Pending> = pending.iter().filter(|p| p.1.to_lowercase() == who).collect();
        let mut text = String::new();
        let mut heading: Option<String> = None;
        for (id, _, slug, app_title, title, body) in &mine {
            let group = app_title.clone().or_else(|| slug.as_ref().map(|_| "An app".to_string())).unwrap_or_else(|| "Elsewhere".to_string());
            if heading.as_deref() != Some(group.as_str()) {
                text.push_str(&format!("\n{group}\n"));
                heading = Some(group);
            }
            text.push_str(&format!("- {title}. {body} Open it: {base}/n/{id}\n"));
        }
        let ids: Vec<Uuid> = mine.iter().map(|p| p.0).collect();
        let subject = format!("{} notice{} waiting for you", ids.len(), if ids.len() == 1 { "" } else { "s" });
        let body = format!("Here is what happened since your last digest.\n{text}");
        let email = users.iter().find(|u| u.user_name.eq_ignore_ascii_case(&who)).and_then(crate::service::people::primary_email);
        let sent = repo
            .with_client(move |c| {
                let mut tx = c.transaction()?;
                let Some(to) = address_of(&mut tx, &who, email.as_deref())? else { return Ok(false) };
                // The digest is the mail the hourly limit points to, so the limit does not hold it back.
                tx.execute("INSERT INTO email_outbox (id, to_address, subject, body) VALUES ($1, $2, $3, $4)", &[&Uuid::new_v4(), &to, &subject, &body])?;
                tx.execute("UPDATE notifications SET email_state = 'digested' WHERE id = ANY($1)", &[&ids])?;
                tx.commit()?;
                Ok(true)
            })
            .map_err(storage)?;
        if sent {
            digests += 1;
        }
    }
    Ok(json!({ "digests": digests }))
}

/// The `send_digests` job, daily at 07:00 in the institution's zone.
pub fn send_digests_job(ctx: &crate::jobs::JobCtx, _payload: Value) -> Result<Value, crate::jobs::JobError> {
    send_digests(ctx.repository()).map_err(crate::jobs::JobError::Retry)
}
