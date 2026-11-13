//! Connections phase 3: the PostgreSQL connector.
//!
//! A connection runs one `SELECT` against another database and syncs its rows into a dataset, the way a file drop does.
//!
//! - **It reaches the network only through `net::egress`.** The host is checked against the outbound allowlist and
//!   looked up once. The connection goes to that address, and the name is used only to check the certificate.
//! - **TLS is always on.** `verify-full` checks the chain and the name. `require` checks the chain and not the name, so
//!   a source whose certificate is for another name still works, and nothing is sent in the clear. The certificates it
//!   trusts are the system's. There is no mode that skips them.
//! - **It cannot write.** The query must be one `SELECT` or `WITH`, with no semicolon outside a string or comment. It
//!   runs in a read-only transaction regardless, so a function that writes is refused by the source.
//! - **Memory stays flat.** Rows come through a cursor, 5,000 at a time, and are written 1,000 at a time. The query
//!   runs twice in one `REPEATABLE READ` transaction, so both passes see the same rows: the first checks every row and
//!   counts them, so a fault or a source that shrank changes nothing, and the second writes.
//! - **The password is read from the secrets store for this one connection and is never in an error.**
//!
//! NIST SP 800-53 SC-7, SC-8 and AC-4.

use crate::connections::{key_source_of, last_good, sync_ctx, trash_removed, type_one, write_batch, Conn, Counts, Fault, RunOpts, Source, BATCH, SHRINK_FLOOR_PERCENT};
use crate::net::egress::{resolve_and_check, Allow, Resolver, SystemResolver};
use crate::state::SharedState;
use postgres::config::SslMode;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, RootCertStore, SignatureScheme};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

/// Rows read from the cursor at once.
pub const FETCH: usize = 5_000;
/// How long the query may run, unless the connection says less or more.
pub const DEFAULT_TIMEOUT_SECS: u64 = 300;
const MAX_TIMEOUT_SECS: u64 = 3_600;
const CONNECT_SECS: u64 = 5;

// ---- The query check ---------------------------------------------------------------------------------------------

/// Whether `sql` is one `SELECT` or `WITH` statement. Strings, quoted names, dollar quotes and comments are read, so a
/// semicolon inside one is not a second statement. This is a check for a plain mistake. The read-only transaction is the
/// guard.
pub fn check_query(sql: &str) -> Result<(), String> {
    let b = sql.as_bytes();
    let mut i = 0;
    let mut first_word: Option<String> = None;
    let mut word = String::new();
    let end_word = |word: &mut String, first: &mut Option<String>| {
        if !word.is_empty() && first.is_none() {
            *first = Some(word.to_lowercase());
        }
        word.clear();
    };
    while i < b.len() {
        let c = b[i];
        match c {
            b'\'' => {
                end_word(&mut word, &mut first_word);
                // An `E'...'` string reads a backslash as an escape.
                let escapes = i > 0 && matches!(b[i - 1], b'e' | b'E') && (i < 2 || !b[i - 2].is_ascii_alphanumeric());
                i += 1;
                loop {
                    match b.get(i) {
                        None => return Err("The query has a string that is never closed".to_string()),
                        Some(b'\\') if escapes => i += 2,
                        Some(b'\'') if b.get(i + 1) == Some(&b'\'') => i += 2,
                        Some(b'\'') => break,
                        Some(_) => i += 1,
                    }
                }
            }
            b'"' => {
                end_word(&mut word, &mut first_word);
                i += 1;
                loop {
                    match b.get(i) {
                        None => return Err("The query has a quoted name that is never closed".to_string()),
                        Some(b'"') if b.get(i + 1) == Some(&b'"') => i += 2,
                        Some(b'"') => break,
                        Some(_) => i += 1,
                    }
                }
            }
            b'-' if b.get(i + 1) == Some(&b'-') => {
                end_word(&mut word, &mut first_word);
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                end_word(&mut word, &mut first_word);
                let mut depth = 1;
                i += 2;
                while depth > 0 {
                    match (b.get(i), b.get(i + 1)) {
                        (None, _) => return Err("The query has a comment that is never closed".to_string()),
                        (Some(b'/'), Some(b'*')) => {
                            depth += 1;
                            i += 2;
                        }
                        (Some(b'*'), Some(b'/')) => {
                            depth -= 1;
                            i += 2;
                        }
                        _ => i += 1,
                    }
                }
                continue;
            }
            b'$' => {
                // A dollar quote: `$$ ... $$` or `$tag$ ... $tag$`. A `$1` is a parameter, not a quote.
                let mut j = i + 1;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                if b.get(j) == Some(&b'$') && !(j > i + 1 && b[i + 1].is_ascii_digit()) {
                    end_word(&mut word, &mut first_word);
                    let tag = &sql[i..=j];
                    match sql[j + 1..].find(tag) {
                        Some(at) => i = j + 1 + at + tag.len(),
                        None => return Err("The query has a quoted block that is never closed".to_string()),
                    }
                    continue;
                }
                end_word(&mut word, &mut first_word);
            }
            b';' => return Err("The query must be one statement, with no semicolon outside a string".to_string()),
            c if c.is_ascii_alphanumeric() || c == b'_' => word.push(c as char),
            _ => end_word(&mut word, &mut first_word),
        }
        i += 1;
    }
    end_word(&mut word, &mut first_word);
    match first_word.as_deref() {
        Some("select" | "with") => Ok(()),
        Some(other) => Err(format!("The query must begin with SELECT or WITH, not {}", other.to_uppercase())),
        None => Err("The query is empty".to_string()),
    }
}

/// The parts of a postgres connection's `config`, checked. Nothing connects.
pub fn check_config(config: &Value) -> Result<(), String> {
    let text = |k: &str| config.get(k).and_then(Value::as_str).map(str::trim).unwrap_or("");
    let host = text("host");
    if host.is_empty() || host.len() > 253 || host.contains(|c: char| c.is_whitespace() || matches!(c, '/' | '@' | '*' | ',')) {
        return Err("config.host is the source's host name".to_string());
    }
    if let Some(p) = config.get("port").filter(|p| !p.is_null()) {
        p.as_u64().filter(|p| (1..=65535).contains(p)).ok_or("config.port is a number from 1 to 65535")?;
    }
    if text("database").is_empty() || text("user").is_empty() {
        return Err("config.database and config.user are required".to_string());
    }
    if !matches!(text("tls"), "require" | "verify-full") {
        return Err("config.tls is require or verify-full. The connection is always encrypted".to_string());
    }
    if let Some(t) = config.get("timeout_seconds").filter(|t| !t.is_null()) {
        t.as_u64().filter(|t| (1..=MAX_TIMEOUT_SECS).contains(t)).ok_or("config.timeout_seconds is a number from 1 to 3600")?;
    }
    check_query(config.get("query").and_then(Value::as_str).unwrap_or(""))
}

fn timeout_of(config: &Value) -> u64 {
    config.get("timeout_seconds").and_then(Value::as_u64).unwrap_or(DEFAULT_TIMEOUT_SECS)
}

// ---- TLS ---------------------------------------------------------------------------------------------------------

/// Accepts a certificate whose chain is trusted even when it is not for the name asked. Any other fault is a fault.
#[derive(Debug)]
struct ChainOnly(Arc<WebPkiServerVerifier>);

impl ServerCertVerifier for ChainOnly {
    fn verify_server_cert(&self, end_entity: &CertificateDer<'_>, intermediates: &[CertificateDer<'_>], name: &ServerName<'_>, ocsp: &[u8], now: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        match self.0.verify_server_cert(end_entity, intermediates, name, ocsp, now) {
            Err(rustls::Error::InvalidCertificate(CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. })) => Ok(ServerCertVerified::assertion()),
            other => other,
        }
    }

    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

/// The connector for `mode`. The trusted certificates are the system's.
fn connector(mode: &str) -> Result<tokio_postgres_rustls::MakeRustlsConnect, String> {
    let mut roots = RootCertStore::empty();
    let found = rustls_native_certs::load_native_certs();
    for cert in found.certs {
        let _ = roots.add(cert);
    }
    if roots.is_empty() {
        return Err("This server has no trusted certificates to check the source against".to_string());
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone()).with_safe_default_protocol_versions().map_err(|e| e.to_string())?;
    let config = if mode == "verify-full" {
        builder.with_root_certificates(roots).with_no_client_auth()
    } else {
        let verifier = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider).build().map_err(|e| e.to_string())?;
        builder.dangerous().with_custom_certificate_verifier(Arc::new(ChainOnly(verifier))).with_no_client_auth()
    };
    Ok(tokio_postgres_rustls::MakeRustlsConnect::new(config))
}

// ---- The connection ----------------------------------------------------------------------------------------------

/// A message with the password taken out, in case a library ever repeats it.
fn scrub(message: String, password: &str) -> String {
    if password.is_empty() {
        message
    } else {
        message.replace(password, "***")
    }
}

/// What a source's error means, in a sentence.
fn explain(e: &postgres::Error, secs: u64, password: &str) -> String {
    if let Some(db) = e.as_db_error() {
        let said = match db.code().code() {
            "57014" => format!("The query ran past its time limit of {secs} second{}", if secs == 1 { "" } else { "s" }),
            "25006" => format!("The query tried to change data, and the source's read-only transaction refused it ({})", db.message()),
            "28P01" | "28000" => "The source refused the user or the password".to_string(),
            "3D000" => "The source has no such database".to_string(),
            _ => format!("The source answered: {}", db.message()),
        };
        return scrub(said, password);
    }
    // The reason is in the errors underneath, such as which fault the certificate had.
    let mut parts = vec![e.to_string()];
    let mut cause = std::error::Error::source(e);
    while let Some(c) = cause {
        parts.push(c.to_string());
        cause = c.source();
    }
    let detail = parts.join(": ");
    let said = if detail.to_lowercase().contains("certificate") { format!("The source's certificate was refused: {detail}") } else { format!("The source could not be reached: {detail}") };
    scrub(said, password)
}

fn open(state: &SharedState, config: &Value, password: &str) -> Result<postgres::Client, String> {
    let text = |k: &str| config.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let host = text("host");
    let port = config.get("port").and_then(Value::as_u64).unwrap_or(5432) as u16;
    let allow = {
        let settings = state.settings.read().map_err(|e| e.to_string())?;
        Allow::from_settings(&settings)
    };
    let resolver: Arc<dyn Resolver> = state.egress_resolver.clone().unwrap_or_else(|| Arc::new(SystemResolver));
    // The address is chosen once, here. The client connects to it, and the name is used only to check the certificate.
    let addr = resolve_and_check(&host, port, &allow, resolver.as_ref()).map_err(|e| e.to_string())?;
    let tls = connector(&text("tls"))?;
    let mut cfg = postgres::Config::new();
    cfg.host(&host).hostaddr(addr.ip()).port(port).user(&text("user")).password(password).dbname(&text("database"));
    cfg.ssl_mode(SslMode::Require).connect_timeout(Duration::from_secs(CONNECT_SECS)).application_name("scaffoldry-sync");
    cfg.connect(tls).map_err(|e| explain(&e, timeout_of(config), password))
}

/// Reads every row of a cursor, a batch at a time, and hands each batch to `f`.
fn each_batch(tx: &mut postgres::Transaction<'_>, cursor: &str, wrapped: &str, secs: u64, password: &str, mut f: impl FnMut(Vec<Value>) -> Result<(), String>) -> Result<(), String> {
    let ex = |e: postgres::Error| explain(&e, secs, password);
    tx.batch_execute(&format!("DECLARE {cursor} NO SCROLL CURSOR FOR {wrapped}")).map_err(ex)?;
    loop {
        let rows = tx.query(&format!("FETCH {FETCH} FROM {cursor}"), &[]).map_err(ex)?;
        if rows.is_empty() {
            break;
        }
        f(rows.iter().map(|r| r.get::<_, Value>(0)).collect())?;
    }
    tx.batch_execute(&format!("CLOSE {cursor}")).map_err(ex)
}

/// A row as text by column, the way a csv row is.
fn as_text(row: &Value) -> HashMap<String, String> {
    row.as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| {
                    let text = match v {
                        Value::String(s) => s.clone(),
                        Value::Null => String::new(),
                        other => other.to_string(),
                    };
                    (k.clone(), text)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// One run of a postgres connection.
pub(crate) fn apply(state: &SharedState, conn: &Conn, opts: &RunOpts) -> Result<(Option<Counts>, String), Fault> {
    let secret = conn.secret.as_deref().ok_or("The connection names no secret for the source's password")?;
    let password = {
        let held = crate::secrets::get(state, secret).map_err(|e| format!("The secret '{secret}' cannot be read: {e}"))?;
        String::from_utf8_lossy(held.expose()).to_string()
    };
    let secs = timeout_of(&conn.config);
    let ex = |e: postgres::Error| explain(&e, secs, &password);
    let query = conn.config.get("query").and_then(Value::as_str).unwrap_or("");
    check_query(query)?;
    let mut client = open(state, &conn.config, &password)?;
    // One snapshot for both passes, and nothing a query does can write.
    let mut tx = client.build_transaction().isolation_level(postgres::IsolationLevel::RepeatableRead).read_only(true).start().map_err(ex)?;
    tx.batch_execute(&format!("SET LOCAL statement_timeout = {}", secs * 1000)).map_err(ex)?;
    // A new line before the closing bracket, so a comment on the query's last line does not swallow it.
    let wrapped = format!("SELECT to_jsonb(q) FROM (\n{query}\n) q");
    let headers: Vec<String> = tx.prepare(&format!("SELECT * FROM (\n{query}\n) q")).map_err(ex)?.columns().iter().map(|c| c.name().to_string()).collect();

    // A column the query gained or lost is a change of shape, handled as a file's is.
    let shape = Source { headers: Some(headers), rows: Vec::new() };
    if let Some(drift) = crate::connection_drift::find(conn, &shape) {
        return Err(Fault::Drift(drift));
    }
    if conn.pending_proposal.is_some() {
        crate::connection_drift::withdraw(state, conn);
    }

    // First pass: every row is checked and counted, and nothing is written.
    let key_source = key_source_of(conn);
    let (mut keys, mut hasher, mut total) = (HashSet::new(), Sha256::new(), 0usize);
    each_batch(&mut tx, "pass_one", &wrapped, secs, &password, |batch| {
        for row in batch {
            hasher.update(row.to_string().as_bytes());
            hasher.update(b"\n");
            total += 1;
            type_one(conn, &key_source, &as_text(&row), total + 1, &mut keys)?;
        }
        Ok(())
    })?;
    drop(keys);
    let hash: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    let (last_hash, last_rows) = last_good(state, conn.id).map_err(|e| e.to_string())?;
    if last_hash.as_deref() == Some(hash.as_str()) {
        return Ok((None, hash));
    }
    if let Some(prev) = last_rows.filter(|p| *p > 0) {
        if !opts.approve_shrink && total * 100 < prev * SHRINK_FLOOR_PERCENT {
            return Err(format!("source_shrank: the query returned {total} rows and the last run held {prev}. A person must approve a result this much smaller").into());
        }
    }

    // Second pass: the same rows, written in batches.
    let ctx = sync_ctx(state, conn);
    let ferpa = crate::connections::is_sensitive(&conn.sensitivity);
    let (mut seen, mut keys) = (HashSet::with_capacity(total), HashSet::new());
    let mut n = Counts { added: 0, changed: 0, removed: 0, unchanged: 0 };
    let mut pending: Vec<(String, Value)> = Vec::with_capacity(BATCH);
    let flush = |pending: &mut Vec<(String, Value)>, n: &mut Counts| -> Result<(), String> {
        if pending.is_empty() {
            return Ok(());
        }
        let (a, c, u) = write_batch(state, conn, &ctx, std::mem::take(pending), ferpa).map_err(|e| format!("A batch could not be written: {e}"))?;
        n.added += a;
        n.changed += c;
        n.unchanged += u;
        Ok(())
    };
    let mut seen_rows = 0usize;
    each_batch(&mut tx, "pass_two", &wrapped, secs, &password, |batch| {
        for row in batch {
            seen_rows += 1;
            let (key, data) = type_one(conn, &key_source, &as_text(&row), seen_rows + 1, &mut keys)?;
            let id = crate::connections::record_id(conn.id, &conn.dataset_table, &key);
            seen.insert(id.clone());
            pending.push((id, Value::Object(data)));
            if pending.len() >= BATCH {
                flush(&mut pending, &mut n)?;
            }
        }
        Ok(())
    })?;
    flush(&mut pending, &mut n)?;
    n.removed = trash_removed(state, conn, &ctx, seen).map_err(|e| format!("Removed rows could not be trashed: {e}"))?;
    Ok((Some(n), hash))
}
