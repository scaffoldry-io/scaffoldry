//! Integrations, phase 1: everything that leaves goes through `net::egress`.
//!
//! The server connects only to hosts a Platform Admin allowlisted. A name is resolved once and the connection goes to
//! that address, so a second lookup cannot steer it somewhere else. Private, loopback, link-local (the cloud metadata
//! address among them), unique-local and carrier-grade NAT addresses are refused unless a `cidr` entry names that
//! network on purpose. No redirect is followed. No test here needs a network: the resolver is injected, and the two
//! tests that connect do so to a listener on this machine, through a `cidr` entry that allows it. NIST SP 800-53 SC-7,
//! AC-4; OWASP SSRF Prevention Cheat Sheet.

use scaffoldry_server::net::egress::{self, Allow, Client, EgressError, Resolver};
use serde_json::json;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// A resolver that answers from a script: the nth lookup of a name gets the nth answer, and the last one after that.
struct Script {
    answers: Vec<Vec<IpAddr>>,
    calls: AtomicUsize,
}

impl Script {
    fn new(answers: &[&[&str]]) -> Arc<Self> {
        Arc::new(Script {
            answers: answers.iter().map(|a| a.iter().map(|s| s.parse().unwrap()).collect()).collect(),
            calls: AtomicUsize::new(0),
        })
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Resolver for Script {
    fn lookup(&self, _host: &str, _port: u16) -> std::io::Result<Vec<IpAddr>> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.answers[n.min(self.answers.len() - 1)].clone())
    }
}

fn allow(entries: serde_json::Value) -> Allow {
    Allow::parse(&entries).expect("a valid egress.allow")
}

/// A listener on this machine that answers each request with `reply`, and the port it listens on.
fn local_server(reply: &'static str) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().take(4) {
            let Ok(mut s) = stream else { continue };
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let _ = s.write_all(reply.as_bytes());
        }
    });
    port
}

// 1. A name that resolves to a private address is refused unless a `cidr` entry covers it.
#[test]
fn a_name_that_resolves_to_a_private_address_is_refused_unless_a_cidr_entry_covers_it() {
    let dns = Script::new(&[&["10.20.3.4"]]);
    let by_name = allow(json!([{ "host": "sis.example.edu", "ports": [443] }]));
    let err = egress::resolve_and_check("sis.example.edu", 443, &by_name, dns.as_ref()).unwrap_err();
    assert!(matches!(err, EgressError::Blocked(_)), "{err:?}");

    let with_cidr = allow(json!([{ "host": "sis.example.edu", "ports": [443] }, { "cidr": "10.20.0.0/16", "ports": [443] }]));
    let addr = egress::resolve_and_check("sis.example.edu", 443, &with_cidr, dns.as_ref()).unwrap();
    assert_eq!(addr.to_string(), "10.20.3.4:443");

    // A `cidr` entry for another network does not help.
    let other = allow(json!([{ "host": "sis.example.edu", "ports": [443] }, { "cidr": "10.30.0.0/16", "ports": [443] }]));
    assert!(egress::resolve_and_check("sis.example.edu", 443, &other, dns.as_ref()).is_err());
}

// 2. The address is pinned. The name is looked up once, and the connection goes to that address even when a later
// lookup would say something else. The name below does not exist in any DNS, so a response proves the client
// connected to the pinned address and did not look the name up again.
#[test]
fn the_connection_goes_to_the_address_the_check_passed_and_the_name_is_not_looked_up_again() {
    let port = local_server("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
    // First lookup: the local listener, which the `cidr` entry allows. A second lookup would answer the metadata address.
    let dns = Script::new(&[&["127.0.0.1"], &["169.254.169.254"]]);
    let rules = allow(json!([{ "host": "pinned.invalid", "ports": [port] }, { "cidr": "127.0.0.0/8", "ports": [port] }]));
    let client = Client::new(rules, dns.clone());
    let resp = client.get(&format!("http://pinned.invalid:{port}/")).expect("served by the pinned address");
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body, b"ok");
    assert_eq!(dns.calls(), 1, "the name is resolved once");

    // The check itself picks the first address and keeps it.
    let dns = Script::new(&[&["93.184.216.34"], &["10.0.0.1"]]);
    let rules = allow(json!([{ "host": "feed.example.org", "ports": [443] }]));
    let first = egress::resolve_and_check("feed.example.org", 443, &rules, dns.as_ref()).unwrap();
    assert_eq!(first.to_string(), "93.184.216.34:443");
    assert_eq!(dns.calls(), 1);
}

// 3. The cloud metadata address, 127.0.0.1, ::1, and 169.254.0.0/16 are refused.
#[test]
fn loopback_link_local_and_the_metadata_address_are_refused() {
    for target in ["169.254.169.254", "127.0.0.1", "::1", "169.254.10.20", "fe80::1", "100.64.1.1", "192.168.1.1", "172.16.5.5", "fd00:ec2::254", "0.0.0.0", "::ffff:127.0.0.1"] {
        let dns = Script::new(&[&[target]]);
        let rules = allow(json!([{ "host": "looks-fine.example.org", "ports": [443] }]));
        let err = egress::resolve_and_check("looks-fine.example.org", 443, &rules, dns.as_ref()).unwrap_err();
        assert!(matches!(err, EgressError::Blocked(_)), "{target}: {err:?}");
    }
    // An address written as the host is checked the same way.
    let dns = Script::new(&[&["1.1.1.1"]]);
    let rules = allow(json!([{ "host": "169.254.169.254", "ports": [80] }]));
    assert!(matches!(egress::resolve_and_check("169.254.169.254", 80, &rules, dns.as_ref()), Err(EgressError::Blocked(_))));
}

// 4. A host or port not on the list is refused. An empty list refuses everything.
#[test]
fn a_host_or_port_not_on_the_list_is_refused_and_an_empty_list_refuses_everything() {
    let dns = Script::new(&[&["93.184.216.34"]]);
    let rules = allow(json!([{ "host": "feed.example.org", "ports": [443] }]));
    assert!(egress::resolve_and_check("feed.example.org", 443, &rules, dns.as_ref()).is_ok());
    // Names compare without case, and only whole names match.
    assert!(egress::resolve_and_check("FEED.example.org", 443, &rules, dns.as_ref()).is_ok());
    for (host, port) in [("feed.example.org", 80), ("other.example.org", 443), ("evil-feed.example.org", 443), ("feed.example.org.evil.net", 443)] {
        let err = egress::resolve_and_check(host, port, &rules, dns.as_ref()).unwrap_err();
        assert!(matches!(err, EgressError::NotAllowed(_)), "{host}:{port}: {err:?}");
    }
    let empty = Allow::parse(&json!([])).unwrap();
    assert!(matches!(egress::resolve_and_check("feed.example.org", 443, &empty, dns.as_ref()), Err(EgressError::NotAllowed(_))));
    // The client refuses before it opens anything.
    let client = Client::new(empty, dns.clone());
    assert!(matches!(client.get("https://feed.example.org/"), Err(EgressError::NotAllowed(_))));
}

#[test]
fn the_setting_is_checked_before_it_is_used() {
    for bad in [json!({}), json!([{ "host": "a.example" }]), json!([{ "cidr": "10.0.0.0/33", "ports": [1] }]), json!([{ "host": "a.example", "ports": [0] }]), json!([{ "host": "a.example", "cidr": "10.0.0.0/8", "ports": [1] }]), json!([{ "cidr": "not-a-network", "ports": [1] }])] {
        assert!(Allow::parse(&bad).is_err(), "{bad}");
    }
    assert!(Allow::parse(&json!([{ "host": "a.example", "ports": [443] }, { "cidr": "fd12:3456::/48", "ports": [5432] }])).is_ok());
}

// 5. A redirect is a failure.
#[test]
fn a_redirect_is_a_failure() {
    let port = local_server("HTTP/1.1 302 Found\r\nLocation: http://169.254.169.254/latest/meta-data/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let dns = Script::new(&[&["127.0.0.1"]]);
    let rules = allow(json!([{ "host": "hop.invalid", "ports": [port] }, { "cidr": "127.0.0.0/8", "ports": [port] }]));
    let client = Client::new(rules, dns.clone());
    let err = client.get(&format!("http://hop.invalid:{port}/")).unwrap_err();
    assert!(matches!(err, EgressError::Redirect(_)), "{err:?}");
    assert_eq!(dns.calls(), 1, "the redirect target was not looked up");
}

// 8. No outbound connection exists outside `net::egress`. The scan reads every crate's `src`. Tests are left out: they
// connect to the server under test, which is not the server leaving.
#[test]
fn no_code_outside_net_egress_opens_a_connection() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let patterns = [concat!("ureq", "::"), concat!("TcpStream", "::connect"), concat!("req", "west"), concat!("UdpSocket", "::bind")];
    let mut found = Vec::new();
    for krate in std::fs::read_dir(&root).unwrap().flatten() {
        let src = krate.path().join("src");
        if src.is_dir() {
            scan(&src, &patterns, &mut found);
        }
    }
    let outside: Vec<&String> = found.iter().filter(|f| !f.contains("src/net/egress.rs:")).collect();
    assert!(outside.is_empty(), "outbound connections outside net::egress: {outside:#?}");
    assert!(found.iter().any(|f| f.contains("src/net/egress.rs:")), "net::egress is where the client lives");
}

fn scan(dir: &std::path::Path, patterns: &[&str], found: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() {
            scan(&p, patterns, found);
        } else if p.extension().is_some_and(|e| e == "rs") {
            for (n, line) in std::fs::read_to_string(&p).unwrap().lines().enumerate() {
                if patterns.iter().any(|pat| line.contains(pat)) {
                    found.push(format!("{}:{}", p.display(), n + 1));
                }
            }
        }
    }
}
