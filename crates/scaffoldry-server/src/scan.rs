//! Attachments phase 2: scanning a stored file through a daemon on a Unix socket.
//!
//! The daemon speaks ClamAV's streaming protocol. The client sends `zINSTREAM\0`, then the file as chunks, each a
//! four-byte big-endian length and its bytes, then a zero length. The daemon answers with one line, such as
//! `stream: OK` or `stream: Eicar-Test-Signature FOUND`. A daemon that does not answer in 30 seconds is an `error`.

use crate::repository::PostgresRepository;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

const CHUNK: usize = 64 * 1024;
const PING_SECS: u64 = 3;

/// How long the daemon has to answer. `SCAFFOLDRY_SCAN_TIMEOUT_MS` shortens it for tests.
pub fn timeout() -> Duration {
    let ms = std::env::var("SCAFFOLDRY_SCAN_TIMEOUT_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(30_000);
    Duration::from_millis(ms)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Clean,
    /// The signature the daemon named.
    Infected(String),
}

#[derive(Debug, Clone)]
pub struct Scanner {
    pub socket: PathBuf,
}

impl Scanner {
    /// The scanner a `files.scanner` setting names. `None` for no setting, or one that names nothing.
    pub fn from_setting(value: Option<&Value>) -> Option<Scanner> {
        let v = value?;
        if v.get("kind").and_then(Value::as_str) != Some("clamd") {
            return None;
        }
        v.get("socket").and_then(Value::as_str).filter(|s| !s.is_empty()).map(|s| Scanner { socket: PathBuf::from(s) })
    }

    fn connect(&self, wait: Duration) -> Result<UnixStream, String> {
        let stream = UnixStream::connect(&self.socket).map_err(|e| format!("the scanner is not reachable: {e}"))?;
        stream.set_read_timeout(Some(wait)).map_err(|e| e.to_string())?;
        stream.set_write_timeout(Some(wait)).map_err(|e| e.to_string())?;
        Ok(stream)
    }

    /// True when the daemon answers a ping.
    pub fn ping(&self) -> bool {
        let Ok(mut s) = self.connect(Duration::from_secs(PING_SECS).min(timeout())) else { return false };
        s.write_all(b"zPING\0").is_ok() && read_answer(&mut s).is_ok_and(|a| a == "PONG")
    }

    /// Sends the file and reads the verdict.
    pub fn scan(&self, path: &Path) -> Result<Verdict, String> {
        let mut file = std::fs::File::open(path).map_err(|e| format!("the file could not be read: {e}"))?;
        let mut s = self.connect(timeout())?;
        s.write_all(b"zINSTREAM\0").map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = file.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            s.write_all(&(n as u32).to_be_bytes()).map_err(|e| e.to_string())?;
            s.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        }
        s.write_all(&0u32.to_be_bytes()).map_err(|e| e.to_string())?;
        let answer = read_answer(&mut s)?;
        if let Some(found) = answer.strip_suffix(" FOUND") {
            Ok(Verdict::Infected(found.trim_start_matches("stream:").trim().to_string()))
        } else if answer.ends_with("OK") {
            Ok(Verdict::Clean)
        } else {
            Err(format!("the scanner said: {answer}"))
        }
    }
}

/// One answer: bytes up to a NUL or a newline.
fn read_answer(s: &mut UnixStream) -> Result<String, String> {
    let mut out = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match s.read(&mut byte) {
            Ok(0) => break,
            Ok(_) if byte[0] == 0 || byte[0] == b'\n' => break,
            Ok(_) => out.push(byte[0]),
            Err(e) => return Err(format!("the scanner did not answer: {e}")),
        }
        if out.len() > 4096 {
            return Err("the scanner's answer was too long".to_string());
        }
    }
    Ok(String::from_utf8_lossy(&out).trim().to_string())
}

/// Scans one upload and records the result. Returns the new `scan_state`. A file that is not `pending` is left as it is.
pub fn scan_file(repo: &PostgresRepository, ref_id: Uuid) -> Result<String, String> {
    let Some(file) = repo.file_ref_any(ref_id).map_err(|e| e.to_string())? else { return Ok("gone".to_string()) };
    if file.scan_state != "pending" {
        return Ok(file.scan_state);
    }
    let settings = repo.get_platform_settings().map_err(|e| e.to_string())?;
    let Some(scanner) = Scanner::from_setting(settings.get("files.scanner")) else {
        repo.set_scan_state(ref_id, "skipped").map_err(|e| e.to_string())?;
        return Ok("skipped".to_string());
    };
    let Some(dir) = crate::files::dir() else {
        repo.set_scan_state(ref_id, "error").map_err(|e| e.to_string())?;
        return Ok("error".to_string());
    };
    let path = crate::files::blob_path(&dir, &file.sha256);
    // The daemon is slow and the call blocks, so it runs apart from the database client.
    let verdict = scanner.scan(&path);
    match verdict {
        Ok(Verdict::Clean) => {
            repo.set_scan_state(ref_id, "clean").map_err(|e| e.to_string())?;
            Ok("clean".to_string())
        }
        Ok(Verdict::Infected(signature)) => {
            // The same bytes are infected for every upload of them. Each is let go of, and the bytes are removed.
            repo.mark_sha_infected(&file.sha256).map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(&path);
            repo.append_ledger_decision(crate::repository::AppendDecisionParams {
                principal: "scanner@scaffoldry.internal".to_string(),
                organization_code: "INST".to_string(),
                app_slug: Some(file.app_slug.clone()),
                decision_type: scaffoldry_core::ledger::DecisionType::FileInfected,
                oscal_control_id: "SI-03".to_string(),
                rationale: format!("The scanner found {signature} in '{}' (sha256 {}), uploaded by {}. The file was removed.", file.name, file.sha256, file.uploaded_by),
                payload: json!({ "sha256": file.sha256, "signature": signature, "ref_id": ref_id, "uploaded_by": file.uploaded_by }),
            })
            .map_err(|e| e.to_string())?;
            Ok("infected".to_string())
        }
        Err(_) => {
            repo.set_scan_state(ref_id, "error").map_err(|e| e.to_string())?;
            Ok("error".to_string())
        }
    }
}

/// Scans the uploads still `pending`, oldest first, until none are left or `max` have been done. Returns how many.
pub fn scan_pending(repo: &PostgresRepository, max: usize, mut stop: impl FnMut() -> bool) -> Result<usize, String> {
    let mut done = 0;
    while done < max && !stop() {
        let ids = repo.pending_scan_ids(50).map_err(|e| e.to_string())?;
        if ids.is_empty() {
            break;
        }
        for id in ids {
            scan_file(repo, id)?;
            done += 1;
        }
    }
    Ok(done)
}

/// What the Overview shows: whether a scanner is set, whether it answers, the last result, and the files that need a look.
pub fn overview(repo: Option<&PostgresRepository>, setting: Option<&Value>) -> Value {
    let scanner = Scanner::from_setting(setting);
    let (counts, last) = repo
        .map(|r| (r.scan_counts().unwrap_or_default(), r.last_scan_result().unwrap_or(None)))
        .unwrap_or_default();
    let count = |k: &str| counts.get(k).copied().unwrap_or(0);
    json!({
        "scanner": {
            "configured": scanner.is_some(),
            "reachable": scanner.as_ref().is_some_and(Scanner::ping),
            "last_result": last,
        },
        "error": count("error"),
        "skipped": count("skipped"),
        "pending": count("pending"),
        "infected": count("infected"),
    })
}
