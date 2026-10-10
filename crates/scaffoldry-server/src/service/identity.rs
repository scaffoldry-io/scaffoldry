//! Identity resolution, token generation, and setup tokens.
//! Built purely from stored rows (api_tokens, scim_users, roles, organizations).

use crate::state::{ApiToken, AuthUser, ServerState, SharedState};
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::str::FromStr;
use uuid::Uuid;

pub fn generate_raw_token() -> String {
    format!("scf_{:x}{:x}", Uuid::new_v4().as_simple(), Uuid::new_v4().as_simple())
}

pub fn hash_token(raw_token: &str) -> String {
    let bytes = Sha256::digest(raw_token.as_bytes());
    let mut s = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

/// Resolves an AuthUser from stored rows for the given eppn.
/// A token carries no name, no affiliation, and no department.
pub fn resolve_user(eppn: &str, state: &SharedState) -> Option<AuthUser> {
    if eppn == "setup@scaffoldry.local" {
        return Some(AuthUser {
            eppn: "setup@scaffoldry.local".to_string(),
            name: "Setup Identity".to_string(),
            role_title: "Setup".to_string(),
            affiliation: "central_admin".to_string(),
            department: "System".to_string(),
        });
    }

    // Load SCIM user
    let scim_user = if let Some(ref repo) = state.repository {
        repo.get_scim_user_by_username(eppn).ok().flatten()
    } else {
        let users = state.users.read().ok()?;
        users.values().find(|u| u.user_name.eq_ignore_ascii_case(eppn)).cloned()
    };

    if let Some(ref u) = scim_user {
        if !u.active {
            return None;
        }
    }

    let (def_name, def_role, def_aff, def_dept) = default_persona_attributes(eppn);

    // Resolve name
    let name = scim_user.as_ref().and_then(|u| {
        u.name.get("formatted").and_then(|v| v.as_str()).map(String::from).or_else(|| {
            let given = u.name.get("givenName").and_then(|v| v.as_str()).unwrap_or("");
            let family = u.name.get("familyName").and_then(|v| v.as_str()).unwrap_or("");
            let full = format!("{} {}", given, family).trim().to_string();
            if full.is_empty() { None } else { Some(full) }
        })
    }).unwrap_or(def_name);

    // Resolve department
    let department = scim_user.as_ref().and_then(|u| {
        u.enterprise_extension.as_ref()
            .and_then(|e| e.get("department"))
            .and_then(|v| v.as_str())
            .map(String::from)
    }).unwrap_or(def_dept);

    // Resolve role_title
    let role_title = scim_user.as_ref().and_then(|u| u.title.clone()).unwrap_or(def_role);

    // Resolve affiliation:
    let is_platform_admin = if let Some(ref repo) = state.repository {
        repo.with_client({
            let eppn_owned = eppn.to_string();
            move |client| {
                let row = client.query_one(
                    "SELECT COUNT(*) FROM roles r                      JOIN persons p ON r.person_id = p.id                      JOIN organizations o ON r.organization_id = o.id                      WHERE LOWER(p.eppn) = LOWER($1)                        AND r.scoped_affiliation = 'platform_admin'                        AND o.parent_id IS NULL",
                    &[&eppn_owned],
                )?;
                let cnt: i64 = row.get(0);
                Ok(cnt > 0)
            }
        }).unwrap_or(false)
    } else {
        let roles = state.roles.read().unwrap();
        let orgs = state.organizations.read().unwrap();
        roles.iter().any(|r| {
            r.eppn.eq_ignore_ascii_case(eppn)
                && r.scoped_affiliation == "platform_admin"
                && orgs.values().any(|o| o.id == r.organization_id && o.parent_id.is_none())
        })
    };

    let affiliation = if is_platform_admin {
        "central_admin".to_string()
    } else if let Some(ref u) = scim_user {
        let mut aff = "member".to_string();
        for r in &u.roles {
            let typ = r.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if typ.eq_ignore_ascii_case("eduPersonScopedAffiliation") {
                if let Some(val) = r.get("value").and_then(|v| v.as_str()) {
                    let aff_str = val.split('@').next().unwrap_or(val);
                    if let Ok(parsed) = scaffoldry_core::standards::eduperson::EduPersonAffiliation::from_str(aff_str) {
                        aff = match parsed {
                            scaffoldry_core::standards::eduperson::EduPersonAffiliation::Faculty => "faculty",
                            scaffoldry_core::standards::eduperson::EduPersonAffiliation::Student => "student",
                            scaffoldry_core::standards::eduperson::EduPersonAffiliation::Staff => "staff",
                            scaffoldry_core::standards::eduperson::EduPersonAffiliation::Employee => "employee",
                            scaffoldry_core::standards::eduperson::EduPersonAffiliation::Member => "member",
                            scaffoldry_core::standards::eduperson::EduPersonAffiliation::Affiliate => "affiliate",
                            scaffoldry_core::standards::eduperson::EduPersonAffiliation::Alum => "alum",
                        }.to_string();
                        break;
                    }
                }
            }
        }
        aff
    } else {
        if def_aff == "central_admin" { "member".to_string() } else { def_aff }
    };

    Some(AuthUser {
        eppn: eppn.to_string(),
        name,
        role_title,
        affiliation,
        department,
    })
}

pub fn boot_setup_token_needed(state: &ServerState) -> bool {
    let token_count = if let Some(ref repo) = state.repository {
        repo.count_api_tokens().unwrap_or(0)
    } else {
        state.api_tokens.read().map(|t| t.len() as i64).unwrap_or(0)
    };
    if token_count > 0 {
        return false;
    }

    let has_platform_admin = if let Some(ref repo) = state.repository {
        repo.with_client(|client| {
            let row = client.query_one(
                "SELECT COUNT(*) FROM roles r                  JOIN organizations o ON r.organization_id = o.id                  WHERE r.scoped_affiliation = 'platform_admin'                    AND o.parent_id IS NULL",
                &[],
            )?;
            let cnt: i64 = row.get(0);
            Ok(cnt > 0)
        }).unwrap_or(false)
    } else {
        let roles = state.roles.read().unwrap();
        let orgs = state.organizations.read().unwrap();
        roles.iter().any(|r| {
            r.scoped_affiliation == "platform_admin"
                && orgs.values().any(|o| o.id == r.organization_id && o.parent_id.is_none())
        })
    };

    !has_platform_admin
}

pub fn create_setup_token(state: &ServerState) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let raw_token = generate_raw_token();
    let token_hash = hash_token(&raw_token);
    let id = Uuid::new_v4();
    let token = ApiToken {
        token_hash,
        id,
        kind: "setup".to_string(),
        eppn: "setup@scaffoldry.local".to_string(),
        label: "Boot Setup Token".to_string(),
        original_admin: None,
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(24),
        last_used_at: None,
        revoked_at: None,
    };
    state.persist_api_token(&token)?;
    Ok(raw_token)
}

pub fn default_persona_attributes(eppn: &str) -> (String, String, String, String) {
    if eppn == "jordan.lee@state.edu" {
        (
            "Jordan Lee".to_string(),
            "Central Enterprise Administrator".to_string(),
            "central_admin".to_string(),
            "Central IT & Institutional Governance".to_string(),
        )
    } else if eppn.contains("curie") {
        (
            "Dr. Marie Curie".to_string(),
            "Professor & Lab Director".to_string(),
            "faculty".to_string(),
            "biology".to_string(),
        )
    } else if eppn.contains("connor") || eppn.contains("sarah") {
        (
            "Dr. Sarah Connor".to_string(),
            "Department Chair & Professor".to_string(),
            "faculty".to_string(),
            "Computer Science".to_string(),
        )
    } else if eppn.contains("vance") {
        (
            "Marcus Vance".to_string(),
            "Senior Research Administrator".to_string(),
            "staff".to_string(),
            "Office of Sponsored Programs".to_string(),
        )
    } else if eppn.contains("einstein") {
        (
            "Albert Einstein".to_string(),
            "Professor of Physics".to_string(),
            "faculty".to_string(),
            "physics".to_string(),
        )
    } else if eppn.contains("student") {
        (
            "Alice Smith".to_string(),
            "Graduate Research Assistant".to_string(),
            "student".to_string(),
            "biology".to_string(),
        )
    } else if eppn.contains("admin") {
        (
            "Admin User".to_string(),
            "Department Administrator".to_string(),
            "staff".to_string(),
            "biology".to_string(),
        )
    } else if eppn.contains("editor") {
        (
            "Editor User".to_string(),
            "Research Assistant".to_string(),
            "staff".to_string(),
            "biology".to_string(),
        )
    } else if eppn.contains("owner") {
        (
            "Owner User".to_string(),
            "Principal Investigator".to_string(),
            "faculty".to_string(),
            "biology".to_string(),
        )
    } else {
        (
            eppn.to_string(),
            "Member".to_string(),
            "member".to_string(),
            "general".to_string(),
        )
    }
}

/// Helper for tests to create a valid API token and ensure SCIM persona exists in DB.
pub fn issue_test_token_and_user(eppn: &str) -> String {
    let eppn_owned = eppn.to_string();
    std::thread::spawn(move || {
        let eppn = &eppn_owned;
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string()
    });

    let raw_token = generate_raw_token();
    let token_hash = hash_token(&raw_token);
    let token_id = Uuid::new_v4();

    let mut client = match postgres::Client::connect(&db_url, postgres::NoTls) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("DEBUG: issue_test_token_and_user DB connect error: {e:?}");
            return raw_token;
        }
    };
    if true {
        let root_org_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let _ = client.execute(
            "INSERT INTO organizations (id, parent_id, name, code, org_type)              VALUES ($1, NULL, 'Institution', 'INST', 'Institution')              ON CONFLICT (id) DO NOTHING",
            &[&root_org_id],
        );

        let (name, role_title, aff, dept) = default_persona_attributes(eppn);

        let (first_name, last_name) = match name.split_once(' ') {
            Some((f, l)) => (f, l),
            None => (name.as_str(), "User"),
        };
        let email = eppn.to_string();
        let person_id = Uuid::new_v4();
        let _ = client.execute(
            "INSERT INTO persons (id, first_name, last_name, email, eppn) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (eppn) DO UPDATE SET first_name = EXCLUDED.first_name, last_name = EXCLUDED.last_name, email = EXCLUDED.email",
            &[&person_id, &first_name, &last_name, &email, &eppn],
        );

        if aff == "central_admin" {
            if let Ok(p_row) = client.query_one("SELECT id FROM persons WHERE LOWER(eppn) = LOWER($1)", &[&eppn]) {
                let pid: Uuid = p_row.get(0);
                let role_id = Uuid::new_v4();
                let _ = client.execute(
                    "INSERT INTO roles (id, person_id, organization_id, role_title, scoped_affiliation, is_primary, source) VALUES ($1, $2, $3, $4, 'platform_admin', true, 'scim') ON CONFLICT (id) DO NOTHING",
                    &[&role_id, &pid, &root_org_id, &role_title],
                );
            }
        }

        let scim_id = format!("usr-{}", &hash_token(eppn)[..8]);
        let scim_roles = if aff == "central_admin" {
            serde_json::json!([
                { "type": "scaffoldry", "value": "platform_admin" }
            ])
        } else {
            serde_json::json!([
                { "type": "eduPersonScopedAffiliation", "value": format!("{aff}@state.edu") }
            ])
        };

        let payload = serde_json::json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
            "id": scim_id,
            "userName": eppn,
            "emails": [],
            "name": { "formatted": name },
            "title": role_title,
            "active": true,
            "roles": scim_roles,
            "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User": {
                "department": dept
            }
        });

        let _ = client.execute(
            "INSERT INTO scim_users (id, user_name, active, payload, updated_at)              VALUES ($1, $2, true, $3, NOW())              ON CONFLICT (id) DO UPDATE SET active = true, payload = $3, updated_at = NOW()",
            &[&scim_id, &eppn, &payload],
        );

        let expires_at = Utc::now() + chrono::Duration::days(30);
        if let Err(e) = client.execute(
            "INSERT INTO api_tokens (token_hash, id, kind, eppn, label, created_at, expires_at) VALUES ($1, $2, 'impersonation', $3, 'test token', NOW(), $4)",
            &[&token_hash, &token_id, &eppn, &expires_at],
        ) { eprintln!("DEBUG: api_tokens insert error: {e:?}"); }
    }

        raw_token
    }).join().expect("issue_test_token_and_user thread panicked")
}
