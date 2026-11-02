//! Notifications phase 2: the one way mail leaves the server.
//!
//! The host is checked and resolved by `egress::resolve_and_check`, once. The connection goes to that address, and TLS
//! checks the certificate against the host's name, from the system certificate store. There is no plain-text mode:
//! `tls` speaks TLS from the first byte (RFC 8314), and `starttls` refuses a server that does not offer STARTTLS
//! (RFC 3207). There is no connection pool: each run opens one connection and closes it.
//!
//! An error never carries the server's own words, because a server may repeat back what it was sent. It says what
//! failed and the reply code, and nothing else.

use super::egress::{resolve_and_check, Allow, EgressError, Resolver};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{SmtpConnection, TlsParameters};
use lettre::transport::smtp::extension::ClientId;
use std::time::Duration;

/// How long one step of the conversation may take.
pub const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// TLS from the first byte, usually port 465.
    Tls,
    /// A plain connection that must turn into TLS before anything else, usually port 587.
    StartTls,
}

/// Where mail goes and how the server signs in to it.
pub struct Relay<'a> {
    pub host: &'a str,
    pub port: u16,
    pub security: Security,
    /// The name this server gives in `EHLO`.
    pub hello: &'a str,
    /// The user name and password, when the relay wants a sign-in.
    pub credentials: Option<(&'a str, &'a [u8])>,
}

/// An open connection to the relay. It sends one message after another and is closed when dropped.
pub struct Session {
    conn: SmtpConnection,
}

fn refused(what: &str, e: &lettre::transport::smtp::Error) -> String {
    let code = e.status().map(|c| format!(" ({c})")).unwrap_or_default();
    let kind = if e.is_timeout() {
        "timed out"
    } else if e.is_tls() {
        "failed in TLS"
    } else if e.is_permanent() {
        "was refused for good"
    } else if e.is_transient() {
        "was refused for now"
    } else {
        "failed"
    };
    format!("{what} {kind}{code}")
}

impl Session {
    /// Checks the host against the allowlist, resolves it once, connects to that address, turns on TLS and signs in.
    pub fn open(relay: &Relay<'_>, allow: &Allow, resolver: &dyn Resolver) -> Result<Session, String> {
        let addr = resolve_and_check(relay.host, relay.port, allow, resolver).map_err(|e| match e {
            EgressError::NotAllowed(m) | EgressError::Blocked(m) | EgressError::Failed(m) | EgressError::Redirect(m) | EgressError::TooLarge(m) => m,
        })?;
        let tls = TlsParameters::builder(relay.host.trim_end_matches('.').to_string()).build_rustls().map_err(|e| refused("Setting up TLS", &e))?;
        let hello = ClientId::Domain(relay.hello.to_string());
        let mut conn = match relay.security {
            Security::Tls => SmtpConnection::connect(addr, Some(TIMEOUT), &hello, Some(&tls), None).map_err(|e| refused("Connecting to the mail server", &e))?,
            Security::StartTls => {
                let mut conn = SmtpConnection::connect(addr, Some(TIMEOUT), &hello, None, None).map_err(|e| refused("Connecting to the mail server", &e))?;
                if !conn.can_starttls() {
                    return Err("The mail server does not offer STARTTLS, and mail is never sent in the clear".to_string());
                }
                conn.starttls(&tls, &hello).map_err(|e| refused("Starting TLS", &e))?;
                conn
            }
        };
        if let Some((user, password)) = relay.credentials {
            let password = String::from_utf8(password.to_vec()).map_err(|_| "The mail password is not text".to_string())?;
            conn.auth(&[Mechanism::Plain, Mechanism::Login], &Credentials::new(user.to_string(), password)).map_err(|e| refused("Signing in to the mail server", &e))?;
        }
        Ok(Session { conn })
    }

    /// Sends one message.
    pub fn send(&mut self, message: &lettre::Message) -> Result<(), String> {
        self.conn.send(message.envelope(), &message.formatted()).map(|_| ()).map_err(|e| refused("Sending", &e))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.conn.quit();
    }
}
