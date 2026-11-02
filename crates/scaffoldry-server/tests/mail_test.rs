//! Notifications phase 2: email.
//!
//! A notice that should be mailed puts a row in `email_outbox` in the same transaction. The `send_email` job sends the
//! due rows over SMTP through `net::egress`: the host must be on `egress.allow`, the name is resolved once, and the
//! connection goes to that address with TLS checked against the name. Nothing is sent while `mail.enabled` is false.
//!
//! These tests run a fake SMTP server in the process. It speaks TLS with a certificate from a throwaway authority that
//! this process trusts through `SSL_CERT_FILE`, which the system certificate store reads. The name `mail.test` reaches
//! it through an injected resolver and an allowlist entry for 127.0.0.1. No test needs a network.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::mail;
use scaffoldry_server::net::egress::Resolver;
use scaffoldry_server::notifications::{self, AppRef, Notice, PersonRef, RecordRef, Recipients, TableRef};
use scaffoldry_server::secrets::{self, MasterKey};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const OWNER: &str = "einstein@physics.state.edu";
const WORKSPACE: &str = "ws-physics-optics";
const BASE: &str = "https://desk.state.edu";
/// The mail server's password. Looked for everywhere it must not be.
const PASSWORD: &str = "smtp-Pw-UNMISTAKABLE-8fQ2";

// ---- A certificate authority for this process -------------------------------------------------------------------

struct Pki {
    cert: PathBuf,
    key: PathBuf,
}

/// A throwaway authority and a certificate for `mail.test` it signed, made once with the system's `openssl`. The
/// authority becomes the trust store of this process.
fn pki() -> &'static Pki {
    static PKI: OnceLock<Pki> = OnceLock::new();
    PKI.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("scaffoldry-mail-pki-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            let out = std::process::Command::new("openssl").args(args).current_dir(&dir).output().expect("openssl");
            assert!(out.status.success(), "openssl {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        let ec = ["-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes"];
        run(&[&["req", "-x509"][..], &ec, &["-keyout", "ca.key", "-out", "ca.pem", "-days", "2", "-subj", "/CN=Scaffoldry Test Mail CA"]].concat());
        run(&[&["req"][..], &ec, &["-keyout", "leaf.key", "-out", "leaf.csr", "-subj", "/CN=mail.test"]].concat());
        std::fs::write(dir.join("leaf.ext"), "subjectAltName=DNS:mail.test\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n").unwrap();
        run(&["x509", "-req", "-in", "leaf.csr", "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial", "-out", "leaf.pem", "-days", "2", "-extfile", "leaf.ext"]);
        // The system certificate store reads this. Every test in this file sets the same value.
        std::env::set_var("SSL_CERT_FILE", dir.join("ca.pem"));
        Pki { cert: dir.join("leaf.pem"), key: dir.join("leaf.key") }
    })
}

fn server_tls() -> Arc<rustls::ServerConfig> {
    let p = pki();
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(&p.cert).unwrap().map(Result::unwrap).collect();
    let key = PrivateKeyDer::from_pem_file(&p.key).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Arc::new(rustls::ServerConfig::builder_with_provider(provider).with_safe_default_protocol_versions().unwrap().with_no_client_auth().with_single_cert(certs, key).unwrap())
}

// ---- A fake SMTP server -----------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct Got {
    /// The password from `AUTH PLAIN`, decoded.
    password: Option<String>,
    from: String,
    to: Vec<String>,
    data: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Tls,
    StartTls,
}

struct Fake {
    port: u16,
    got: Arc<Mutex<Vec<Got>>>,
    /// When set, `MAIL FROM` gets a temporary failure. Its text repeats what the client sent, password included.
    fail: Arc<AtomicBool>,
}

impl Fake {
    fn start(mode: Mode) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (got, fail) = (Arc::new(Mutex::new(Vec::new())), Arc::new(AtomicBool::new(false)));
        let (g, f, tls) = (got.clone(), fail.clone(), server_tls());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (g, f, tls) = (g.clone(), f.clone(), tls.clone());
                std::thread::spawn(move || {
                    let _ = serve(stream, mode, tls, g, f);
                });
            }
        });
        Fake { port, got, fail }
    }

    fn got(&self) -> Vec<Got> {
        self.got.lock().unwrap().clone()
    }
}

fn line<S: Read>(s: &mut S) -> std::io::Result<String> {
    let mut out = Vec::new();
    let mut b = [0u8; 1];
    loop {
        if s.read(&mut b)? == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        out.push(b[0]);
        if out.ends_with(b"\r\n") {
            out.truncate(out.len() - 2);
            return Ok(String::from_utf8_lossy(&out).into_owned());
        }
    }
}

fn serve(mut tcp: TcpStream, mode: Mode, tls: Arc<rustls::ServerConfig>, got: Arc<Mutex<Vec<Got>>>, fail: Arc<AtomicBool>) -> std::io::Result<()> {
    if mode == Mode::StartTls {
        tcp.write_all(b"220 mail.test ESMTP\r\n")?;
        loop {
            let l = line(&mut tcp)?;
            let verb = l.split(' ').next().unwrap_or("").to_ascii_uppercase();
            match verb.as_str() {
                "EHLO" => tcp.write_all(b"250-mail.test\r\n250 STARTTLS\r\n")?,
                "STARTTLS" => {
                    tcp.write_all(b"220 go ahead\r\n")?;
                    break;
                }
                // Nothing but STARTTLS is taken in the clear.
                _ => tcp.write_all(b"530 5.7.0 Must issue a STARTTLS command first\r\n")?,
            }
        }
        let conn = rustls::ServerConnection::new(tls).unwrap();
        return session(rustls::StreamOwned::new(conn, tcp), false, got, fail);
    }
    let conn = rustls::ServerConnection::new(tls).unwrap();
    session(rustls::StreamOwned::new(conn, tcp), true, got, fail)
}

fn session<S: Read + Write>(mut s: S, greet: bool, got: Arc<Mutex<Vec<Got>>>, fail: Arc<AtomicBool>) -> std::io::Result<()> {
    use base64::Engine;
    if greet {
        s.write_all(b"220 mail.test ESMTP\r\n")?;
    }
    let mut current = Got::default();
    let mut auth_line = String::new();
    loop {
        let l = line(&mut s)?;
        let verb = l.split(' ').next().unwrap_or("").to_ascii_uppercase();
        match verb.as_str() {
            "EHLO" | "HELO" => s.write_all(b"250-mail.test\r\n250-AUTH PLAIN LOGIN\r\n250 8BITMIME\r\n")?,
            "AUTH" => {
                auth_line = l.clone();
                let mut parts = l.split(' ').skip(1);
                let _mechanism = parts.next();
                let encoded = match parts.next() {
                    Some(e) => e.to_string(),
                    None => {
                        s.write_all(b"334 \r\n")?;
                        line(&mut s)?
                    }
                };
                let decoded = base64::engine::general_purpose::STANDARD.decode(encoded.trim()).unwrap_or_default();
                current.password = decoded.split(|b| *b == 0).nth(2).map(|p| String::from_utf8_lossy(p).into_owned());
                s.write_all(b"235 2.7.0 Authentication successful\r\n")?;
            }
            "MAIL" => {
                if fail.load(Ordering::SeqCst) {
                    let password = current.password.clone().unwrap_or_default();
                    s.write_all(format!("451 4.3.0 Try again later ({auth_line} {password})\r\n").as_bytes())?;
                } else {
                    current.from = l.clone();
                    s.write_all(b"250 OK\r\n")?;
                }
            }
            "RCPT" => {
                current.to.push(l.trim_start_matches(|c: char| c != '<').trim_matches(|c| c == '<' || c == '>').to_string());
                s.write_all(b"250 OK\r\n")?;
            }
            "DATA" => {
                s.write_all(b"354 End data with <CR><LF>.<CR><LF>\r\n")?;
                let mut data = String::new();
                loop {
                    let l = line(&mut s)?;
                    if l == "." {
                        break;
                    }
                    data.push_str(&l);
                    data.push('\n');
                }
                current.data = data;
                got.lock().unwrap().push(std::mem::take(&mut current));
                s.write_all(b"250 OK queued\r\n")?;
            }
            "RSET" | "NOOP" => s.write_all(b"250 OK\r\n")?,
            "QUIT" => {
                s.write_all(b"221 Bye\r\n")?;
                return Ok(());
            }
            _ => s.write_all(b"502 Command not implemented\r\n")?,
        }
        s.flush()?;
    }
}

// ---- The world --------------------------------------------------------------------------------------------------

/// Answers every name with one address.
struct Fixed(IpAddr);

impl Resolver for Fixed {
    fn lookup(&self, _host: &str, _port: u16) -> std::io::Result<Vec<IpAddr>> {
        Ok(vec![self.0])
    }
}

fn key_file() -> PathBuf {
    use base64::Engine;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    let dir = std::env::temp_dir().join(format!("scaffoldry-key-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("master.key");
    std::fs::write(&path, base64::engine::general_purpose::STANDARD.encode(bytes)).unwrap();
    path
}

struct World {
    db: TestDb,
    state: Arc<ServerState>,
    app: axum::Router,
    key: PathBuf,
    fake: Fake,
}

fn settings(port: u16, enabled: bool, mode: Mode) -> Vec<(&'static str, Value)> {
    let security = if mode == Mode::Tls { "tls" } else { "starttls" };
    vec![
        ("mail.enabled", json!(enabled)),
        ("mail.smtp", json!({ "host": "mail.test", "port": port, "security": security, "from": "desk@state.edu", "username": "desk", "secret": "smtp_password" })),
        ("platform.base_url", json!(BASE)),
        ("egress.allow", json!([{ "cidr": "127.0.0.1/32", "ports": [port] }])),
    ]
}

fn put_settings(db: &TestDb, values: Vec<(&'static str, Value)>) {
    db.run(move |c| {
        for (k, v) in values {
            c.execute(
                "INSERT INTO platform_settings (key, value, updated_by) VALUES ($1, $2, 'test') ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
                &[&k, &v],
            )
            .unwrap();
        }
    });
}

fn world_with(enabled: bool, mode: Mode) -> World {
    pki();
    let fake = Fake::start(mode);
    let db = TestDb::new();
    put_settings(&db, settings(fake.port, enabled, mode));
    let key = key_file();
    let mut state = ServerState::connect(db.url()).expect("server");
    state.master_key_file = Some(key.clone());
    let state = Arc::new(state);
    state.seed_demo().expect("seed");
    secrets::put(&state.repository, &MasterKey::from_file(&key).unwrap(), "smtp_password", PASSWORD.as_bytes(), ADMIN).unwrap();
    let app = build_app_with_state(state.clone()).expect("router");
    World { db, state, app, key, fake }
}

fn world() -> World {
    world_with(true, Mode::Tls)
}

async fn send(app: &axum::Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value, String) {
    let b = Request::builder().method(method).uri(uri);
    let b = if who.is_empty() { b } else { b.header("authorization", format!("Bearer {}", issue_test_token_and_user(who))) };
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null), text)
}

/// Runs the `send_email` job's work once, as the job does, with the test's resolver.
async fn send_due(w: &World, resolver: Arc<dyn Resolver>) -> Value {
    let (repo, key) = (w.state.repository.clone(), w.key.clone());
    tokio::task::spawn_blocking(move || mail::send_due(&repo, Some(Path::new(&key)), resolver.as_ref())).await.unwrap().expect("the job ran")
}

fn local() -> Arc<dyn Resolver> {
    Arc::new(Fixed("127.0.0.1".parse().unwrap()))
}

async fn notify(w: &World, to: &str, notice: Notice) -> usize {
    let (st, to) = (w.state.clone(), to.to_string());
    tokio::task::spawn_blocking(move || notifications::notify(&st, &Recipients::Person(to), Some(ADMIN), &notice)).await.unwrap().unwrap()
}

fn delegation() -> Notice {
    Notice::DelegationReceived { by: PersonRef(ADMIN.into()) }
}

#[derive(Debug)]
struct Row {
    to: String,
    subject: String,
    body: String,
    state: String,
    attempts: i32,
    wait_secs: f64,
    last_error: Option<String>,
    notification: Option<uuid::Uuid>,
}

fn outbox(db: &TestDb) -> Vec<Row> {
    db.run(|c| {
        c.query(
            "SELECT to_address, subject, body, state, attempts, EXTRACT(EPOCH FROM next_attempt_at - NOW())::float8, last_error, notification_id FROM email_outbox ORDER BY created_at",
            &[],
        )
        .unwrap()
        .iter()
        .map(|r| Row { to: r.get(0), subject: r.get(1), body: r.get(2), state: r.get(3), attempts: r.get(4), wait_secs: r.get(5), last_error: r.get(6), notification: r.get(7) })
        .collect()
    })
}

fn email_states(db: &TestDb, who: &'static str) -> Vec<String> {
    db.run(move |c| {
        c.query("SELECT email_state FROM notifications WHERE lower(recipient) = lower($1) ORDER BY created_at", &[&who]).unwrap().iter().map(|r| r.get(0)).collect()
    })
}

/// Makes every queued row due now, as if its wait had passed.
fn make_due(db: &TestDb) {
    db.run(|c| {
        c.execute("UPDATE email_outbox SET next_attempt_at = NOW() - INTERVAL '1 second' WHERE state = 'queued'", &[]).unwrap();
    });
}

// ---- The tests --------------------------------------------------------------------------------------------------

// 1. With `mail.enabled` false nothing is sent, nothing is queued, and the inbox still works.
#[tokio::test]
async fn with_mail_off_nothing_is_sent_and_the_inbox_still_works() {
    let w = world_with(false, Mode::Tls);
    assert_eq!(notify(&w, OWNER, delegation()).await, 1);
    let (s, b, _) = send(&w.app, "GET", "/api/v1/me/notifications/count", OWNER, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["unread"], 1, "{b}");
    assert!(outbox(&w.db).is_empty(), "nothing is queued while mail is off");
    assert_eq!(email_states(&w.db, OWNER), vec!["none".to_string()]);
    // A row already queued, from before mail was turned off, waits. The job sends nothing.
    w.db.run(|c| {
        c.execute("INSERT INTO email_outbox (id, to_address, subject, body) VALUES (gen_random_uuid(), 'someone@state.edu', 'Waiting', 'Waiting')", &[]).unwrap();
    });
    let report = send_due(&w, local()).await;
    assert_eq!(report["sent"], 0, "{report}");
    assert!(w.fake.got().is_empty(), "the mail server heard nothing");
    assert_eq!(outbox(&w.db)[0].state, "queued");
}

// 2. A notice that should be mailed makes one message, to the right address, with the right subject and a link under
// `platform.base_url`. The text holds no record value from the fixture. Both TLS on connect and STARTTLS work.
#[tokio::test]
async fn a_mailed_notice_is_one_message_with_the_title_the_body_and_a_link_and_no_record_value() {
    for mode in [Mode::Tls, Mode::StartTls] {
        let w = world_with(true, mode);
        let slug = format!("mail-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        let manifest = json!({
            "slug": slug, "title": "Lab Bookings", "description": "", "organization_code": "PHYS", "department": "physics", "workspace_id": WORKSPACE, "views": [],
            "tables": [{ "id": "bookings", "name": "Bookings", "slug": "bookings", "primary_field": "title", "fields": [{ "name": "title", "label": "Title", "field_type": "Text", "required": false, "ferpa_sensitive": false }] }],
        });
        let (s, b, _) = send(&w.app, "POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest)).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
        const VALUE: &str = "Confidential-Booking-VALUE-q81";
        let (s, b, _) = send(&w.app, "POST", &format!("/api/v1/apps/{slug}/records"), OWNER, Some(json!({ "data": { "_table_id": "bookings", "title": VALUE } }))).await;
        assert!(s.is_success(), "{b}");
        let record = b["id"].as_str().or_else(|| b["record"]["id"].as_str()).expect("a record id").to_string();

        let notice = Notice::Mention { by: PersonRef(ADMIN.into()), app: AppRef(slug.clone()), table: TableRef("bookings".into()), record: RecordRef(record.clone()) };
        assert_eq!(notify(&w, OWNER, notice).await, 1);
        let rows = outbox(&w.db);
        assert_eq!(rows.len(), 1, "one message: {rows:?}");
        let id = rows[0].notification.expect("the row names its notice");
        let title: String = w.db.run(move |c| c.query_one("SELECT title FROM notifications WHERE id = $1", &[&id]).unwrap().get(0));
        assert_eq!(rows[0].to, OWNER);
        assert_eq!(rows[0].subject, title);
        assert!(rows[0].body.contains(&format!("Open it: {BASE}/n/{id}")), "{}", rows[0].body);
        assert_eq!(email_states(&w.db, OWNER), vec!["queued".to_string()]);

        let report = send_due(&w, local()).await;
        assert_eq!(report["sent"], 1, "{report}");
        let got = w.fake.got();
        assert_eq!(got.len(), 1, "one message reached the server");
        assert_eq!(got[0].to, vec![OWNER.to_string()]);
        assert_eq!(got[0].password.as_deref(), Some(PASSWORD), "it signed in with the stored password");
        let text = got[0].data.replace("=\n", "");
        assert!(text.contains(&format!("{BASE}/n/{id}")), "{text}");
        assert!(text.contains("Lab Bookings"), "the app's name may appear: {text}");
        assert!(!text.contains(VALUE), "no record value: {text}");
        assert!(!text.contains(&record), "not even the record's id: {text}");
        assert!(!text.to_ascii_lowercase().contains("bearer") && !text.contains("token="), "no token: {text}");
        assert_eq!(outbox(&w.db)[0].state, "sent");
        assert_eq!(email_states(&w.db, OWNER), vec!["sent".to_string()]);
    }
}

// 3. A server error retries after 1 minute, 10 minutes, 1 hour and 6 hours, and the row is `failed` after the last.
// A success ends `sent`.
#[tokio::test]
async fn a_failure_retries_on_the_schedule_and_ends_failed_and_a_success_ends_sent() {
    let w = world();
    w.fake.fail.store(true, Ordering::SeqCst);
    notify(&w, OWNER, delegation()).await;
    for (attempt, wait) in [(1, 60.0), (2, 600.0), (3, 3600.0), (4, 21600.0)] {
        let report = send_due(&w, local()).await;
        assert_eq!(report["retried"], 1, "attempt {attempt}: {report}");
        let row = &outbox(&w.db)[0];
        assert_eq!((row.state.as_str(), row.attempts), ("queued", attempt), "{row:?}");
        assert!((row.wait_secs - wait).abs() < 30.0, "attempt {attempt} waits {wait}s, not {}", row.wait_secs);
        // Not due yet: another run leaves it alone.
        assert_eq!(send_due(&w, local()).await["retried"], 0);
        make_due(&w.db);
    }
    let report = send_due(&w, local()).await;
    assert_eq!(report["failed"], 1, "{report}");
    let row = &outbox(&w.db)[0];
    assert_eq!((row.state.as_str(), row.attempts), ("failed", 5));
    assert!(row.last_error.as_deref().is_some_and(|e| e.contains("451")), "the error is kept: {row:?}");
    assert_eq!(email_states(&w.db, OWNER), vec!["failed".to_string()]);
    let (s, b, _) = send(&w.app, "GET", "/api/v1/admin/mail", ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["enabled"], true);
    assert_eq!(b["queued"], 0);
    assert_eq!(b["failures"][0]["domain"], "physics.state.edu", "the recipient's domain, not the address: {b}");
    assert!(!b.to_string().contains(OWNER), "{b}");

    w.fake.fail.store(false, Ordering::SeqCst);
    notify(&w, OWNER, delegation()).await;
    let (_, b, _) = send(&w.app, "GET", "/api/v1/admin/mail", ADMIN, None).await;
    assert_eq!(b["queued"], 1, "{b}");
    assert!(b["oldest_queued_at"].is_string(), "{b}");
    assert_eq!(send_due(&w, local()).await["sent"], 1);
    assert_eq!(outbox(&w.db)[1].state, "sent");
}

// 4. The SMTP host must be on `egress.allow`, and a name that resolves to a private address is refused.
#[tokio::test]
async fn the_mail_host_must_be_allowlisted_and_a_private_address_is_refused() {
    let w = world();
    notify(&w, OWNER, delegation()).await;
    // Off the list: nothing connects.
    put_settings(&w.db, vec![("egress.allow", json!([]))]);
    let report = send_due(&w, local()).await;
    assert_eq!(report["retried"], 1, "{report}");
    let row = &outbox(&w.db)[0];
    assert!(row.last_error.as_deref().is_some_and(|e| e.contains("allowlist")), "{row:?}");
    // On the list by name, but the name resolves to a private address no cidr entry names.
    put_settings(&w.db, vec![("egress.allow", json!([{ "host": "mail.test", "ports": [w.fake.port] }]))]);
    make_due(&w.db);
    send_due(&w, Arc::new(Fixed("10.0.0.25".parse().unwrap()))).await;
    let row = &outbox(&w.db)[0];
    assert!(row.last_error.as_deref().is_some_and(|e| e.contains("private")), "{row:?}");
    assert!(w.fake.got().is_empty(), "the mail server heard nothing");
    // Allowed again, it goes.
    put_settings(&w.db, vec![("egress.allow", json!([{ "cidr": "127.0.0.1/32", "ports": [w.fake.port] }]))]);
    make_due(&w.db);
    assert_eq!(send_due(&w, local()).await["sent"], 1);
}

// 5. The password is never in a log, a response, or an error, even when the mail server repeats it back.
#[tokio::test]
async fn the_password_is_never_in_a_log_a_response_or_an_error() {
    let w = world();
    w.fake.fail.store(true, Ordering::SeqCst);
    notify(&w, OWNER, delegation()).await;
    let report = send_due(&w, local()).await;
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(format!("\0desk\0{PASSWORD}"));
    let leaks = |text: &str| text.contains(PASSWORD) || text.contains(&encoded);
    assert!(!leaks(&report.to_string()), "{report}");
    let row = &outbox(&w.db)[0];
    assert!(row.last_error.is_some());
    assert!(!leaks(row.last_error.as_deref().unwrap()), "{row:?}");
    let (_, _, text) = send(&w.app, "GET", "/api/v1/admin/mail", ADMIN, None).await;
    assert!(!leaks(&text), "{text}");
    let (_, _, text) = send(&w.app, "POST", "/api/v1/admin/mail/test", ADMIN, Some(json!({ "reason": "Checking the relay" }))).await;
    assert!(!leaks(&text), "{text}");
    let (_, _, text) = send(&w.app, "GET", "/api/v1/settings", ADMIN, None).await;
    assert!(!leaks(&text), "{text}");
    let ledger: Vec<String> = w.db.run(|c| c.query("SELECT row_to_json(l)::text FROM governance_ledger l", &[]).unwrap().iter().map(|r| r.get(0)).collect());
    assert!(ledger.iter().all(|l| !leaks(l)), "the ledger holds no password");
    let stored: Vec<String> = w.db.run(|c| c.query("SELECT row_to_json(o)::text FROM email_outbox o", &[]).unwrap().iter().map(|r| r.get(0)).collect());
    assert!(stored.iter().all(|l| !leaks(l)), "the outbox holds no password: {stored:?}");
    // The job's own record of the run, as the jobs list shows it, is the report above.
}

// 6. Past 50 mails to one person in an hour, a notice stays in the inbox and waits for the digest.
#[tokio::test]
async fn the_fifty_first_mail_in_an_hour_stays_in_the_inbox_for_the_digest() {
    let w = world();
    w.db.run(|c| {
        c.execute(
            "INSERT INTO email_outbox (id, to_address, subject, body, state, sent_at) SELECT gen_random_uuid(), $1, 'Earlier', 'Earlier', 'sent', NOW() FROM generate_series(1, 50)",
            &[&OWNER],
        )
        .unwrap();
        // Mail from more than an hour ago does not count.
        c.execute(
            "INSERT INTO email_outbox (id, to_address, subject, body, state, sent_at, created_at) SELECT gen_random_uuid(), 'other@state.edu', 'Old', 'Old', 'sent', NOW(), NOW() - INTERVAL '2 hours' FROM generate_series(1, 60)",
            &[],
        )
        .unwrap();
    });
    assert_eq!(notify(&w, OWNER, delegation()).await, 1, "the notice is made");
    assert_eq!(outbox(&w.db).len(), 110, "no 51st mail is queued");
    assert_eq!(email_states(&w.db, OWNER), vec!["digest".to_string()]);
    let (_, b, _) = send(&w.app, "GET", "/api/v1/me/notifications/count", OWNER, None).await;
    assert_eq!(b["unread"], 1, "it is in the inbox: {b}");
    // Someone else is under the limit.
    let other = "other@state.edu";
    issue_test_token_and_user(other);
    notify(&w, other, delegation()).await;
    assert_eq!(email_states(&w.db, other), vec!["queued".to_string()]);
}

// 7. `/n/{id}` needs a session: without one the desk is told to sign in, and with one it gets the notice's target.
// Only the notice's recipient may follow it. (The desk's side, sign-in and then the redirect, is a web test.)
#[tokio::test]
async fn following_a_mailed_link_needs_a_session_and_then_leads_to_the_target() {
    let w = world();
    notify(&w, OWNER, delegation()).await;
    let id = outbox(&w.db)[0].notification.unwrap();
    let path = format!("/api/v1/me/notifications/{id}/open");
    let (s, b, _) = send(&w.app, "GET", &path, "", None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{b}");
    let (s, b, _) = send(&w.app, "GET", &path, "someone.else@state.edu", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "another person's notice is not found: {b}");
    let (s, b, _) = send(&w.app, "GET", &path, OWNER, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(b["path"].as_str().is_some_and(|p| p.starts_with('/')), "a path on the desk: {b}");
    let (_, b, _) = send(&w.app, "GET", "/api/v1/me/notifications/count", OWNER, None).await;
    assert_eq!(b["unread"], 0, "following it marks it read: {b}");
}

// 8. The test mail goes only to the caller, needs a reason, and is a ledger entry. Only a Platform Admin may send it.
#[tokio::test]
async fn the_test_mail_goes_only_to_the_caller_and_is_a_ledger_entry() {
    let w = world();
    let (s, b, _) = send(&w.app, "POST", "/api/v1/admin/mail/test", ADMIN, Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a reason is needed: {b}");
    let (s, _, _) = send(&w.app, "POST", "/api/v1/admin/mail/test", OWNER, Some(json!({ "reason": "Mine" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, b, _) = send(&w.app, "POST", "/api/v1/admin/mail/test", ADMIN, Some(json!({ "reason": "Checking the relay", "to": "elsewhere@example.com" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let rows = outbox(&w.db);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].to, ADMIN, "only to the caller, whatever the body says");
    assert_eq!(send_due(&w, local()).await["sent"], 1);
    let got = w.fake.got();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].to, vec![ADMIN.to_string()]);
    let entries: Vec<Value> = w.db.run(|c| {
        c.query("SELECT payload FROM governance_ledger WHERE decision_type = 'MailTested'", &[]).unwrap().iter().map(|r| r.get(0)).collect()
    });
    assert_eq!(entries.len(), 1, "one ledger entry");
    let ledger_text = entries[0].to_string();
    assert!(ledger_text.contains("Checking the relay") || ledger_text.contains("state.edu"), "{ledger_text}");
}

// The settings are in the closed list and checked before they are stored.
#[tokio::test]
async fn the_mail_settings_are_checked() {
    let w = world();
    let put = |key: &'static str, value: Value| {
        let app = w.app.clone();
        async move { send(&app, "PUT", &format!("/api/v1/settings/{key}"), ADMIN, Some(value)).await.0 }
    };
    assert_eq!(put("mail.enabled", json!("yes")).await, StatusCode::BAD_REQUEST);
    assert_eq!(put("mail.enabled", json!(false)).await, StatusCode::OK);
    assert_eq!(put("mail.smtp", json!({ "host": "mail.test", "port": 465, "security": "none", "from": "desk@state.edu" })).await, StatusCode::BAD_REQUEST);
    assert_eq!(put("mail.smtp", json!({ "host": "mail.test", "port": 465, "security": "tls", "from": "not an address" })).await, StatusCode::BAD_REQUEST);
    assert_eq!(put("mail.smtp", json!({ "host": "mail.test", "port": 465, "security": "tls", "from": "desk@state.edu", "username": "desk" })).await, StatusCode::BAD_REQUEST, "a username needs a secret");
    assert_eq!(put("mail.smtp", json!({ "host": "mail.test", "port": 587, "security": "starttls", "from": "desk@state.edu", "username": "desk", "secret": "smtp_password" })).await, StatusCode::OK);
    assert_eq!(put("platform.base_url", json!("http://desk.state.edu")).await, StatusCode::BAD_REQUEST, "links are https");
    assert_eq!(put("platform.base_url", json!("https://desk.state.edu")).await, StatusCode::OK);
    // The secret the mail settings name cannot be deleted while they name it.
    let (s, b, _) = send(&w.app, "DELETE", "/api/v1/admin/secrets/smtp_password", ADMIN, Some(json!({ "reason": "Retired" }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert!(b["error"].as_str().unwrap_or_default().contains("mail.smtp"), "{b}");
}
