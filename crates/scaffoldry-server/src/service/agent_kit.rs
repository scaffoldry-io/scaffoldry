//! The agent kit (agent coverage phase 4): what an AI tool needs to work with this server from the first message.
//!
//! The kit is text for the server's current version. It writes nothing. The AI tool shows the person which files it
//! would create or change, asks, and then writes them. The kit holds no token, address or record value.

use crate::service::prompts::{GUIDE, OUTSIDE_GUIDE};
use serde_json::{json, Value};

pub const BEGIN: &str = "<!-- scaffoldry:begin -->";
pub const END: &str = "<!-- scaffoldry:end -->";

/// The date the guides below were last changed. Change it with them: an agent compares it with its copy to see that the
/// copy is stale.
pub const GUIDES_DATE: &str = "2026-10-24";

/// Where a skill folder goes, for Claude Code and for Codex.
pub const SKILL_PATHS: [&str; 2] = [".claude/skills/scaffoldry/", ".agents/skills/scaffoldry/"];

/// The guides the skill carries, as `(file name, text)`.
const GUIDES: &[(&str, &str)] = &[("building-apps.md", GUIDE), ("outside-apps.md", OUTSIDE_GUIDE)];

fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The line that says which server and which guides a file came from.
fn header() -> String {
    format!("<!-- scaffoldry-version: {}; guides-date: {GUIDES_DATE} -->", version())
}

fn agents_md_section() -> String {
    format!(
        "{BEGIN}\n{header}\n## Scaffoldry\n\
Scaffoldry is the institution's governed place for apps and records. You reach it over MCP, as the signed-in person. You never get more than they have.\n\n\
Start here:\n\
- Read the guide: the MCP resource `scaffoldry://guide/building-apps`.\n\
- Call `list_workspaces`, then `list_apps`, then `describe_app` before you change anything.\n\
- If a request fails, read the error's `hint`. Do not look for another way to the same data.\n\n\
Rules you keep:\n\
- Propose, never assume approval. `propose_app_change` changes nothing until a person approves it. Say the app is live only when its status is Approved.\n\
- Never copy records into files, chat or notes. Report counts, not values.\n\
- Fields marked sensitive stay in the platform. Do not paste, summarize or move them anywhere else.\n\
- Do not store a token in a file. Use the one your MCP client holds.\n\
{END}\n",
        header = header()
    )
}

fn skill_md() -> String {
    format!(
        "---\nname: scaffoldry\ndescription: Build and use governed apps in Scaffoldry over MCP: propose changes, wait for approval, and work with records safely.\n---\n{}\n\n\
# Scaffoldry\n\n\
Use this skill when the person asks for an app, a form, a table of records, or an approval in Scaffoldry.\n\n\
1. Read `references/building-apps.md` once.\n\
2. Call `list_workspaces`, `list_apps` and `describe_app`.\n\
3. Propose changes with `propose_app_change`. Never say a change is live before it is approved.\n\
4. Never copy records into files. Report counts, not values.\n\
5. For a page that runs outside Scaffoldry, read `references/outside-apps.md` instead of reaching for a token.\n",
        header()
    )
}

/// A guide with the header above its text.
fn reference(text: &str) -> String {
    format!("{}\n\n{text}", header())
}

pub fn kit() -> Value {
    let mut files = vec![json!({ "path": "SKILL.md", "content": skill_md() })];
    for (name, text) in GUIDES {
        files.push(json!({ "path": format!("references/{name}"), "content": reference(text) }));
    }
    // What the agent shows the person before it writes: every path, and what happens to it.
    let mut plan = vec![json!({ "path": "AGENTS.md", "action": "create, or change only the text between the markers" })];
    for dir in SKILL_PATHS {
        for f in &files {
            plan.push(json!({ "path": format!("{dir}{}", f["path"].as_str().unwrap_or_default()), "action": "create, or replace" }));
        }
    }
    json!({
        "version": version(),
        "guides_date": GUIDES_DATE,
        "agents_md": agents_md_section(),
        "markers": { "begin": BEGIN, "end": END },
        "skill": { "paths": SKILL_PATHS, "files": files },
        "files": plan,
        "instructions": "Show the person the files above and ask before you write. Change an existing AGENTS.md only between the markers and keep every other line exactly. If it has no markers, add the section at the end."
    })
}
