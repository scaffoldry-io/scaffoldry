use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write;
use thiserror::Error;

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{:02x}", b);
    }
    s
}

pub const GENESIS_PREVIOUS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Error)]
pub enum LedgerError {
    #[error("Ledger chain is broken at sequence {sequence}: expected previous hash {expected}, got {found}")]
    BrokenChain {
        sequence: u64,
        expected: String,
        found: String,
    },
    #[error("Ledger entry hash mismatch at sequence {sequence}: computed {computed}, recorded {recorded}")]
    HashMismatch {
        sequence: u64,
        computed: String,
        recorded: String,
    },
    #[error("Invalid sequence number: expected {expected}, got {found}")]
    SequenceMismatch {
        expected: u64,
        found: u64,
    },
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
    #[error("Repository error: {0}")]
    RepositoryError(String),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum DecisionType {
    AppPublished,
    VanityDnsBound,
    PolicyRevision,
    WorkflowRuleApproved,
    AccessRoleGranted,
    DatasetAccessShared,
    StatutoryAttestation,
    ImpersonationSessionStarted,
    ImpersonationSessionEnded,
    WorkspaceCreated,
    WorkspaceUpdated,
    WorkspaceMemberAdded,
    WorkspaceMemberRemoved,
    WorkspaceMemberRoleUpdated,
    UserAccessChanged,
    AccessRoleRevoked,
    TokenRevoked,
    DataLabelChanged,
    ProcessDefinitionChanged,
    ProcessInstanceReassigned,
    ProcessInstanceCancelled,
}

impl DecisionType {
    pub const ALL: &'static [DecisionType] = &[
        DecisionType::AppPublished,
        DecisionType::VanityDnsBound,
        DecisionType::PolicyRevision,
        DecisionType::WorkflowRuleApproved,
        DecisionType::AccessRoleGranted,
        DecisionType::DatasetAccessShared,
        DecisionType::StatutoryAttestation,
        DecisionType::ImpersonationSessionStarted,
        DecisionType::ImpersonationSessionEnded,
        DecisionType::WorkspaceCreated,
        DecisionType::WorkspaceUpdated,
        DecisionType::WorkspaceMemberAdded,
        DecisionType::WorkspaceMemberRemoved,
        DecisionType::WorkspaceMemberRoleUpdated,
        DecisionType::UserAccessChanged,
        DecisionType::AccessRoleRevoked,
        DecisionType::TokenRevoked,
        DecisionType::DataLabelChanged,
        DecisionType::ProcessDefinitionChanged,
        DecisionType::ProcessInstanceReassigned,
        DecisionType::ProcessInstanceCancelled,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            DecisionType::AppPublished => "AppPublished",
            DecisionType::VanityDnsBound => "VanityDnsBound",
            DecisionType::PolicyRevision => "PolicyRevision",
            DecisionType::WorkflowRuleApproved => "WorkflowRuleApproved",
            DecisionType::AccessRoleGranted => "AccessRoleGranted",
            DecisionType::DatasetAccessShared => "DatasetAccessShared",
            DecisionType::StatutoryAttestation => "StatutoryAttestation",
            DecisionType::ImpersonationSessionStarted => "ImpersonationSessionStarted",
            DecisionType::ImpersonationSessionEnded => "ImpersonationSessionEnded",
            DecisionType::WorkspaceCreated => "WorkspaceCreated",
            DecisionType::WorkspaceUpdated => "WorkspaceUpdated",
            DecisionType::WorkspaceMemberAdded => "WorkspaceMemberAdded",
            DecisionType::WorkspaceMemberRemoved => "WorkspaceMemberRemoved",
            DecisionType::WorkspaceMemberRoleUpdated => "WorkspaceMemberRoleUpdated",
            DecisionType::UserAccessChanged => "UserAccessChanged",
            DecisionType::AccessRoleRevoked => "AccessRoleRevoked",
            DecisionType::TokenRevoked => "TokenRevoked",
            DecisionType::DataLabelChanged => "DataLabelChanged",
            DecisionType::ProcessDefinitionChanged => "ProcessDefinitionChanged",
            DecisionType::ProcessInstanceReassigned => "ProcessInstanceReassigned",
            DecisionType::ProcessInstanceCancelled => "ProcessInstanceCancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().find(|d| d.as_str() == s).copied()
    }
}

#[derive(Debug, Clone)]
pub struct LedgerHashInput<'a> {
    pub sequence: u64,
    pub timestamp_iso: &'a str,
    pub previous_hash: &'a str,
    pub principal: &'a str,
    pub organization_code: &'a str,
    pub app_slug: Option<&'a str>,
    pub decision_type: &'a DecisionType,
    pub oscal_control_id: &'a str,
    pub rationale: &'a str,
    pub payload_hash: &'a str,
}

#[derive(Debug, Clone)]
pub struct NewLedgerEntryParams<'a> {
    pub sequence: u64,
    pub timestamp_iso: String,
    pub previous_hash: String,
    pub principal: String,
    pub organization_code: String,
    pub app_slug: Option<String>,
    pub decision_type: DecisionType,
    pub oscal_control_id: String,
    pub rationale: String,
    pub payload: &'a serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LedgerEntry {
    pub sequence: u64,
    pub timestamp_iso: String,
    pub previous_hash: String,
    pub principal: String,
    pub organization_code: String,
    pub app_slug: Option<String>,
    pub decision_type: DecisionType,
    pub oscal_control_id: String,
    pub rationale: String,
    pub payload_hash: String,
    pub entry_hash: String,
}

impl LedgerEntry {
    pub fn compute_payload_hash(payload: &serde_json::Value) -> String {
        let serialized = serde_json::to_string(payload).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(serialized.as_bytes());
        bytes_to_hex(&hasher.finalize())
    }

    pub fn compute_entry_hash(input: &LedgerHashInput<'_>) -> String {
        let dec_str = input.decision_type.as_str();

        let content = format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            input.sequence,
            input.timestamp_iso,
            input.previous_hash,
            input.principal,
            input.organization_code,
            input.app_slug.unwrap_or(""),
            dec_str,
            input.oscal_control_id,
            input.rationale,
            input.payload_hash
        );

        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        bytes_to_hex(&hasher.finalize())
    }

    pub fn new(params: NewLedgerEntryParams<'_>) -> Self {
        let payload_hash = Self::compute_payload_hash(params.payload);
        let entry_hash = Self::compute_entry_hash(&LedgerHashInput {
            sequence: params.sequence,
            timestamp_iso: &params.timestamp_iso,
            previous_hash: &params.previous_hash,
            principal: &params.principal,
            organization_code: &params.organization_code,
            app_slug: params.app_slug.as_deref(),
            decision_type: &params.decision_type,
            oscal_control_id: &params.oscal_control_id,
            rationale: &params.rationale,
            payload_hash: &payload_hash,
        });

        Self {
            sequence: params.sequence,
            timestamp_iso: params.timestamp_iso,
            previous_hash: params.previous_hash,
            principal: params.principal,
            organization_code: params.organization_code,
            app_slug: params.app_slug,
            decision_type: params.decision_type,
            oscal_control_id: params.oscal_control_id,
            rationale: params.rationale,
            payload_hash,
            entry_hash,
        }
    }

    pub fn verify(&self) -> Result<(), LedgerError> {
        let computed = Self::compute_entry_hash(&LedgerHashInput {
            sequence: self.sequence,
            timestamp_iso: &self.timestamp_iso,
            previous_hash: &self.previous_hash,
            principal: &self.principal,
            organization_code: &self.organization_code,
            app_slug: self.app_slug.as_deref(),
            decision_type: &self.decision_type,
            oscal_control_id: &self.oscal_control_id,
            rationale: &self.rationale,
            payload_hash: &self.payload_hash,
        });

        if computed != self.entry_hash {
            return Err(LedgerError::HashMismatch {
                sequence: self.sequence,
                computed,
                recorded: self.entry_hash.clone(),
            });
        }

        Ok(())
    }
}

pub fn verify_ledger_chain(chain: &[LedgerEntry]) -> Result<bool, LedgerError> {
    if chain.is_empty() {
        return Ok(true);
    }

    for (i, entry) in chain.iter().enumerate() {
        entry.verify()?;

        if entry.sequence != i as u64 {
            return Err(LedgerError::SequenceMismatch {
                expected: i as u64,
                found: entry.sequence,
            });
        }

        if i == 0 {
            if entry.previous_hash != GENESIS_PREVIOUS_HASH {
                return Err(LedgerError::BrokenChain {
                    sequence: 0,
                    expected: GENESIS_PREVIOUS_HASH.into(),
                    found: entry.previous_hash.clone(),
                });
            }
        } else {
            let prev_entry = &chain[i - 1];
            if entry.previous_hash != prev_entry.entry_hash {
                return Err(LedgerError::BrokenChain {
                    sequence: entry.sequence,
                    expected: prev_entry.entry_hash.clone(),
                    found: entry.previous_hash.clone(),
                });
            }
        }
    }

    Ok(true)
}
