//! What the dataset catalog shows. The HTTP routes and the MCP tool both ask here, so they cannot drift apart.

use crate::state::AuthUser;

/// Whether the catalog may show a dataset's sample rows to this caller. A Platform Admin sees every sample.
/// Everyone else sees samples only of public and directory data.
pub fn sample_visible(caller_is_admin: bool, sensitivity_level: &str) -> bool {
    caller_is_admin || matches!(sensitivity_level.to_lowercase().as_str(), "public" | "directory")
}

/// The same, for a signed-in caller.
pub fn sample_visible_to(caller: &AuthUser, sensitivity_level: &str) -> bool {
    sample_visible(caller.affiliation == "central_admin", sensitivity_level)
}
