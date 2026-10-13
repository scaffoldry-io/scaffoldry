//! Attachments phase 1: file bytes on disk, named by their SHA-256.
//!
//! A file sits at `$SCAFFOLDRY_FILES_DIR/aa/bb/{sha256}`. It is written to a temporary name first and renamed once
//! its hash is known, so a reader never sees half a file. The type of a file comes from its first bytes. What the
//! client says about the type is never read.

use crate::repository::PostgresRepository;
use crate::state::SharedState;
use axum::body::{Body, HttpBody};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

pub const DEFAULT_MAX_MB: u64 = 50;
pub const MAX_MAX_MB: u64 = 500;
pub const DEFAULT_QUOTA_MB: u64 = 5000;
/// How long an upload may wait for a record to take it.
pub const UNATTACHED_HOURS: i64 = 24;

const SNIFF_BYTES: usize = 512;
const TMP: &str = ".tmp";

/// Why a store stopped.
#[derive(Debug)]
pub enum StoreError {
    TooLarge,
    Blocked,
    Bad(String),
    Io(String),
}

/// A file that was written.
#[derive(Debug)]
pub struct Stored {
    pub sha256: String,
    pub size: u64,
    pub media_type: &'static str,
}

/// The directory named by `SCAFFOLDRY_FILES_DIR`. `None` when it is not set, which turns uploads off.
pub fn dir() -> Option<PathBuf> {
    std::env::var_os("SCAFFOLDRY_FILES_DIR").filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Checks the files directory at boot. Unset is allowed. A directory that is missing or that the server cannot write
/// to is an error, so a bad mount stops the server instead of failing the first upload.
pub fn open_dir() -> Result<Option<PathBuf>, String> {
    let Some(dir) = dir() else { return Ok(None) };
    if !dir.is_dir() {
        return Err(format!("SCAFFOLDRY_FILES_DIR '{}' is not a directory that exists", dir.display()));
    }
    let probe = dir.join(format!(".probe-{}", uuid::Uuid::new_v4()));
    std::fs::write(&probe, b"x").map_err(|e| format!("SCAFFOLDRY_FILES_DIR '{}' is not writable: {e}", dir.display()))?;
    let _ = std::fs::remove_file(&probe);
    Ok(Some(dir))
}

/// Where the blob with this hash lives.
pub fn blob_path(dir: &Path, sha256: &str) -> PathBuf {
    dir.join(&sha256[0..2]).join(&sha256[2..4]).join(sha256)
}

fn setting_mb(state: &SharedState, key: &str, default: u64) -> u64 {
    state.settings.read().ok().and_then(|s| s.get(key).and_then(serde_json::Value::as_u64)).unwrap_or(default)
}

/// The most bytes one file may hold.
pub fn max_bytes(state: &SharedState) -> u64 {
    setting_mb(state, "files.max_mb", DEFAULT_MAX_MB).clamp(1, MAX_MAX_MB) * 1024 * 1024
}

/// The most bytes the files of one app may hold in all.
pub fn quota_bytes(state: &SharedState) -> u64 {
    setting_mb(state, "files.app_quota_mb", DEFAULT_QUOTA_MB).max(1) * 1024 * 1024
}

/// A header value with its `%XX` escapes undone. A browser sends the name this way because a header holds Latin-1 only.
pub fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%' && i + 2 < bytes.len() && hex(bytes[i + 1]).is_some() && hex(bytes[i + 2]).is_some() {
            out.push((hex(bytes[i + 1]).unwrap() * 16 + hex(bytes[i + 2]).unwrap()) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/// A file name with its path, control characters, and odd characters taken out.
pub fn clean_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let mut out: String = base
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ' | '(' | ')') { c } else { '_' })
        .collect();
    while out.contains("..") {
        out = out.replace("..", ".");
    }
    let out = out.trim_matches(|c: char| c == '.' || c == ' ').chars().take(120).collect::<String>();
    if out.is_empty() { "file".to_string() } else { out }
}

const BLOCKED_EXTENSIONS: &[&str] = &[
    "exe", "dll", "com", "bat", "cmd", "msi", "scr", "ps1", "psm1", "vbs", "vbe", "wsf", "jar", "apk", "app", "sh", "bash", "js", "mjs", "cjs", "html", "htm",
    "xhtml", "svg", "svgz", "php", "hta", "lnk", "reg", "pif", "cpl", "swf",
];

/// True when the name's last extension is one that runs or renders in a browser or a shell.
pub fn blocked_name(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| BLOCKED_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

fn starts_with_ci(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len() && text.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// The type of a file from its first bytes, or `None` when the bytes are an executable, a script, or a page.
pub fn sniff(head: &[u8]) -> Option<&'static str> {
    const NATIVE: &[&[u8]] = &[
        b"\x7fELF", b"#!", b"\xfe\xed\xfa\xce", b"\xfe\xed\xfa\xcf", b"\xce\xfa\xed\xfe", b"\xcf\xfa\xed\xfe", b"\xca\xfe\xba\xbe", b"FWS", b"CWS",
        b"ZWS",
    ];
    if NATIVE.iter().any(|sig| head.starts_with(sig)) {
        return None;
    }
    // A Windows program starts with "MZ" and then binary. Text that happens to start with those letters is text.
    if head.starts_with(b"MZ") && head[2..head.len().min(16)].iter().any(|b| *b < 9 || *b >= 0x80) {
        return None;
    }
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if head.starts_with(b"\xff\xd8\xff") {
        return Some("image/jpeg");
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if head.starts_with(b"%PDF-") {
        return Some("application/pdf");
    }
    if head.starts_with(b"PK\x03\x04") {
        let has = |needle: &[u8]| head.windows(needle.len()).any(|w| w == needle);
        return Some(if has(b"word/") {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        } else if has(b"xl/") {
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        } else if has(b"ppt/") {
            "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        } else if has(b"META-INF/") {
            // A jar is a zip that runs.
            return None;
        } else {
            "application/zip"
        });
    }
    if head.contains(&0) {
        return Some("application/octet-stream");
    }
    // Text: valid UTF-8, allowing a character cut off at the end of the sample.
    let text = match std::str::from_utf8(head) {
        Ok(t) => t,
        Err(e) if e.error_len().is_none() => std::str::from_utf8(&head[..e.valid_up_to()]).unwrap_or(""),
        Err(_) => return Some("application/octet-stream"),
    };
    let trimmed = text.trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n']);
    const MARKUP: &[&str] = &["<!doctype", "<html", "<head", "<body", "<script", "<svg", "<?xml", "<?php", "<iframe", "<meta", "<style", "<link", "<a "];
    if MARKUP.iter().any(|m| starts_with_ci(trimmed, m)) {
        return None;
    }
    let first_line = text.lines().next().unwrap_or("");
    Some(if first_line.contains(',') && text.contains('\n') { "text/csv" } else { "text/plain" })
}

/// True for the types a browser may show inline. Everything else is a download.
pub fn inline_type(media_type: &str) -> bool {
    matches!(media_type, "image/png" | "image/jpeg" | "image/gif" | "image/webp")
}

/// Reads the body to the end, writing it to a temporary file and hashing as it goes. It stops at `limit` bytes. When
/// the hash is known the file moves to its place, unless the same bytes are already there.
pub async fn store(dir: &Path, mut body: Body, declared: u64, limit: u64) -> Result<Stored, StoreError> {
    if declared > limit {
        return Err(StoreError::TooLarge);
    }
    let tmp_dir = dir.join(TMP);
    tokio::fs::create_dir_all(&tmp_dir).await.map_err(|e| StoreError::Io(e.to_string()))?;
    let tmp = tmp_dir.join(uuid::Uuid::new_v4().to_string());
    let outcome = write_body(&tmp, &mut body, declared, limit).await;
    let (sha256, size, media_type) = match outcome {
        Ok(v) => v,
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(e);
        }
    };
    let dest = blob_path(dir, &sha256);
    if tokio::fs::try_exists(&dest).await.unwrap_or(false) {
        let _ = tokio::fs::remove_file(&tmp).await;
    } else {
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| StoreError::Io(e.to_string()))?;
        }
        if let Err(e) = tokio::fs::rename(&tmp, &dest).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(StoreError::Io(e.to_string()));
        }
    }
    Ok(Stored { sha256, size, media_type })
}

async fn write_body(tmp: &Path, body: &mut Body, declared: u64, limit: u64) -> Result<(String, u64, &'static str), StoreError> {
    let mut file = tokio::fs::File::create(tmp).await.map_err(|e| StoreError::Io(e.to_string()))?;
    let mut hasher = Sha256::new();
    let mut size: u64 = 0;
    let mut head: Vec<u8> = Vec::with_capacity(SNIFF_BYTES);
    let mut media_type: Option<&'static str> = None;
    loop {
        let frame = std::future::poll_fn(|cx| std::pin::Pin::new(&mut *body).poll_frame(cx)).await;
        let Some(frame) = frame else { break };
        let frame = frame.map_err(|e| StoreError::Bad(format!("the upload was cut off: {e}")))?;
        let Ok(data) = frame.into_data() else { continue };
        size += data.len() as u64;
        if size > limit {
            return Err(StoreError::TooLarge);
        }
        if size > declared {
            return Err(StoreError::Bad("the body is longer than its Content-Length".to_string()));
        }
        if media_type.is_none() {
            let room = SNIFF_BYTES - head.len();
            head.extend_from_slice(&data[..data.len().min(room)]);
            if head.len() >= SNIFF_BYTES {
                media_type = Some(sniff(&head).ok_or(StoreError::Blocked)?);
            }
        }
        hasher.update(&data);
        file.write_all(&data).await.map_err(|e| StoreError::Io(e.to_string()))?;
    }
    if size != declared {
        return Err(StoreError::Bad("the body is shorter than its Content-Length".to_string()));
    }
    let media_type = match media_type {
        Some(t) => t,
        None => sniff(&head).ok_or(StoreError::Blocked)?,
    };
    file.flush().await.map_err(|e| StoreError::Io(e.to_string()))?;
    let digest = hasher.finalize();
    let sha256: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    Ok((sha256, size, media_type))
}

/// Removes uploads no record took within `hours`, and the blobs nothing refers to any more. Returns how many uploads
/// went.
pub fn purge_unattached(repo: &PostgresRepository, hours: i64) -> Result<usize, String> {
    let (removed, orphans) = repo.purge_unattached_refs(hours).map_err(|e| e.to_string())?;
    if let Some(dir) = dir() {
        for sha in orphans {
            let _ = std::fs::remove_file(blob_path(&dir, &sha));
        }
    }
    Ok(removed)
}
