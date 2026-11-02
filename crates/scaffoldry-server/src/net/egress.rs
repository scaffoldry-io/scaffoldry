//! Stub: integrations phase 1 is not built yet. Everything is refused.
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

#[derive(Debug)]
pub enum EgressError {
    NotAllowed(String),
    Blocked(String),
    Redirect(String),
    Failed(String),
}

pub trait Resolver: Send + Sync {
    fn lookup(&self, host: &str, port: u16) -> std::io::Result<Vec<IpAddr>>;
}

pub struct Allow;

impl Allow {
    pub fn parse(_v: &serde_json::Value) -> Result<Allow, String> {
        Err("not built".into())
    }
}

pub fn resolve_and_check(_host: &str, _port: u16, _allow: &Allow, _resolver: &dyn Resolver) -> Result<SocketAddr, EgressError> {
    Err(EgressError::Failed("not built".into()))
}

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

pub struct Client;

impl Client {
    pub fn new(_allow: Allow, _resolver: Arc<dyn Resolver>) -> Self {
        Client
    }
    pub fn get(&self, _url: &str) -> Result<Response, EgressError> {
        Err(EgressError::Failed("not built".into()))
    }
}
