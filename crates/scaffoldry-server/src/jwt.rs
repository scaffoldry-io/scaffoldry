//! Tokens from the institution's identity provider (foundation phase 10).
//!
//! Scaffoldry is a resource server. It verifies a token the provider signed and signs nothing itself. The issuer, the
//! audience and the provider's JWKS are settings (`oidc.issuer`, `oidc.audience`, `oidc.jwks`), read on every request, so
//! replacing the keys takes effect at once. Only RS256 and ES256 are accepted. The only claims trusted are the ones that
//! name the person: `eppn`, else `sub`. A claim never grants a role. RFC 7519, RFC 7517 and RFC 8725.

use crate::state::SharedState;
use jsonwebtoken::jwk::{AlgorithmParameters, JwkSet};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::Value;

/// The seconds of clock difference between the provider and this server that are forgiven.
const LEEWAY_SECONDS: u64 = 30;

#[derive(Deserialize)]
struct Claims {
    sub: Option<String>,
    eppn: Option<String>,
    /// RFC 9068: the client the token was issued to.
    client_id: Option<String>,
    /// OpenID Connect: the party the token was issued to.
    azp: Option<String>,
}

/// What a valid provider token says: who the person is and, when it names one, which client it was issued to.
pub struct Verified {
    pub person: String,
    pub client: Option<String>,
}

fn setting(state: &SharedState, key: &str) -> Option<Value> {
    state.settings.read().ok().and_then(|s| s.get(key).cloned())
}

fn text(state: &SharedState, key: &str) -> Option<String> {
    setting(state, key).and_then(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string))
}

/// Whether a bearer is shaped like a JWT: three parts joined by two dots. A stored token is a random string with no dot, so
/// the two never meet. A shaped token that does not verify is refused, never looked up.
pub fn is_jwt_shaped(token: &str) -> bool {
    token.matches('.').count() == 2
}

/// The issuer to name in the protected-resource document, when one is set.
pub fn configured_issuer(state: &SharedState) -> Option<String> {
    text(state, "oidc.issuer")
}

/// The person (and client) a provider token names, when the token is valid. None when it is not, or when any of the three settings is
/// missing.
pub fn validate_jwt(token: &str, state: &SharedState) -> Option<Verified> {
    let (issuer, audience) = (text(state, "oidc.issuer")?, text(state, "oidc.audience")?);
    let jwks: JwkSet = serde_json::from_value(setting(state, "oidc.jwks").filter(|v| v.is_object())?).ok()?;
    let header = decode_header(token).ok()?;
    if !matches!(header.alg, Algorithm::RS256 | Algorithm::ES256) {
        return None;
    }
    let mut validation = Validation::new(header.alg);
    validation.set_issuer(&[issuer.as_str()]);
    validation.set_audience(&[audience.as_str()]);
    validation.set_required_spec_claims(&["exp", "iss", "aud"]);
    validation.validate_nbf = true;
    validation.leeway = LEEWAY_SECONDS;
    for jwk in &jwks.keys {
        let family_matches = matches!(
            (&jwk.algorithm, header.alg),
            (AlgorithmParameters::RSA(_), Algorithm::RS256) | (AlgorithmParameters::EllipticCurve(_), Algorithm::ES256)
        );
        let kid_matches = header.kid.as_ref().is_none_or(|kid| jwk.common.key_id.as_ref() == Some(kid));
        if !family_matches || !kid_matches {
            continue;
        }
        let Ok(key) = DecodingKey::from_jwk(jwk) else { continue };
        if let Ok(data) = decode::<Claims>(token, &key, &validation) {
            let person = data.claims.eppn.or(data.claims.sub).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())?;
            let client = data.claims.client_id.or(data.claims.azp).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            return Some(Verified { person, client });
        }
    }
    None
}
