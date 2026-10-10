//! Institutional OIDC configuration constants.

pub const DEFAULT_ISSUER: &str = "https://auth.state.edu";

pub fn get_jwt_issuer() -> String {
    std::env::var("SCAFFOLDRY_OIDC_ISSUER").unwrap_or_else(|_| DEFAULT_ISSUER.to_string())
}
