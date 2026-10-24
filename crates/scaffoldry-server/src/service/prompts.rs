//! Prompts: short templates that tell an agent which tools to call, in order. They run no model and change nothing.

use serde_json::{json, Value};

pub const GUIDE_URI: &str = "scaffoldry://guide/building-apps";
pub const GUIDE: &str = include_str!("../../guides/building-apps.md");
pub const OUTSIDE_GUIDE_URI: &str = "scaffoldry://guide/outside-apps";
pub const OUTSIDE_GUIDE: &str = include_str!("../../guides/outside-apps.md");

struct Prompt {
    name: &'static str,
    description: &'static str,
    /// `(name, description, required)`.
    arguments: &'static [(&'static str, &'static str, bool)],
    /// The template. `{name}` is replaced by the argument of that name.
    template: &'static str,
}

const PROMPTS: &[Prompt] = &[
    Prompt {
        name: "build_app_from_description",
        description: "Turn a plain description into a proposed app, then wait for approval.",
        arguments: &[("description", "What the person wants to keep track of, in their words", true)],
        template: "Build an app for this request: {description}\n\
Read the resource scaffoldry://guide/building-apps first. Then, in order:\n\
1. Call list_workspaces and ask the person which workspace if more than one fits.\n\
2. Call list_apps for it, so you do not duplicate an app that exists. If one fits, call describe_app and change it instead.\n\
3. Draft a manifest. Use a Select for a fixed set of choices and a Link for a reference to another table. Mark a field ferpa_sensitive when it identifies a student.\n\
4. Call propose_app_change with the whole manifest and a one-sentence summary. Show the person the checks it returns.\n\
5. Stop. Only when the person says to, or an admin has approved, call decide_proposal with the digest the proposal showed. Never say the app is live before the status is Approved.\n\
6. Call create_record to add the first row.",
    },
    Prompt {
        name: "import_spreadsheet",
        description: "Bring rows from a spreadsheet into an app, a batch at a time.",
        arguments: &[("app_slug", "The app to import into", true), ("columns", "The spreadsheet's column names", true)],
        template: "Import a spreadsheet with the columns {columns} into the app {app_slug}.\n\
1. Call describe_app to read its tables and fields. Match each column to a field. Ask the person about any column that matches none.\n\
2. If a field is missing, call propose_app_change to add it, and wait for approval before you go on.\n\
3. Send a few rows first with create_record, and call list_records to check how they look. A Select takes an option id, and a Link takes a record id.\n\
4. Then send the rest in small batches with create_record. If a row is refused, read the hint, fix that row, and go on.\n\
5. Never copy the spreadsheet's values anywhere else. Report counts, not values.",
    },
    Prompt {
        name: "design_approval_process",
        description: "Add an approval step to an app, with the people who decide.",
        arguments: &[("app_slug", "The app that needs the approval", true), ("description", "Who approves what, in the person's words", true)],
        template: "Design an approval process for the app {app_slug}: {description}\n\
1. Call describe_app to read the app, its fields, and any rules it already has.\n\
2. Call propose_app_change with the app_slug and a rule that waits on the approver. Name who decides by position or by person. Do not use a webhook step.\n\
3. Show the person the checks and who will decide. A change that affects who can see data needs a second person to approve it.\n\
4. When it is live, call list_waiting_decisions to see the steps waiting, and call decide_process only for the person who decides.",
    },
    Prompt {
        name: "set_up_this_project",
        description: "Set this project up to work with Scaffoldry: an AGENTS.md section and a skill, after the person agrees.",
        arguments: &[],
        template: "Set this project up to use Scaffoldry.\n\
1. Call get_agent_kit.\n\
2. Show the person every file in its `files` list and what will happen to it. Then ask whether to go ahead. Write nothing before they say yes.\n\
3. For AGENTS.md: if the file has the markers <!-- scaffoldry:begin --> and <!-- scaffoldry:end -->, change only the text between the markers and keep every other line exactly. If it has no markers, add the section at the end. If there is no file, create it.\n\
4. Write the skill folder to the paths the person chooses, for Claude Code or for Codex, with the file contents the kit gives.\n\
5. Put no token, address or record value in any file. Tell the person what you wrote.",
    },
    Prompt {
        name: "explain_denial",
        description: "Explain why a request was refused and what would change it.",
        arguments: &[("error", "The error text, as the tool returned it", true)],
        template: "A request was refused: {error}\n\
Explain it to the person in plain words. Do not look for another route to the same data.\n\
1. Call list_workspaces to see what the person can reach.\n\
2. If the refusal names a workspace, call get_workspace to see their role and the workspace's rules.\n\
3. Say why it was refused, what would change it (a role, an approval, or a different request), and who can grant that.\n\
4. Offer to help write the request. Do not guess at a rule. Read it.",
    },
];

/// The prompts `prompts/list` adds to the ones that were already there.
pub fn listed() -> Vec<Value> {
    PROMPTS
        .iter()
        .map(|p| {
            let args: Vec<Value> = p.arguments.iter().map(|(n, d, r)| json!({ "name": n, "description": d, "required": r })).collect();
            json!({ "name": p.name, "description": p.description, "arguments": args })
        })
        .collect()
}

/// A prompt by name, with its arguments put in. `None` when no prompt has the name.
pub fn render(name: &str, args: &Value) -> Option<Value> {
    let p = PROMPTS.iter().find(|p| p.name == name)?;
    let mut text = p.template.to_string();
    for (n, _, _) in p.arguments {
        let value = args.get(*n).and_then(Value::as_str).unwrap_or("(not given)");
        text = text.replace(&format!("{{{n}}}"), value);
    }
    Some(json!({
        "description": p.description,
        "messages": [{ "role": "user", "content": { "type": "text", "text": text } }]
    }))
}
