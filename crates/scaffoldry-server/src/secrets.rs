//! Stub: integrations phase 1 is not built yet.
use crate::repository::PostgresRepository;
use crate::state::ServerState;
use std::path::Path;

#[derive(Debug)]
pub struct MasterKey;
#[derive(Debug)]
pub struct Secret;
#[derive(Debug)]
pub struct SecretError;

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not built")
    }
}

impl MasterKey {
    pub fn from_file(_p: &Path) -> Result<MasterKey, SecretError> {
        Err(SecretError)
    }
}

impl Secret {
    pub fn expose(&self) -> &[u8] {
        &[]
    }
}

pub fn seal(_k: &MasterKey, _id: uuid::Uuid, _v: &[u8]) -> (Vec<u8>, Vec<u8>) {
    (Vec::new(), Vec::new())
}
pub fn open(_k: &MasterKey, _id: uuid::Uuid, _c: &[u8], _n: &[u8]) -> Result<Secret, SecretError> {
    Err(SecretError)
}
pub fn get(_s: &ServerState, _name: &str) -> Result<Secret, SecretError> {
    Err(SecretError)
}
pub fn boot_check(_r: &PostgresRepository, _k: Option<&Path>) -> Result<(), String> {
    Err("not built".into())
}
pub fn rotate_master_key(_r: &PostgresRepository, _o: &Path, _n: &Path) -> Result<usize, String> {
    Err("not built".into())
}
