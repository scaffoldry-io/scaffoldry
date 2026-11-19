//! The NIST controls the platform knows, with their titles.
//!
//! Two catalogs (SP 800-53 Rev 5 and SP 800-171 Rev 3) and the three SP 800-53 baselines are
//! embedded from `governance/catalog/`. They are slimmed by `governance/scripts/slim-catalog.py`.
//! The files are parsed once, on first use.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

const SP800_53: &str = include_str!("../../../governance/catalog/sp800-53r5.slim.json");
const SP800_171: &str = include_str!("../../../governance/catalog/sp800-171r3.slim.json");
const BASELINES: &str = include_str!("../../../governance/catalog/baselines.json");

#[derive(Debug, Clone, Deserialize)]
pub struct Control {
    /// `sp800-53r5` or `sp800-171r3`. Set when the file is loaded.
    #[serde(skip)]
    pub catalog: &'static str,
    pub id: String,
    pub title: String,
    pub family_id: String,
    pub family_title: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub statement: String,
}

#[derive(Deserialize)]
struct CatalogFile {
    controls: Vec<Control>,
}

#[derive(Deserialize)]
struct BaselineFile {
    baselines: HashMap<String, Vec<String>>,
}

struct Loaded {
    controls: Vec<Control>,
    by_id: HashMap<String, usize>,
    baselines: HashMap<String, Vec<String>>,
}

fn loaded() -> &'static Loaded {
    static LOADED: OnceLock<Loaded> = OnceLock::new();
    LOADED.get_or_init(|| {
        let mut controls = Vec::new();
        for (name, text) in [("sp800-53r5", SP800_53), ("sp800-171r3", SP800_171)] {
            let file: CatalogFile = serde_json::from_str(text).expect("embedded catalog is valid");
            controls.extend(file.controls.into_iter().map(|c| Control { catalog: name, ..c }));
        }
        let by_id = controls.iter().enumerate().map(|(i, c)| (c.id.to_ascii_lowercase(), i)).collect();
        let file: BaselineFile = serde_json::from_str(BASELINES).expect("embedded baselines are valid");
        Loaded { controls, by_id, baselines: file.baselines }
    })
}

/// Bring a control id to OSCAL form: `AC-02` and `AC-2(1)` become `ac-2` and `ac-2.1`.
/// An id that is not shaped like an SP 800-53 id is only trimmed and lowercased.
pub fn normalize(raw: &str) -> String {
    let s = raw.trim().to_ascii_lowercase();
    let b = s.as_bytes();
    if b.len() < 4 || !b[0].is_ascii_lowercase() || !b[1].is_ascii_lowercase() || b[2] != b'-' {
        return s;
    }
    let mut out = s[..3].to_string();
    let mut rest = &s[3..];
    let mut first = true;
    while !rest.is_empty() {
        let (open, body) = match rest.as_bytes()[0] {
            b'.' if !first => (b'.', &rest[1..]),
            b'(' if !first => (b'(', &rest[1..]),
            b'0'..=b'9' if first => (b' ', rest),
            _ => return s,
        };
        let end = body.find(|c: char| !c.is_ascii_digit()).unwrap_or(body.len());
        let Ok(n) = body[..end].parse::<u32>() else { return s };
        rest = &body[end..];
        if open == b'(' {
            let Some(after) = rest.strip_prefix(')') else { return s };
            rest = after;
        }
        if !first {
            out.push('.');
        }
        out.push_str(&n.to_string());
        first = false;
    }
    out
}

/// One control by id. The id is normalized first.
pub fn control(id: &str) -> Option<&'static Control> {
    let l = loaded();
    l.by_id.get(&normalize(id)).map(|&i| &l.controls[i])
}

pub fn title(id: &str) -> Option<&'static str> {
    control(id).map(|c| c.title.as_str())
}

/// Controls whose id, title, or statement contains `query` (case-insensitive), best match first.
/// An empty query matches every control. `family` is a family id or title. `catalog` is a catalog name.
pub fn search(query: &str, family: Option<&str>, catalog: Option<&str>) -> Vec<&'static Control> {
    let q = query.trim().to_ascii_lowercase();
    let mut hits: Vec<(u8, &Control)> = Vec::new();
    for c in &loaded().controls {
        if catalog.is_some_and(|n| n != c.catalog) {
            continue;
        }
        if family.is_some_and(|f| !f.eq_ignore_ascii_case(&c.family_id) && !f.eq_ignore_ascii_case(&c.family_title)) {
            continue;
        }
        let rank = if q.is_empty() || c.id.eq_ignore_ascii_case(&q) || c.id.eq_ignore_ascii_case(&normalize(&q)) {
            0
        } else if c.title.to_ascii_lowercase().starts_with(&q) {
            1
        } else if c.title.to_ascii_lowercase().contains(&q) {
            2
        } else if c.statement.to_ascii_lowercase().contains(&q) {
            3
        } else {
            continue;
        };
        hits.push((rank, c));
    }
    hits.sort_by_key(|(rank, _)| *rank);
    hits.into_iter().map(|(_, c)| c).collect()
}

/// The control ids a baseline selects: `low`, `moderate`, or `high`.
pub fn baseline(name: &str) -> Option<&'static [String]> {
    loaded().baselines.get(&name.trim().to_ascii_lowercase()).map(Vec::as_slice)
}

const PROVENANCE: &str = include_str!("../../../governance/catalog/PROVENANCE.json");

/// The canonical source URL of a catalog, from `governance/catalog/PROVENANCE.json`.
pub fn source_url(catalog: &str) -> Option<&'static str> {
    static URLS: OnceLock<HashMap<&'static str, String>> = OnceLock::new();
    let urls = URLS.get_or_init(|| {
        let doc: serde_json::Value = serde_json::from_str(PROVENANCE).expect("embedded provenance is valid");
        let url_of = |file: &str| doc["files"].as_array().into_iter().flatten().find(|f| f["name"] == file).and_then(|f| f["source_url"].as_str()).map(str::to_string);
        [("sp800-53r5", "NIST_SP-800-53_rev5_catalog.json"), ("sp800-171r3", "NIST_SP800-171_rev3_catalog.json")]
            .into_iter()
            .filter_map(|(name, file)| url_of(file).map(|u| (name, u)))
            .collect()
    });
    urls.get(catalog).map(String::as_str)
}
