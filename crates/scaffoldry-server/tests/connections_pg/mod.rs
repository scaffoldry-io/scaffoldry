//! Shared by the connections phase 3 tests: a second database as the source, certificates made with `openssl`, and a
//! TLS proxy in front of the source. The proxy answers Postgres's TLS request, presents a certificate, and passes the
//! bytes on. It stands in for a source that serves TLS, which the test database does not.
#![allow(dead_code)]

use crate::common::TestDb;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::net::egress::Resolver;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, OnceLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

pub const ADMIN: &str = "jordan.lee@state.edu";
pub const SECRET: &str = "source_pw";
/// The name the good certificate is for, and the name every connection uses. It resolves to the proxy.
pub const DB_HOST: &str = "db.example.test";
pub const OTHER_HOST: &str = "other.example.test";

/// Every name resolves to `ip`.
pub struct To(pub IpAddr);
impl Resolver for To {
    fn lookup(&self, _host: &str, _port: u16) -> std::io::Result<Vec<IpAddr>> {
        Ok(vec![self.0])
    }
}

struct Pki {
    dir: PathBuf,
}

fn openssl(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("openssl").args(args).current_dir(dir).output().expect("openssl is installed");
    assert!(out.status.success(), "openssl {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// One CA for the whole binary, and a certificate for each name. The CA is the only trusted root: the connector reads
/// the system's roots, and `SSL_CERT_FILE` names this one.
fn pki() -> &'static Pki {
    static PKI: OnceLock<Pki> = OnceLock::new();
    PKI.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("scaffoldry-pki-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        openssl(&dir, &["req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-keyout", "ca.key", "-out", "ca.pem", "-subj", "/CN=Test CA", "-days", "2",
            "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign"]);
        for name in [DB_HOST, OTHER_HOST] {
            let (key, csr, crt, ext) = (format!("{name}.key"), format!("{name}.csr"), format!("{name}.pem"), format!("{name}.ext"));
            std::fs::write(dir.join(&ext), format!("subjectAltName=DNS:{name}\nextendedKeyUsage=serverAuth\n")).unwrap();
            openssl(&dir, &["req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-keyout", &key, "-out", &csr, "-subj", &format!("/CN={name}")]);
            openssl(&dir, &["x509", "-req", "-in", &csr, "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial", "-out", &crt, "-days", "2", "-extfile", &ext]);
        }
        std::env::set_var("SSL_CERT_FILE", dir.join("ca.pem"));
        Pki { dir }
    })
}

/// A TLS proxy for `name`'s certificate in front of `backend`. Returns its port.
pub async fn proxy(name: &str, backend: SocketAddr) -> u16 {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let pki = pki();
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(pki.dir.join(format!("{name}.pem"))).unwrap().map(|c| c.unwrap()).collect();
    let key = PrivateKeyDer::from_pem_file(pki.dir.join(format!("{name}.key"))).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut client, _)) = listener.accept().await else { continue };
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                // The client first asks, in plain text, whether the server serves TLS.
                let mut ask = [0u8; 8];
                if client.read_exact(&mut ask).await.is_err() || ask != [0, 0, 0, 8, 4, 210, 22, 47] {
                    return;
                }
                if client.write_all(b"S").await.is_err() {
                    return;
                }
                let Ok(mut tls) = acceptor.accept(client).await else { return };
                let Ok(mut backend) = tokio::net::TcpStream::connect(backend).await else { return };
                let _ = tokio::io::copy_bidirectional(&mut tls, &mut backend).await;
            });
        }
    });
    port
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

pub struct World {
    pub db: TestDb,
    pub state: Arc<ServerState>,
    pub app: Router,
    /// The source database, with a table `programs (code, name, credits)`.
    pub src: TestDb,
    pub database: String,
    pub user: String,
    pub password: String,
    /// A proxy with the good certificate, and one whose certificate is for another name.
    pub good_port: u16,
    pub other_port: u16,
}

pub async fn world() -> World {
    pki();
    // The source first: the database made last is the one this thread's tokens and routes use.
    let src = TestDb::new();
    let db = TestDb::new();
    let cfg = postgres::Config::from_str(src.url()).expect("the source url");
    let user = cfg.get_user().unwrap_or("postgres").to_string();
    let password = cfg.get_password().map(|p| String::from_utf8_lossy(p).to_string()).unwrap_or_else(|| "unused".to_string());
    let database = cfg.get_dbname().unwrap_or("postgres").to_string();
    let host = match cfg.get_hosts().first() {
        Some(postgres::config::Host::Tcp(h)) => h.clone(),
        _ => "127.0.0.1".to_string(),
    };
    let port = cfg.get_ports().first().copied().unwrap_or(5432);
    let backend: SocketAddr = format!("{host}:{port}").parse().unwrap_or_else(|_| SocketAddr::new("127.0.0.1".parse().unwrap(), port));
    src.run(|c| {
        c.batch_execute(
            "CREATE TABLE programs (code text PRIMARY KEY, name text, credits integer);
             CREATE TABLE audit (note text);
             CREATE FUNCTION touch() RETURNS text AS $$ BEGIN INSERT INTO audit VALUES ('touched'); RETURN 'T'; END $$ LANGUAGE plpgsql VOLATILE;",
        )
        .unwrap()
    });
    let good_port = proxy(DB_HOST, backend).await;
    let other_port = proxy(OTHER_HOST, backend).await;

    let mut state = ServerState::connect(db.url()).expect("server");
    let key = key_file();
    state.master_key_file = Some(key.clone());
    state.egress_resolver = Some(Arc::new(To("127.0.0.1".parse().unwrap())));
    let state = Arc::new(state);
    state.seed_demo().expect("seed");
    let master = scaffoldry_server::secrets::MasterKey::from_file(&key).unwrap();
    scaffoldry_server::secrets::put(&state.repository, &master, SECRET, password.as_bytes(), ADMIN).unwrap();
    let app = build_app_with_state(state.clone()).expect("router");
    let w = World { db, state, app, src, database, user, password, good_port, other_port };
    w.allow(&[w.good_port, w.other_port]);
    w
}

impl World {
    /// The allowlist: the test host on these ports, and the loopback network on purpose.
    pub fn allow(&self, ports: &[u16]) {
        self.state.settings.write().unwrap().insert("egress.allow".into(), json!([{ "host": DB_HOST, "ports": ports }, { "cidr": "127.0.0.1/32", "ports": ports }]));
    }

    /// The allowlist with the host and no network, so a name that resolves to loopback is blocked.
    pub fn allow_host_only(&self, ports: &[u16]) {
        self.state.settings.write().unwrap().insert("egress.allow".into(), json!([{ "host": DB_HOST, "ports": ports }]));
    }

    pub fn allow_nothing(&self) {
        self.state.settings.write().unwrap().insert("egress.allow".into(), json!([]));
    }
}

pub async fn send(app: &Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {}", issue_test_token_and_user(who)));
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

pub fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", &uuid::Uuid::new_v4().simple().to_string()[..8])
}

pub struct Conn {
    pub id: String,
    pub app: String,
}

/// The body of a new connection to the source. `tls` and `query` are the parts the tests change.
pub fn body(w: &World, port: u16, tls: &str, query: &str, extra: Value) -> Value {
    let mut config = json!({ "host": DB_HOST, "port": port, "database": w.database, "user": w.user, "tls": tls, "query": query });
    if let (Some(c), Some(e)) = (config.as_object_mut(), extra.as_object()) {
        c.extend(e.clone());
    }
    json!({
        "name": unique("Source"), "kind": "postgres", "secret": SECRET, "reason": "The registrar's database", "config": config,
        "dataset_app": unique("pgprograms"), "dataset_table": "programs", "key_field": "code",
        "column_map": [
            { "source": "code", "field": "code", "type": "Text" },
            { "source": "name", "field": "name", "type": "Text" },
            { "source": "credits", "field": "credits", "type": "Number" },
        ],
        "sensitivity": "Directory",
    })
}

pub const ALL: &str = "SELECT code, name, credits FROM programs";

pub async fn connect(w: &World, body: Value) -> (StatusCode, Value, String) {
    let app = body["dataset_app"].as_str().unwrap().to_string();
    let (s, b) = send(&w.app, "POST", "/api/v1/admin/connections", ADMIN, Some(body)).await;
    (s, b, app)
}

pub async fn connect_ok(w: &World, body: Value) -> Conn {
    let (s, b, app) = connect(w, body).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    Conn { id: b["connection"]["id"].as_str().unwrap().to_string(), app }
}

pub async fn run(w: &World, c: &Conn) -> Value {
    let (s, b) = send(&w.app, "POST", &format!("/api/v1/admin/connections/{}/run", c.id), ADMIN, Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    b["run"].clone()
}

pub fn live(w: &World, app: &str) -> i64 {
    let app = app.to_string();
    w.db.run(move |c| c.query_one("SELECT COUNT(*) FROM dataset_records WHERE app_slug = $1 AND deleted_at IS NULL", &[&app]).unwrap().get(0))
}

pub fn seed(w: &World, rows: i64) {
    w.src.run(move |c| {
        c.execute("INSERT INTO programs SELECT 'P' || lpad(g::text, 6, '0'), 'Program ' || g, 3 + (g % 3)::int FROM generate_series(1, $1::bigint) g", &[&rows]).unwrap();
    });
}
