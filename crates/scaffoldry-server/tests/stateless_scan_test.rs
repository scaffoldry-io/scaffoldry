//! Stateless server phase 1: the source is read as text. Nothing is held between requests that is not on a list.

use std::path::Path;

/// The fields of `ServerState` that may hold a lock. The list starts as today's fields. Each phase removes the ones
/// it turns into queries or into cached collections, and none is added.
const ALLOWED_FIELDS: &[&str] = &[
    "app_views",
    "workspaces",
    "collaborators",
    "guards",
    "guard_policy_cache",
    "datasets",
    "relationships",
    "automations",
    "process_instances",
    "ledger",
    "engine",
    "organizations",
    "position_types",
    "labels",
    "settings",
    "formula_rate_limits",
];

/// The `static` locks that may remain. None does after phase 2.
const ALLOWED_STATICS: &[&str] = &[];

fn source(rel: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn server_state_holds_no_lock_that_is_not_on_the_list() {
    let text = source("src/state.rs");
    let start = text.find("pub struct ServerState {").expect("the struct");
    let body = &text[start..text[start..].find("\n}\n").map(|i| start + i).expect("the end of the struct")];
    let mut found = Vec::new();
    for line in body.lines().skip(1) {
        let line = line.trim();
        if line.starts_with("//") || !(line.contains("RwLock<") || line.contains("Mutex<")) {
            continue;
        }
        let name = line.trim_start_matches("pub ").split(':').next().unwrap().trim().to_string();
        found.push(name);
    }
    let unlisted: Vec<&String> = found.iter().filter(|f| !ALLOWED_FIELDS.contains(&f.as_str())).collect();
    assert!(unlisted.is_empty(), "these fields hold state between requests and are not on the list: {unlisted:?}");
}

#[test]
fn no_static_holds_a_lock_outside_the_list() {
    let mut files = Vec::new();
    rust_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    let mut offenders = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        for (n, line) in text.lines().enumerate() {
            let l = line.trim_start();
            let is_static = l.starts_with("static ") || l.starts_with("pub static ") || l.starts_with("pub(crate) static ");
            if is_static && ["Mutex<", "RwLock<", "OnceLock<", "LazyLock<"].iter().any(|k| l.contains(k)) {
                let name = l.split("static ").nth(1).unwrap().split(':').next().unwrap().trim().to_string();
                if !ALLOWED_STATICS.contains(&name.as_str()) {
                    offenders.push(format!("{}:{} {name}", f.display(), n + 1));
                }
            }
        }
    }
    assert!(offenders.is_empty(), "statics that hold a lock: {offenders:?}");
}

/// The collections phase 3 moved onto the mechanism. Only `ServerState::reload` may replace them. Every other
/// write reaches PostgreSQL first, bumps the counter, and lets `refresh` read it back.
const RELOADED: &[&str] = &["organizations", "position_types", "workspaces", "collaborators", "guards", "datasets", "relationships", "automations"];

fn without_space(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

#[test]
fn only_reload_writes_the_collections_the_mechanism_owns() {
    let mut files = Vec::new();
    rust_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    let mut offenders = Vec::new();
    for f in files {
        let flat = without_space(&std::fs::read_to_string(&f).unwrap());
        let is_state = f.file_name().is_some_and(|n| n == "state.rs");
        // In state.rs a write is allowed between the start of `reload` and the start of `new`.
        let allowed = if is_state { flat.find("fnreload(").zip(flat.find("pubfnnew()")) } else { None };
        for name in RELOADED {
            let needle = format!("{name}.write()");
            let mut from = 0;
            while let Some(i) = flat[from..].find(&needle) {
                let at = from + i;
                // `.write()` on a field only: `self.workspaces.write()` and `state.workspaces.write()`, not `x_workspaces`.
                let field = at == 0 || matches!(flat.as_bytes()[at - 1], b'.');
                if field && !allowed.is_some_and(|(start, end)| at > start && at < end) {
                    offenders.push(format!("{}: {needle}", f.display()));
                }
                from = at + needle.len();
            }
        }
    }
    assert!(offenders.is_empty(), "these write a cached collection outside reload: {offenders:?}");
}

/// Only these files put an app definition into the engine. The load from storage is in `state.rs`. A saved view
/// re-merges itself into the copy this instance holds, in `service/views.rs`. An approval, a route, or a tool does not
/// register a definition: it commits to PostgreSQL, bumps `manifests`, and lets `refresh` load the app whose version moved.
const MAY_REGISTER: &[&str] = &["state.rs", "views.rs"];

#[test]
fn only_the_load_and_a_saved_view_put_an_app_definition_in_the_engine() {
    let mut files = Vec::new();
    rust_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    let mut offenders = Vec::new();
    for f in files {
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        let flat = without_space(&std::fs::read_to_string(&f).unwrap());
        if !MAY_REGISTER.contains(&name.as_str()) && (flat.contains(".register_manifest(") || flat.contains(".register_runtime_manifest(")) {
            offenders.push(f.display().to_string());
        }
    }
    assert!(offenders.is_empty(), "these register an app definition themselves: {offenders:?}");
}
