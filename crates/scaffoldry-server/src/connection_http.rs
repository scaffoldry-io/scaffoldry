//! Connections phase 4: the HTTP connector.
//!
//! A connection reads a JSON or JSON Lines web service and syncs its rows into a dataset.
//!
//! - **https only, and only to the allowlist.** Every page goes through `net::egress`, so the address is checked and
//!   pinned. The address a page names as the next one is checked the same way, so a service cannot send the sync
//!   somewhere the allowlist does not name.
//! - **No redirects.** A redirect is a failure, and is not followed.
//! - **Bounded.** 5 MB for a page, 100 pages for a run, and 10 minutes in all.
//! - **Checked before it is written.** Every page is read first. The rows are kept in a private temporary file, so
//!   memory stays flat and the service is asked once. Then the rows are checked, counted and written, as for any source.
//! - **A bearer token** may come from the secrets store. It is sent in a header, and taken out of any message.
//!
//! NIST SP 800-53 SC-7, SC-8 and AC-4.

use crate::connection_feed::{drive, Feed};
use crate::connections::{Conn, Counts, Fault, RunOpts};
use crate::net::egress::{Client, EgressError, FetchOptions};
use crate::state::SharedState;
use serde_json::Value;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Pages a run reads.
pub const MAX_PAGES: usize = 100;
/// How long a run may take.
pub const TOTAL_SECS: u64 = 600;
/// Rows handed over at once.
const BATCH_ROWS: usize = 5_000;

fn text<'a>(config: &'a Value, key: &str) -> Option<&'a str> {
    config.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// The parts of an http connection's `config`, checked. Nothing connects.
pub fn check_config(config: &Value) -> Result<(), String> {
    let url = text(config, "url").ok_or("config.url is the address of the service")?;
    let address = crate::net::egress::read_address(url).ok_or("config.url is not an address")?;
    if !address.https {
        return Err("config.url must be an https address. Plain http is not used".to_string());
    }
    if address.has_credentials {
        return Err("config.url may not carry a user name or password. Use a secret".to_string());
    }
    let format = text(config, "format").unwrap_or("");
    if !matches!(format, "json" | "jsonl") {
        return Err("config.format is json or jsonl".to_string());
    }
    for key in ["path", "next"] {
        if let Some(v) = config.get(key).filter(|v| !v.is_null()) {
            v.as_str().filter(|s| !s.trim().is_empty()).ok_or_else(|| format!("config.{key} is a path such as data or meta.next"))?;
        }
    }
    if format == "jsonl" && text(config, "next").is_some() {
        return Err("config.next needs json. A JSON Lines source is one page".to_string());
    }
    Ok(())
}

/// The value at a dotted path, such as `meta.next`. A number reads an array.
fn at<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(v, |cur, part| match part.parse::<usize>() {
        Ok(i) if cur.is_array() => cur.get(i),
        _ => cur.get(part),
    })
}

fn is_https(url: &str) -> bool {
    crate::net::egress::read_address(url).is_some_and(|a| a.https)
}

/// Rows read from a service, kept in a temporary file between the two passes.
struct HttpFeed {
    client: Client,
    url: String,
    json: bool,
    path: Option<String>,
    next: Option<String>,
    token: Option<String>,
    spool: PathBuf,
}

impl HttpFeed {
    fn scrub(&self, message: String) -> String {
        match &self.token {
            Some(t) if !t.is_empty() => message.replace(t.as_str(), "***"),
            _ => message,
        }
    }

    /// One page: its rows, and the address of the next.
    fn page(&self, url: &str, left: Duration) -> Result<(Vec<Value>, Option<String>), String> {
        let mut headers = vec![("Accept".to_string(), if self.json { "application/json" } else { "application/x-ndjson, application/json" }.to_string())];
        if let Some(t) = &self.token {
            headers.push(("Authorization".to_string(), format!("Bearer {t}")));
        }
        let opts = FetchOptions { total_timeout: left.min(Duration::from_secs(60)), headers, ..FetchOptions::default() };
        let resp = self.client.fetch(url, &opts).map_err(|e| match e {
            EgressError::Redirect(to) => format!("The source answered with a redirect to {to}. A redirect is a failure, and is not followed"),
            EgressError::TooLarge(m) => format!("A page was larger than the 5 MB limit ({m})"),
            other => other.to_string(),
        })?;
        if !(200..300).contains(&resp.status) {
            return Err(format!("The source answered {}", resp.status));
        }
        if !self.json {
            let body = std::str::from_utf8(&resp.body).map_err(|_| "The source did not answer in UTF-8".to_string())?;
            let mut rows = Vec::new();
            for (i, line) in body.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
                let v: Value = serde_json::from_str(line).map_err(|_| format!("Line {} of the answer is not JSON", i + 1))?;
                if !v.is_object() {
                    return Err(format!("Line {} of the answer is not an object", i + 1));
                }
                rows.push(v);
            }
            return Ok((rows, None));
        }
        let doc: Value = serde_json::from_slice(&resp.body).map_err(|_| "The source did not answer with JSON".to_string())?;
        let list = match &self.path {
            Some(p) => at(&doc, p).ok_or_else(|| format!("The answer has no list of rows at \"{p}\""))?,
            None => &doc,
        };
        let rows = list.as_array().ok_or_else(|| format!("The answer holds no list of rows at \"{}\"", self.path.as_deref().unwrap_or("the top")))?;
        if rows.iter().any(|r| !r.is_object()) {
            return Err("A row of the answer is not an object".to_string());
        }
        let next = match self.next.as_deref().and_then(|p| at(&doc, p)) {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if s.trim().is_empty() => None,
            Some(Value::String(s)) => {
                if !is_https(s) {
                    return Err("The next address is not an https address".to_string());
                }
                Some(s.clone())
            }
            Some(_) => return Err(format!("\"{}\" in the answer is not an address", self.next.as_deref().unwrap_or(""))),
        };
        Ok((rows.clone(), next))
    }

    /// First pass: asks the service for each page, and keeps its rows.
    fn fetch_all(&mut self, f: &mut dyn FnMut(Vec<Value>) -> Result<(), String>) -> Result<(), String> {
        let started = Instant::now();
        let mut out = BufWriter::new(open_spool(&self.spool).map_err(|e| format!("The rows could not be kept: {e}"))?);
        let mut url = self.url.clone();
        for page_no in 1..=MAX_PAGES {
            let left = Duration::from_secs(TOTAL_SECS).checked_sub(started.elapsed()).filter(|d| !d.is_zero()).ok_or("The run passed its limit of 10 minutes")?;
            let (rows, next) = self.page(&url, left).map_err(|e| self.scrub(e))?;
            for chunk in rows.chunks(BATCH_ROWS) {
                for row in chunk {
                    writeln!(out, "{row}").map_err(|e| format!("The rows could not be kept: {e}"))?;
                }
                f(chunk.to_vec())?;
            }
            match next {
                None => return out.flush().map_err(|e| format!("The rows could not be kept: {e}")),
                Some(_) if page_no == MAX_PAGES => return Err(format!("The source has more than {MAX_PAGES} pages. A run reads at most {MAX_PAGES} pages")),
                Some(n) => url = n,
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
fn open_spool(path: &PathBuf) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)
}

#[cfg(not(unix))]
fn open_spool(path: &PathBuf) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().write(true).create_new(true).open(path)
}

impl Drop for HttpFeed {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.spool);
    }
}

impl Feed for HttpFeed {
    fn columns(&mut self) -> Result<Option<Vec<String>>, String> {
        // A web service does not list its columns. They are the keys its rows have.
        Ok(None)
    }

    fn each(&mut self, pass: u8, f: &mut dyn FnMut(Vec<Value>) -> Result<(), String>) -> Result<(), String> {
        if pass == 1 {
            return self.fetch_all(f);
        }
        let file = std::fs::File::open(&self.spool).map_err(|e| format!("The kept rows could not be read: {e}"))?;
        let mut batch = Vec::with_capacity(BATCH_ROWS);
        for line in BufReader::new(file).lines() {
            let line = line.map_err(|e| format!("The kept rows could not be read: {e}"))?;
            batch.push(serde_json::from_str(&line).map_err(|e| format!("A kept row could not be read: {e}"))?);
            if batch.len() >= BATCH_ROWS {
                f(std::mem::take(&mut batch))?;
            }
        }
        if !batch.is_empty() {
            f(batch)?;
        }
        Ok(())
    }
}

/// One run of an http connection.
pub(crate) fn apply(state: &SharedState, conn: &Conn, opts: &RunOpts) -> Result<(Option<Counts>, String), Fault> {
    check_config(&conn.config)?;
    let token = match conn.secret.as_deref() {
        Some(name) => {
            let held = crate::secrets::get(state, name).map_err(|e| format!("The secret '{name}' cannot be read: {e}"))?;
            Some(String::from_utf8_lossy(held.expose()).trim().to_string())
        }
        None => None,
    };
    let mut feed = HttpFeed {
        client: Client::for_state(state),
        url: text(&conn.config, "url").unwrap_or_default().to_string(),
        json: text(&conn.config, "format") != Some("jsonl"),
        path: text(&conn.config, "path").map(str::to_string),
        next: text(&conn.config, "next").map(str::to_string),
        token,
        spool: std::env::temp_dir().join(format!("scaffoldry-sync-{}.jsonl", uuid::Uuid::new_v4())),
    };
    drive(state, conn, opts, &mut feed)
}
