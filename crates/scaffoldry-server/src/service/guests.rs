//! Guests phase 1: a guest and a link.
//!
//! A guest is a person outside the directory who proves they own an email address and may use one form. They ask for a link,
//! which works once for 15 minutes, and trade it for a session of eight hours limited to that app and form. Nothing here
//! says whether an address has applied before: the start answers the same sentence whether or not it did anything.
//!
//! A guest's identity inside the system is `guest:{id}@guest.invalid`. That name cannot resolve, and `resolve_user` knows
//! nothing about it, so a guest is never a directory person and cannot be added to a workspace.

use crate::service::rate_limit::{hit, Window};
use crate::service::ServiceError;
use crate::state::{ApiToken, AuthUser, SharedState};
use aes_gcm::aead::{rand_core::RngCore, OsRng};
use scaffoldry_core::{TokenOp, TokenScope};
use serde_json::{json, Value};
use uuid::Uuid;

/// What a start answers, whatever it did.
pub const NEUTRAL: &str = "If that address can use this form, a link is on its way.";
/// The domain of every guest address. It is a name that cannot resolve.
pub const DOMAIN: &str = "guest.invalid";
const LINK_MINUTES: i64 = 15;
const SESSION_HOURS: i64 = 8;
/// Requests an hour: for one address, for one network address, and for one form.
pub const PER_ADDRESS: i32 = 3;
pub const PER_NETWORK: i32 = 20;
pub const PER_FORM: i32 = 200;

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

/// The address a guest has inside the system.
pub fn address_of(id: Uuid) -> String {
    format!("guest:{id}@{DOMAIN}")
}

/// The guest an address names, when it is a guest's.
pub fn id_of(address: &str) -> Option<Uuid> {
    address.strip_prefix("guest:")?.strip_suffix(&format!("@{DOMAIN}"))?.parse().ok()
}

/// Whether an address is a guest's. A guest is given nothing a directory person is given.
pub fn is_guest_address(address: &str) -> bool {
    address.trim().to_lowercase().ends_with(&format!("@{DOMAIN}"))
}

fn setting_bool(state: &SharedState, key: &str) -> bool {
    state.settings.read().ok().and_then(|s| s.get(key).and_then(Value::as_bool)).unwrap_or(false)
}

fn closed_forms(state: &SharedState) -> Vec<String> {
    state
        .settings
        .read()
        .ok()
        .and_then(|s| s.get("forms.disabled").cloned())
        .and_then(|v| v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()))
        .unwrap_or_default()
}

/// The hook an institution adds a check through, such as a CAPTCHA. With `guests.captcha_hook` empty it returns true. A start
/// that fails the check is answered like any other, and sends nothing. The check sees the request body and nothing else.
pub fn captcha_ok(state: &SharedState, _body: &Value) -> bool {
    let _hook = state.settings.read().ok().and_then(|s| s.get("guests.captcha_hook").and_then(Value::as_str).map(str::to_string));
    true
}

/// An address as it is kept: trimmed and in lower case, or `None` when it is not one.
fn clean_email(v: Option<&Value>) -> Option<String> {
    let email = v?.as_str()?.trim().to_lowercase();
    let (local, domain) = email.split_once('@')?;
    let ok = !local.is_empty()
        && email.len() <= 320
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !email.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | ',' | ';' | '"'))
        && email.matches('@').count() == 1;
    ok.then_some(email)
}

/// Whether a form accepts guests: it is a form view of this app, its audience is `Guests`, and nobody closed it.
fn open_form(state: &SharedState, app: &str, form: Uuid) -> bool {
    use scaffoldry_engine::HostRouter;
    if closed_forms(state).contains(&form.to_string()) || state.engine.read().ok().and_then(|e| e.resolve_by_slug(app).map(|_| ())).is_none() {
        return false;
    }
    state.repository.get_app_view(app, form).ok().flatten().is_some_and(|v| v.kind == "form" && v.config.get("audience").and_then(Value::as_str) == Some("Guests"))
}

fn token(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    OsRng.fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// `POST /guest/{app}/{form}/start`: asks for a link. The answer is `NEUTRAL` for every request that is not refused for
/// asking too often, so it cannot be used to learn who has applied.
pub fn start(state: &SharedState, headers: &axum::http::HeaderMap, app: &str, form: &str, body: &Value) -> Result<Value, ServiceError> {
    let neutral = || Ok(json!({ "message": NEUTRAL }));
    // A hidden field people never see. A bot that fills it gets the same answer as everyone, and nothing happens.
    if body.get("website").and_then(Value::as_str).is_some_and(|w| !w.trim().is_empty()) {
        return neutral();
    }
    if !setting_bool(state, "guests.enabled") {
        return neutral();
    }
    let Some(email) = clean_email(body.get("email")) else { return neutral() };
    if !captcha_ok(state, body) {
        return neutral();
    }

    let network = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|v| v.split(',').next()).map(str::trim).filter(|a| !a.is_empty()).unwrap_or("unknown").to_string();
    hit(state, format!("guest-addr:{email}"), PER_ADDRESS, Window::Hour)?;
    hit(state, format!("guest-net:{network}"), PER_NETWORK, Window::Hour)?;
    hit(state, format!("guest-form:{form}"), PER_FORM, Window::Hour)?;

    let Ok(form_id) = Uuid::parse_str(form) else { return neutral() };
    if !open_form(state, app, form_id) {
        return neutral();
    }
    // Mail has to be on and complete for a link to be sent. Otherwise nothing is made, so no guest exists that was never mailed.
    let Ok(base) = crate::mail::mailing(&state.settings.read().unwrap_or_else(|p| p.into_inner())) else { return neutral() };

    let (raw, id, app, to) = (token(16), Uuid::new_v4(), app.to_string(), email.clone());
    let hash = crate::service::identity::hash_token(&raw);
    state
        .repository
        .with_client(move |c| {
            let mut tx = c.transaction()?;
            let row = tx.query_one("INSERT INTO guests (id, email) VALUES ($1, $2) ON CONFLICT (email) DO UPDATE SET email = EXCLUDED.email RETURNING id, disabled_at IS NOT NULL", &[&id, &to])?;
            let (guest, disabled): (Uuid, bool) = (row.get(0), row.get(1));
            if disabled {
                return Ok(());
            }
            tx.execute(
                "INSERT INTO guest_links (token_hash, guest_id, app_slug, form_id, expires_at) VALUES ($1, $2, $3, $4, NOW() + make_interval(mins => $5::int))",
                &[&hash, &guest, &app, &form_id, &(LINK_MINUTES as i32)],
            )?;
            let text = format!(
                "Use this link to open the form: {base}/g/{raw}\n\nIt works once, and it stops working in {LINK_MINUTES} minutes. If you did not ask for it, you can ignore this mail.\n"
            );
            crate::mail::queue_sign_in_link(&mut tx, &to, "Your sign-in link", &text)?;
            tx.commit()?;
            Ok(())
        })
        .map_err(storage)?;
    neutral()
}

/// `POST /guest/verify`: trades a link for a session. A link that is wrong, used or expired is not found, whichever it is.
pub fn verify(state: &SharedState, body: &Value) -> Result<Value, ServiceError> {
    let not_found = || ServiceError::NotFound("This link does not work. It may have been used, or it may have expired.".to_string());
    let raw = body.get("token").and_then(Value::as_str).filter(|t| !t.is_empty() && t.len() <= 200).ok_or_else(not_found)?;
    let hash = crate::service::identity::hash_token(raw);
    // Used once: the update is what claims the link, so two requests cannot both have it.
    let claimed = state
        .repository
        .with_client(move |c| {
            let mut tx = c.transaction()?;
            let link = tx
                .query_opt(
                    "UPDATE guest_links SET used_at = NOW() WHERE token_hash = $1 AND used_at IS NULL AND expires_at > NOW() RETURNING guest_id, app_slug, form_id",
                    &[&hash],
                )?
                .map(|r| (r.get::<_, Uuid>(0), r.get::<_, String>(1), r.get::<_, Uuid>(2)));
            let Some((guest, app, form)) = link else { return Ok(None) };
            let live = tx.query_opt("UPDATE guests SET email_verified_at = COALESCE(email_verified_at, NOW()), last_seen_at = NOW() WHERE id = $1 AND disabled_at IS NULL RETURNING id", &[&guest])?;
            if live.is_none() {
                return Ok(None);
            }
            tx.commit()?;
            Ok(Some((guest, app, form)))
        })
        .map_err(storage)?;
    let (guest, app, form) = claimed.ok_or_else(not_found)?;
    if !setting_bool(state, "guests.enabled") {
        return Err(not_found());
    }

    let session = format!("scf_{}", token(32));
    let now = chrono::Utc::now();
    let expires = now + chrono::Duration::hours(SESSION_HOURS);
    // Limited to this app, and to no table: every ordinary route refuses it. The form's own routes read the form from the label.
    let scope = TokenScope { apps: vec![app.clone()], tables: std::collections::BTreeMap::<String, Vec<TokenOp>>::new(), tools: Some(Vec::new()), max_rows_per_call: None, level: None };
    let row = ApiToken {
        token_hash: crate::service::identity::hash_token(&session),
        id: Uuid::new_v4(),
        kind: "guest".to_string(),
        eppn: address_of(guest),
        label: format!("guest-form:{form}"),
        original_admin: None,
        created_at: now,
        expires_at: expires,
        last_used_at: None,
        revoked_at: None,
        app_slug: Some(app.clone()),
        scope: Some(scope),
    };
    state.persist_api_token(&row).map_err(|e| ServiceError::StorageUnavailable(e.to_string()))?;
    Ok(json!({ "token": session, "expires_at": expires.to_rfc3339(), "app": app, "form": form }))
}

/// The person a guest session is, when its guest may still use it. The row is read on every request, so a guest an
/// administrator disables, or guests turned off, stop at once.
pub fn session_user(state: &SharedState, row: &ApiToken) -> Option<AuthUser> {
    if !setting_bool(state, "guests.enabled") {
        return None;
    }
    let id = id_of(&row.eppn)?;
    let live: bool = state
        .repository
        .with_client(move |c| Ok(c.query_opt("SELECT disabled_at IS NULL FROM guests WHERE id = $1", &[&id])?.is_some_and(|r| r.get::<_, bool>(0))))
        .ok()?;
    live.then(|| AuthUser {
        eppn: row.eppn.clone(),
        name: "Guest".to_string(),
        role_title: "Guest".to_string(),
        affiliation: "guest".to_string(),
        department: String::new(),
        view_as_by: None,
        via_client: None,
        shared_link: None,
    })
}
