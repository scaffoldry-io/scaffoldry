//! Integrations phase 1: the one way out of the server.
//!
//! Every outbound connection goes through here, and a test scans the source to keep it that way. The rules:
//!
//! - A connection may go only where the setting `egress.allow` says: a host and its ports, or a network (`cidr`) and its
//!   ports. The list is empty by default, so nothing leaves until a Platform Admin adds an entry.
//! - A name is resolved once. The connection then goes to that address, never to the name again, so a second lookup
//!   cannot steer it elsewhere (DNS rebinding). TLS still checks the certificate against the name.
//! - Loopback, private, link-local (the cloud metadata address is one), unique-local, carrier-grade NAT, multicast and
//!   unspecified addresses are refused unless a `cidr` entry names that network on purpose.
//! - No redirect is followed, and a redirect is a failure, unless the caller allows a few (`FetchOptions::max_redirects`).
//!   Then each hop is resolved and checked against the list again, and none may go from https to http.
//! - 5 seconds to connect, 30 in all, and at most 5 MB of response, unless the caller asks for less.
//!
//! NIST SP 800-53 SC-7 and AC-4; the OWASP Server-Side Request Forgery Prevention Cheat Sheet.

use serde_json::Value;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;
use ureq::http::Uri;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_RESPONSE_BYTES: u64 = 5 * 1024 * 1024;

/// Why a request did not go out, or did not come back.
#[derive(Debug)]
pub enum EgressError {
    /// The host and port are not on the outbound allowlist.
    NotAllowed(String),
    /// The address is one the server does not reach unless a `cidr` entry names it.
    Blocked(String),
    /// The answer was a redirect. Redirects are not followed.
    Redirect(String),
    /// The answer was larger than the limit.
    TooLarge(String),
    /// The name did not resolve, the connection failed, or the answer could not be read.
    Failed(String),
}

impl std::fmt::Display for EgressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EgressError::NotAllowed(m) | EgressError::Blocked(m) | EgressError::Redirect(m) | EgressError::TooLarge(m) | EgressError::Failed(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for EgressError {}

/// Turns a name into addresses. Tests inject their own, so no test needs a network.
pub trait Resolver: Send + Sync {
    fn lookup(&self, host: &str, port: u16) -> std::io::Result<Vec<IpAddr>>;
}

/// The operating system's resolver.
pub struct SystemResolver;

impl Resolver for SystemResolver {
    fn lookup(&self, host: &str, port: u16) -> std::io::Result<Vec<IpAddr>> {
        Ok((host, port).to_socket_addrs()?.map(|a| a.ip()).collect())
    }
}

/// A network: an address and a prefix length.
#[derive(Debug, Clone)]
struct Net {
    addr: IpAddr,
    prefix: u8,
}

impl Net {
    fn parse(s: &str) -> Option<Net> {
        let (addr, prefix) = s.split_once('/')?;
        let addr: IpAddr = addr.trim().parse().ok()?;
        let prefix: u8 = prefix.trim().parse().ok()?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        (prefix <= max).then_some(Net { addr: canonical(addr), prefix })
    }

    fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, canonical(ip)) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => masked(u32::from(net).into(), 32, self.prefix) == masked(u32::from(ip).into(), 32, self.prefix),
            (IpAddr::V6(net), IpAddr::V6(ip)) => masked(u128::from(net), 128, self.prefix) == masked(u128::from(ip), 128, self.prefix),
            _ => false,
        }
    }
}

fn masked(bits: u128, width: u8, prefix: u8) -> u128 {
    if prefix == 0 {
        return 0;
    }
    let shift = u32::from(width - prefix);
    (bits >> shift) << shift
}

/// An IPv6 address that carries an IPv4 one is judged as that IPv4 address.
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
        v4 => v4,
    }
}

fn in_v4(ip: Ipv4Addr, net: [u8; 4], prefix: u8) -> bool {
    masked(u32::from(ip).into(), 32, prefix) == masked(u32::from(Ipv4Addr::from(net)).into(), 32, prefix)
}

fn in_v6(ip: Ipv6Addr, net: u128, prefix: u8) -> bool {
    masked(u128::from(ip), 128, prefix) == masked(net, 128, prefix)
}

/// The kind of address the server does not reach unless a `cidr` entry names it, or `None` for a public one.
fn special(ip: IpAddr) -> Option<&'static str> {
    match canonical(ip) {
        IpAddr::V4(v4) => {
            let ranges: [([u8; 4], u8, &str); 9] = [
                ([0, 0, 0, 0], 8, "unspecified"),
                ([127, 0, 0, 0], 8, "loopback"),
                ([10, 0, 0, 0], 8, "private"),
                ([172, 16, 0, 0], 12, "private"),
                ([192, 168, 0, 0], 16, "private"),
                ([169, 254, 0, 0], 16, "link-local"),
                ([100, 64, 0, 0], 10, "carrier-grade NAT"),
                ([224, 0, 0, 0], 4, "multicast"),
                ([240, 0, 0, 0], 4, "reserved"),
            ];
            ranges.iter().find(|(net, prefix, _)| in_v4(v4, *net, *prefix)).map(|(_, _, kind)| *kind)
        }
        IpAddr::V6(v6) => {
            let ranges: [(u128, u8, &str); 6] = [
                (0, 128, "unspecified"),
                (1, 128, "loopback"),
                (0xfe80 << 112, 10, "link-local"),
                (0xfec0 << 112, 10, "site-local"),
                (0xfc00 << 112, 7, "unique-local"),
                (0xff00 << 112, 8, "multicast"),
            ];
            ranges.iter().find(|(net, prefix, _)| in_v6(v6, *net, *prefix)).map(|(_, _, kind)| *kind)
        }
    }
}

#[derive(Debug, Clone)]
enum Entry {
    Host { host: String, ports: Vec<u16> },
    Cidr { net: Net, ports: Vec<u16> },
}

/// The outbound allowlist: the value of the setting `egress.allow`.
#[derive(Debug, Clone, Default)]
pub struct Allow {
    entries: Vec<Entry>,
}

fn normal_host(host: &str) -> String {
    host.trim().trim_start_matches('[').trim_end_matches(']').trim_end_matches('.').to_ascii_lowercase()
}

impl Allow {
    /// Reads and checks the setting's value: an array of `{ "host", "ports" }` and `{ "cidr", "ports" }`.
    pub fn parse(value: &Value) -> Result<Allow, String> {
        let items = value.as_array().ok_or("egress.allow must be a list")?;
        let mut entries = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let obj = item.as_object().ok_or_else(|| format!("entry {i} must be an object"))?;
            if obj.keys().any(|k| k != "host" && k != "cidr" && k != "ports") {
                return Err(format!("entry {i} may hold only host or cidr, and ports"));
            }
            let ports: Vec<u16> = obj
                .get("ports")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("entry {i} needs ports"))?
                .iter()
                .map(|p| p.as_u64().filter(|p| (1..=65535).contains(p)).map(|p| p as u16))
                .collect::<Option<Vec<u16>>>()
                .filter(|p| !p.is_empty())
                .ok_or_else(|| format!("entry {i} needs ports from 1 to 65535"))?;
            match (obj.get("host").and_then(Value::as_str), obj.get("cidr").and_then(Value::as_str)) {
                (Some(host), None) => {
                    let host = normal_host(host);
                    if host.is_empty() || host.len() > 253 || host.contains(|c: char| c.is_whitespace() || c == '/' || c == '@' || c == '*') {
                        return Err(format!("entry {i} names no valid host"));
                    }
                    entries.push(Entry::Host { host, ports });
                }
                (None, Some(cidr)) => {
                    let net = Net::parse(cidr).ok_or_else(|| format!("entry {i} names no valid network"))?;
                    entries.push(Entry::Cidr { net, ports });
                }
                _ => return Err(format!("entry {i} needs exactly one of host or cidr")),
            }
        }
        Ok(Allow { entries })
    }

    /// The allowlist the server holds now. A missing or unreadable setting allows nothing.
    pub fn from_settings(settings: &std::collections::HashMap<String, Value>) -> Allow {
        settings.get("egress.allow").and_then(|v| Allow::parse(v).ok()).unwrap_or_default()
    }

    /// Whether `host` and `port` are on the list by name. A proposal checks a webhook's receiver with this.
    pub fn lists_host(&self, host: &str, port: u16) -> bool {
        self.host_allowed(&normal_host(host), port)
    }

    fn host_allowed(&self, host: &str, port: u16) -> bool {
        self.entries.iter().any(|e| matches!(e, Entry::Host { host: h, ports } if h == host && ports.contains(&port)))
    }

    fn cidr_covers(&self, ip: IpAddr, port: u16) -> bool {
        self.entries.iter().any(|e| matches!(e, Entry::Cidr { net, ports } if net.contains(ip) && ports.contains(&port)))
    }
}

/// Reads an address without opening anything. Code outside this module that only needs to know what an address says
/// asks here, so no other module names the HTTP client.
pub fn read_address(url: &str) -> Option<Address> {
    let uri: Uri = url.parse().ok()?;
    let host = uri.host()?.to_string();
    Some(Address { https: uri.scheme_str() == Some("https"), host, has_credentials: uri.authority().is_some_and(|a| a.as_str().contains('@')) })
}

/// What an address says.
#[derive(Debug, Clone)]
pub struct Address {
    pub https: bool,
    pub host: String,
    /// The address carries a user name or a password.
    pub has_credentials: bool,
}

/// Resolves `host` once and returns the address to connect to, or why not. The first address that passes is kept: the
/// caller connects to it and never looks the name up again.
pub fn resolve_and_check(host: &str, port: u16, allow: &Allow, resolver: &dyn Resolver) -> Result<SocketAddr, EgressError> {
    let name = normal_host(host);
    let named = allow.host_allowed(&name, port);
    let addresses = match name.parse::<IpAddr>() {
        Ok(ip) => vec![ip],
        Err(_) => {
            // A name that is on no entry, and that no network entry could cover, is refused before any lookup.
            if !named && !allow.entries.iter().any(|e| matches!(e, Entry::Cidr { ports, .. } if ports.contains(&port))) {
                return Err(EgressError::NotAllowed(format!("{name}:{port} is not on the outbound allowlist")));
            }
            resolver.lookup(&name, port).map_err(|_| EgressError::Failed(format!("{name} could not be resolved")))?
        }
    };
    if addresses.is_empty() {
        return Err(EgressError::Failed(format!("{name} could not be resolved")));
    }
    let mut blocked: Option<&str> = None;
    for ip in addresses {
        let ip = canonical(ip);
        let covered = allow.cidr_covers(ip, port);
        if let Some(kind) = special(ip) {
            if !covered {
                blocked.get_or_insert(kind);
                continue;
            }
        }
        if named || covered {
            return Ok(SocketAddr::new(ip, port));
        }
    }
    Err(match blocked {
        Some(kind) => EgressError::Blocked(format!("{name} resolves to a {kind} address, which no cidr entry allows")),
        None => EgressError::NotAllowed(format!("{name}:{port} is not on the outbound allowlist")),
    })
}

/// An answer: the status, the media type, and at most the limit of body.
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    /// The `Content-Type` header, as sent.
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

/// How one request may go. The defaults are the module's limits and no redirects.
#[derive(Debug, Clone)]
pub struct FetchOptions {
    pub total_timeout: Duration,
    pub max_bytes: u64,
    /// How many redirects to follow, each checked against the list as a new request is.
    pub max_redirects: u8,
    pub headers: Vec<(String, String)>,
}

impl Default for FetchOptions {
    fn default() -> Self {
        FetchOptions { total_timeout: TOTAL_TIMEOUT, max_bytes: MAX_RESPONSE_BYTES, max_redirects: 0, headers: Vec::new() }
    }
}

/// Where a redirect points, from the page that sent it. `None` when it names nothing usable.
fn redirect_target(from: &Uri, location: &str) -> Option<String> {
    let location = location.trim();
    if location.starts_with("https://") || location.starts_with("http://") {
        return Some(location.to_string());
    }
    if location.starts_with('/') && !location.starts_with("//") {
        let (scheme, authority) = (from.scheme_str()?, from.authority()?);
        return Some(format!("{scheme}://{authority}{location}"));
    }
    None
}

/// The address the check passed, handed to the HTTP client in place of its own lookup.
#[derive(Debug)]
struct Pinned(SocketAddr);

impl ureq::unversioned::resolver::Resolver for Pinned {
    fn resolve(
        &self,
        _uri: &Uri,
        _config: &ureq::config::Config,
        _timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        let mut addrs = self.empty();
        addrs.push(self.0);
        Ok(addrs)
    }
}

/// The outbound HTTP client. Its only way to open a connection is through `resolve_and_check`.
pub struct Client {
    allow: Allow,
    resolver: Arc<dyn Resolver>,
}

impl Client {
    pub fn new(allow: Allow, resolver: Arc<dyn Resolver>) -> Self {
        Client { allow, resolver }
    }

    /// A client with the allowlist this server holds and the system resolver.
    pub fn for_state(state: &crate::state::ServerState) -> Self {
        let settings = state.settings.read().unwrap_or_else(|p| p.into_inner());
        Client::new(Allow::from_settings(&settings), state.egress_resolver.clone().unwrap_or_else(|| Arc::new(SystemResolver)))
    }

    pub fn get(&self, url: &str) -> Result<Response, EgressError> {
        self.fetch(url, &FetchOptions::default())
    }

    pub fn post(&self, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<Response, EgressError> {
        self.send("POST", url, headers, Some(body), &FetchOptions::default())
    }

    /// A `GET` with these limits. Redirects, when allowed, are followed one hop at a time, and each hop is a new request
    /// through the same checks. A redirect from https to http is refused.
    pub fn fetch(&self, url: &str, options: &FetchOptions) -> Result<Response, EgressError> {
        let headers: Vec<(&str, &str)> = options.headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let mut current = url.to_string();
        for hop in 0..=options.max_redirects {
            match self.send("GET", &current, &headers, None, options) {
                Err(EgressError::Redirect(location)) if hop < options.max_redirects => {
                    let from: Uri = current.parse().map_err(|_| EgressError::Failed("The address is not a valid URL".to_string()))?;
                    let next = redirect_target(&from, &location).ok_or_else(|| EgressError::Failed("A redirect named no usable address".to_string()))?;
                    if from.scheme_str() == Some("https") && next.starts_with("http://") {
                        return Err(EgressError::NotAllowed("A redirect from https to http is not followed".to_string()));
                    }
                    current = next;
                }
                Err(EgressError::Redirect(_)) if options.max_redirects == 0 => {
                    return Err(EgressError::Redirect("The answer was a redirect, which is not followed".to_string()))
                }
                Err(EgressError::Redirect(_)) => return Err(EgressError::Redirect(format!("More than {} redirects", options.max_redirects))),
                other => return other,
            }
        }
        Err(EgressError::Redirect(format!("More than {} redirects", options.max_redirects)))
    }

    fn send(&self, method: &str, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>, options: &FetchOptions) -> Result<Response, EgressError> {
        let uri: Uri = url.parse().map_err(|_| EgressError::Failed("The address is not a valid URL".to_string()))?;
        let default_port = match uri.scheme_str() {
            Some("https") => 443,
            Some("http") => 80,
            _ => return Err(EgressError::NotAllowed("Only http and https addresses are allowed".to_string())),
        };
        let host = uri.host().ok_or_else(|| EgressError::Failed("The address names no host".to_string()))?;
        let port = uri.port_u16().unwrap_or(default_port);
        let addr = resolve_and_check(host, port, &self.allow, self.resolver.as_ref())?;
        let agent = agent(addr, options.total_timeout);
        let mut answer = match (method, body) {
            ("POST", Some(body)) => {
                let mut req = agent.post(url);
                for (k, v) in headers {
                    req = req.header(*k, *v);
                }
                req.send(body)
            }
            _ => {
                let mut req = agent.get(url);
                for (k, v) in headers {
                    req = req.header(*k, *v);
                }
                req.call()
            }
        }
        .map_err(|e| EgressError::Failed(format!("The request to {host} failed: {}", short(&e))))?;
        let status = answer.status().as_u16();
        if (300..400).contains(&status) {
            // The target travels in the error, for `fetch` to check as a new request. Nothing follows it here.
            let location = answer.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
            return Err(EgressError::Redirect(location));
        }
        let content_type = answer.headers().get("content-type").and_then(|v| v.to_str().ok()).map(str::to_string);
        let limit = options.max_bytes;
        let body = answer.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| match e {
            ureq::Error::BodyExceedsLimit(_) => EgressError::TooLarge(format!("The answer from {host} is larger than {} KB", limit / 1024)),
            e => EgressError::Failed(format!("The answer from {host} could not be read: {}", short(&e))),
        })?;
        Ok(Response { status, content_type, body })
    }
}

/// An error's own words, without anything it may have carried from the request.
fn short(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Timeout(_) => "it timed out".to_string(),
        ureq::Error::Io(io) => io.kind().to_string(),
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => "the TLS handshake failed".to_string(),
        ureq::Error::ConnectionFailed => "the connection failed".to_string(),
        _ => "the connection failed".to_string(),
    }
}

/// An HTTP client for one request, pinned to `addr`. No proxy, no redirects, rustls on ring, and the system's
/// certificate store.
fn agent(addr: SocketAddr, total: Duration) -> ureq::Agent {
    let roots: Vec<ureq::tls::Certificate<'static>> =
        rustls_native_certs::load_native_certs().certs.iter().map(|c| ureq::tls::Certificate::from_der(c.as_ref()).to_owned()).collect();
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::Rustls)
        .root_certs(ureq::tls::RootCerts::Specific(Arc::new(roots)))
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .build();
    let config = ureq::Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_global(Some(total.min(TOTAL_TIMEOUT)))
        .tls_config(tls)
        .build();
    ureq::Agent::with_parts(config, ureq::unversioned::transport::DefaultConnector::new(), Pinned(addr))
}
