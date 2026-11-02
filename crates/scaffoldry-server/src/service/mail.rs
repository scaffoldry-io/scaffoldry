//! Notifications phase 2: the mail console's rules. A Platform Admin sees whether mail is on, the queue and the
//! failures, and may send a test mail. The test goes only to the caller, needs a reason, and is a `MailTested` ledger
//! entry that names the recipient's domain.

use crate::service::admin::{admin_write, require_platform_admin};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::ledger::DecisionType;
use serde_json::{json, Value};

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

/// Whether mail is on, the queue's depth and age, and the last failures by the recipient's domain.
pub fn status(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    crate::mail::status(&state.repository).map_err(storage)
}

/// Queues one mail to the caller. Whatever else the body says, it goes to nobody else.
pub fn send_test(caller: &AuthUser, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let reason = body.get("reason").and_then(Value::as_str).unwrap_or("").to_string();
    if reason.trim().is_empty() {
        return Err(ServiceError::bad_request("A reason between 1 and 500 characters is required for administrative actions"));
    }
    let settings = state.settings.read().unwrap_or_else(|p| p.into_inner()).clone();
    crate::mail::mailing(&settings).map_err(ServiceError::conflict)?;
    let user = state.repository.users_by_names(std::slice::from_ref(&caller.eppn)).map_err(storage)?;
    let email = user.first().and_then(crate::service::people::primary_email);
    let eppn = caller.eppn.clone();
    let to = state
        .repository
        .with_client(move |c| {
            let mut tx = c.transaction()?;
            let to = crate::mail::address_of(&mut tx, &eppn, email.as_deref())?;
            tx.commit()?;
            Ok(to)
        })
        .map_err(storage)?
        .ok_or_else(|| ServiceError::bad_request("Your account has no email address to send a test to"))?;
    let domain = crate::mail::domain_of(&to);
    admin_write(state, caller, DecisionType::MailTested, "SC-08", &reason, &json!({ "domain": domain }), || {
        let to = to.clone();
        state
            .repository
            .with_client(move |c| {
                let mut tx = c.transaction()?;
                let queued = crate::mail::queue(
                    &mut tx,
                    None,
                    &to,
                    "Test mail from Scaffoldry",
                    "This is a test mail, sent from the console to check that mail can leave the server. Nothing needs doing.\n",
                )?;
                tx.commit()?;
                Ok(queued)
            })
            .map_err(storage)?
            .then_some(())
            .ok_or_else(|| ServiceError::RateLimited("You have had 50 mails in the last hour. Try again later".to_string()))
    })?;
    Ok(json!({ "queued": true, "domain": domain }))
}
