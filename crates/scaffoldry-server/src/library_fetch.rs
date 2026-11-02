//! Stub: policy library phase 5 is not built yet.
use crate::net::egress::Client;
use crate::state::ServerState;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Checked {
    pub status: String,
    pub stored: bool,
    pub marked: Vec<Uuid>,
}

pub fn check_source(_state: &ServerState, _source: Uuid, _client: &Client) -> Result<Checked, String> {
    Err("not built".into())
}

pub fn check_due(_state: &ServerState, _client: &Client) -> Result<usize, String> {
    Err("not built".into())
}
