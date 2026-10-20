//! Workspace Guards: templates, sentences, and Cedar forbid compilation.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const MAX_GUARD_RULES: usize = 20;

pub const ALLOWED_AFFILIATIONS: &[&str] = &[
    "faculty",
    "student",
    "staff",
    "employee",
    "member",
    "affiliate",
    "alum",
    "compliance",
    "central_admin",
];

pub const ALLOWED_TEMPLATES: &[&str] = &[
    "deny_export_unless_affiliation",
    "deny_write_for_affiliation",
    "deny_access_outside_units",
    "deny_sensitive_unless_affiliation",
    "deny_outside_domain",
    "deny_above_sensitivity",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuardRule {
    pub template: String,
    #[serde(default)]
    pub affiliations: Vec<String>,
    #[serde(default)]
    pub unit_ids: Vec<String>,
    /// For `deny_outside_domain`: the domains an app may sit in, each with the domains below it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<String>,
    /// For `deny_above_sensitivity`: the highest level an app may have.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceGuardRecord {
    pub workspace_id: String,
    pub version: i32,
    pub rules: serde_json::Value,
    pub compiled: String,
    pub reason: String,
    pub created_by: String,
    pub created_at: String,
}

impl GuardRule {
    pub fn validate(&self) -> Result<(), String> {
        if !ALLOWED_TEMPLATES.contains(&self.template.as_str()) {
            return Err(format!("Unknown guard template: '{}'", self.template));
        }

        for aff in &self.affiliations {
            if !ALLOWED_AFFILIATIONS.contains(&aff.as_str()) {
                return Err(format!("Unknown affiliation: '{aff}'"));
            }
        }

        match self.template.as_str() {
            "deny_export_unless_affiliation"
            | "deny_write_for_affiliation"
            | "deny_sensitive_unless_affiliation"
                if self.affiliations.is_empty() =>
            {
                return Err(format!("Template '{}' requires at least one affiliation", self.template));
            }
            "deny_access_outside_units" if self.unit_ids.is_empty() => {
                return Err(format!("Template '{}' requires at least one unit_id", self.template));
            }
            "deny_outside_domain" if self.domains.is_empty() => {
                return Err(format!("Template '{}' requires at least one domain", self.template));
            }
            "deny_above_sensitivity" if self.sensitivity.as_deref().and_then(crate::coordinates::sensitivity_rank).is_none() => {
                return Err(format!("Template '{}' requires a sensitivity of {}", self.template, crate::coordinates::SENSITIVITY_LEVELS.join(", ")));
            }
            _ => {}
        }

        Ok(())
    }

    pub fn sentence(&self, unit_names: &HashMap<String, String>) -> String {
        match self.template.as_str() {
            "deny_export_unless_affiliation" => {
                format!("Only {} may export from this workspace.", self.affiliations.join(", "))
            }
            "deny_write_for_affiliation" => {
                format!("{} may not change records in this workspace.", self.affiliations.join(", "))
            }
            "deny_access_outside_units" => {
                let names: Vec<String> = self
                    .unit_ids
                    .iter()
                    .map(|id| unit_names.get(id).cloned().unwrap_or_else(|| id.clone()))
                    .collect();
                format!("Only people in {} may use this workspace.", names.join(", "))
            }
            "deny_sensitive_unless_affiliation" => {
                format!(
                    "Only {} may see or export sensitive student data here.",
                    self.affiliations.join(", ")
                )
            }
            "deny_outside_domain" => format!("Only apps in {} may be used in this workspace.", self.domains.join(", ")),
            "deny_above_sensitivity" => {
                format!("Apps above {} may not be used in this workspace.", self.sensitivity.as_deref().unwrap_or_default())
            }
            _ => String::new(),
        }
    }

    pub fn compile_policy_text(
        &self,
        workspace_id: &str,
        _n: usize,
        unit_names: &HashMap<String, String>,
    ) -> Result<String, String> {
        self.validate()?;
        let sentence = self.sentence(unit_names);
        let policy_str = match self.template.as_str() {
            "deny_export_unless_affiliation" => {
                let when_clause = self
                    .affiliations
                    .iter()
                    .map(|a| format!("principal.scoped_affiliation != \"{a}\""))
                    .collect::<Vec<_>>()
                    .join(" && ");
                format!(
                    r#"@description("{sentence}")
forbid (
    principal,
    action == Action::"export",
    resource
)
when {{
    resource.workspace_id == "{workspace_id}" &&
    {when_clause}
}};"#
                )
            }
            "deny_write_for_affiliation" => {
                let when_clause = self
                    .affiliations
                    .iter()
                    .map(|a| format!("principal.scoped_affiliation == \"{a}\""))
                    .collect::<Vec<_>>()
                    .join(" || ");
                format!(
                    r#"@description("{sentence}")
forbid (
    principal,
    action in [Action::"write", Action::"write_record"],
    resource
)
when {{
    resource.workspace_id == "{workspace_id}" &&
    ({when_clause})
}};"#
                )
            }
            "deny_access_outside_units" => {
                let when_clause = self
                    .unit_ids
                    .iter()
                    .map(|u| format!("!principal.unit_ids.contains(\"{u}\")"))
                    .collect::<Vec<_>>()
                    .join(" && ");
                format!(
                    r#"@description("{sentence}")
forbid (
    principal,
    action in [Action::"access_workspace", Action::"read_app", Action::"write_record", Action::"export", Action::"approve"],
    resource
)
when {{
    resource has workspace_id &&
    resource.workspace_id == "{workspace_id}" &&
    {when_clause}
}};"#
                )
            }
            "deny_sensitive_unless_affiliation" => {
                let when_clause = self
                    .affiliations
                    .iter()
                    .map(|a| format!("principal.scoped_affiliation != \"{a}\""))
                    .collect::<Vec<_>>()
                    .join(" && ");
                format!(
                    r#"@description("{sentence}")
forbid (
    principal,
    action in [Action::"read_app", Action::"export"],
    resource
)
when {{
    resource has is_ferpa_sensitive &&
    resource.is_ferpa_sensitive &&
    resource has workspace_id &&
    resource.workspace_id == "{workspace_id}" &&
    {when_clause}
}};"#
                )
            }
            "deny_outside_domain" => {
                let outside = self.domains.iter().map(|d| format!("!resource.domain_path.contains(\"{d}\")")).collect::<Vec<_>>().join(" && ");
                format!(
                    r#"@description("{sentence}")
forbid (
    principal,
    action in [Action::"read_app", Action::"write_record", Action::"export"],
    resource
)
when {{
    resource has domain_path &&
    resource has workspace_id &&
    resource.workspace_id == "{workspace_id}" &&
    {outside}
}};"#
                )
            }
            "deny_above_sensitivity" => {
                let rank = self.sensitivity.as_deref().and_then(crate::coordinates::sensitivity_rank).unwrap_or_default();
                format!(
                    r#"@description("{sentence}")
forbid (
    principal,
    action in [Action::"read_app", Action::"write_record", Action::"export"],
    resource
)
when {{
    resource has sensitivity_rank &&
    resource has workspace_id &&
    resource.workspace_id == "{workspace_id}" &&
    resource.sensitivity_rank > {rank}
}};"#
                )
            }
            _ => return Err(format!("Unsupported template: {}", self.template)),
        };

        // Extra check: A guard can only forbid!
        if policy_str.contains("permit") {
            return Err("A guard rule cannot compile to permit".to_string());
        }

        Ok(policy_str)
    }
}

pub fn compile_rules(
    workspace_id: &str,
    rules: &[GuardRule],
    unit_names: &HashMap<String, String>,
) -> Result<(String, Vec<String>), String> {
    if rules.len() > MAX_GUARD_RULES {
        return Err(format!(
            "Rules count {} exceeds maximum allowed of {}",
            rules.len(),
            MAX_GUARD_RULES
        ));
    }
    let mut compiled_parts = Vec::new();
    let mut sentences = Vec::new();
    for (i, rule) in rules.iter().enumerate() {
        rule.validate()?;
        let sentence = rule.sentence(unit_names);
        let policy_str = rule.compile_policy_text(workspace_id, i + 1, unit_names)?;
        compiled_parts.push(policy_str);
        sentences.push(sentence);
    }
    Ok((compiled_parts.join("\n\n"), sentences))
}
