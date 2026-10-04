//! Server State and Storage for Datasets, Workspaces, and SCIM Identity

use chrono::Utc;
use scaffoldry_core::{
    verify_ledger_chain, ActionType, AutomationRule, ConditionOperator, DatasetField,
    DatasetRelationship, DecisionType, FieldPredicate, LedgerEntry, LedgerError,
    NewLedgerEntryParams, PublishedDataset, RelationshipType, TriggerEvent,
    GENESIS_PREVIOUS_HASH,
};
use scaffoldry_engine::ManifestEngine;
use scaffoldry_policy::ScaffoldryPolicyEngine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimUser {
    pub id: String,
    pub user_name: String,
    pub name: Value,
    pub active: bool,
    pub emails: Vec<Value>,
    pub roles: Vec<Value>,
    #[serde(rename = "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User")]
    pub enterprise_extension: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimGroup {
    pub id: String,
    pub display_name: String,
    pub members: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceRecord {
    pub id: String,
    pub name: String,
    pub code: String,
    pub organization: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollaboratorRecord {
    pub id: String,
    pub workspace_id: String,
    pub eppn: String,
    pub role: String,
    pub scoped_affiliation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetRecord {
    pub id: String,
    pub app_slug: String,
    pub data: Value,
    pub ceds_mapping: Value,
    pub is_ferpa_sensitive: bool,
    pub created_at: String,
}

pub struct ServerState {
    pub users: RwLock<HashMap<String, ScimUser>>,
    pub groups: RwLock<HashMap<String, ScimGroup>>,
    pub workspaces: RwLock<HashMap<String, WorkspaceRecord>>,
    pub collaborators: RwLock<HashMap<String, Vec<CollaboratorRecord>>>,
    pub records: RwLock<HashMap<String, Vec<DatasetRecord>>>,
    pub datasets: RwLock<HashMap<String, PublishedDataset>>,
    pub relationships: RwLock<HashMap<String, DatasetRelationship>>,
    pub automations: RwLock<HashMap<String, Vec<AutomationRule>>>,
    pub ledger: RwLock<Vec<LedgerEntry>>,
    pub engine: RwLock<ManifestEngine>,
    pub policy_engine: ScaffoldryPolicyEngine,
}

impl ServerState {
    pub fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let engine = ManifestEngine::new()
            .map_err(|e| format!("Failed to initialize ManifestEngine: {e}"))?;
        let policy_engine = ScaffoldryPolicyEngine::default_institutional_engine()
            .map_err(|e| format!("Failed to initialize PolicyEngine: {e}"))?;

        let mut datasets = HashMap::new();
        let mut relationships = HashMap::new();

        let now = Utc::now().to_rfc3339();

        // 1. Faculty Roster & PI Directory
        datasets.insert(
            "faculty".to_string(),
            PublishedDataset {
                id: "faculty".to_string(),
                name: "Faculty & Principal Investigator Directory".to_string(),
                description: "Institutional faculty roster, appointments, and research affiliations".to_string(),
                department: "Academic Affairs".to_string(),
                organization: "University".to_string(),
                sensitivity_level: "Directory".to_string(),
                herm_capability_id: Some("HR-01-ROSTER".to_string()),
                fields: vec![
                    DatasetField { name: "eppn".to_string(), label: "Identity (ePPN)".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: Some("000033".to_string()) },
                    DatasetField { name: "full_name".to_string(), label: "Full Name".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: Some("000115".to_string()) },
                    DatasetField { name: "title".to_string(), label: "Academic Title".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                    DatasetField { name: "department".to_string(), label: "Department".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                    DatasetField { name: "research_specialty".to_string(), label: "Research Specialty".to_string(), field_type: "Text".to_string(), required: false, ferpa_sensitive: false, ceds_code: None },
                ],
                record_count: 240,
                published_at: now.clone(),
                sample_data: vec![
                    json!({ "eppn": "dr.smith@university.edu", "full_name": "Dr. Sarah Smith", "title": "Professor of Physics", "department": "Physics", "research_specialty": "Quantum Lattice Systems" }),
                    json!({ "eppn": "dr.alan@university.edu", "full_name": "Dr. Alan Turing", "title": "Chair of Computing", "department": "Computer Science", "research_specialty": "Automata & Cryptography" }),
                ],
            },
        );

        // 2. Academic Programs & Degrees
        datasets.insert(
            "programs".to_string(),
            PublishedDataset {
                id: "programs".to_string(),
                name: "Academic Programs & Degrees".to_string(),
                description: "Accredited degree programs, CIP taxonomy, and departmental authority".to_string(),
                department: "Provost & Academic Council".to_string(),
                organization: "University".to_string(),
                sensitivity_level: "Public".to_string(),
                herm_capability_id: Some("ACA-02-CURRICULUM".to_string()),
                fields: vec![
                    DatasetField { name: "code".to_string(), label: "Program Code".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: Some("000067".to_string()) },
                    DatasetField { name: "degree_name".to_string(), label: "Degree Name".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                    DatasetField { name: "department".to_string(), label: "Governing Department".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                ],
                record_count: 58,
                published_at: now.clone(),
                sample_data: vec![
                    json!({ "code": "PHYS-PHD", "degree_name": "Doctor of Philosophy in Physics", "department": "Physics" }),
                    json!({ "code": "CS-BS", "degree_name": "Bachelor of Science in Computer Science", "department": "Computer Science" }),
                ],
            },
        );

        // 3. University Course Catalog
        datasets.insert(
            "courses".to_string(),
            PublishedDataset {
                id: "courses".to_string(),
                name: "University Course Catalog".to_string(),
                description: "Active course roster, schedule units, and designated faculty instructors".to_string(),
                department: "Office of the Registrar".to_string(),
                organization: "University".to_string(),
                sensitivity_level: "Directory".to_string(),
                herm_capability_id: Some("REG-01-CATALOG".to_string()),
                fields: vec![
                    DatasetField { name: "course_code".to_string(), label: "Course Code".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: Some("000062".to_string()) },
                    DatasetField { name: "course_title".to_string(), label: "Course Title".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: Some("000064".to_string()) },
                    DatasetField { name: "credits".to_string(), label: "Credit Units".to_string(), field_type: "Number".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                    DatasetField { name: "instructor_eppn".to_string(), label: "Lead Instructor".to_string(), field_type: "Relation".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                    DatasetField { name: "program_code".to_string(), label: "Program Plan".to_string(), field_type: "Relation".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                ],
                record_count: 840,
                published_at: now.clone(),
                sample_data: vec![
                    json!({ "course_code": "PHYS-401", "course_title": "Quantum Mechanics I", "credits": 4, "instructor_eppn": "dr.smith@university.edu", "program_code": "PHYS-PHD" }),
                    json!({ "course_code": "CS-302", "course_title": "Theory of Computation", "credits": 3, "instructor_eppn": "dr.alan@university.edu", "program_code": "CS-BS" }),
                ],
            },
        );

        // 4. Sponsored Research Grants
        datasets.insert(
            "grants".to_string(),
            PublishedDataset {
                id: "grants".to_string(),
                name: "Sponsored Research Projects & Grants".to_string(),
                description: "Federal and private grant awards with FERPA-sensitive student stipends".to_string(),
                department: "Office of Sponsored Research".to_string(),
                organization: "University".to_string(),
                sensitivity_level: "Restricted / FERPA".to_string(),
                herm_capability_id: Some("RES-01-GRANTS".to_string()),
                fields: vec![
                    DatasetField { name: "award_number".to_string(), label: "Award Identifier".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                    DatasetField { name: "project_title".to_string(), label: "Project Title".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                    DatasetField { name: "pi_eppn".to_string(), label: "Principal Investigator".to_string(), field_type: "Relation".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                    DatasetField { name: "amount".to_string(), label: "Total Award Amount".to_string(), field_type: "Number".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                    DatasetField { name: "is_ferpa_restricted".to_string(), label: "Contains Student Assistant Data".to_string(), field_type: "Boolean".to_string(), required: false, ferpa_sensitive: true, ceds_code: None },
                ],
                record_count: 115,
                published_at: now.clone(),
                sample_data: vec![
                    json!({ "award_number": "NSF-PHY-2026-01", "project_title": "Quantum Lattice Topological Phases", "pi_eppn": "dr.smith@university.edu", "amount": 750000, "is_ferpa_restricted": true }),
                ],
            },
        );

        // Seed Relationships
        relationships.insert(
            "rel_course_instructor".to_string(),
            DatasetRelationship {
                id: "rel_course_instructor".to_string(),
                name: "Course Instructor Lookup".to_string(),
                source_dataset_id: "courses".to_string(),
                target_dataset_id: "faculty".to_string(),
                source_field: "instructor_eppn".to_string(),
                target_field: "eppn".to_string(),
                relationship_type: RelationshipType::OneToMany,
                display_field: "full_name".to_string(),
            },
        );

        relationships.insert(
            "rel_course_program".to_string(),
            DatasetRelationship {
                id: "rel_course_program".to_string(),
                name: "Course Degree Plan".to_string(),
                source_dataset_id: "courses".to_string(),
                target_dataset_id: "programs".to_string(),
                source_field: "program_code".to_string(),
                target_field: "code".to_string(),
                relationship_type: RelationshipType::OneToMany,
                display_field: "degree_name".to_string(),
            },
        );

        relationships.insert(
            "rel_grant_pi".to_string(),
            DatasetRelationship {
                id: "rel_grant_pi".to_string(),
                name: "Grant Principal Investigator".to_string(),
                source_dataset_id: "grants".to_string(),
                target_dataset_id: "faculty".to_string(),
                source_field: "pi_eppn".to_string(),
                target_field: "eppn".to_string(),
                relationship_type: RelationshipType::OneToMany,
                display_field: "full_name".to_string(),
            },
        );

        let mut automations = HashMap::new();
        automations.insert(
            "physics-admissions-review".to_string(),
            vec![AutomationRule {
                id: "rule-admissions-auto-approve".to_string(),
                app_slug: "physics-admissions-review".to_string(),
                name: "Honors Fellowship Notification".to_string(),
                description: "Notifies dean and records ledger entry when candidate GPA exceeds 3.85".to_string(),
                enabled: true,
                trigger: TriggerEvent::StatusChanged { to_status: "Approved".to_string() },
                cedar_policy_guard: Some("policy-ferpa-34cfr99".to_string()),
                predicates: vec![FieldPredicate {
                    field_name: "gpa".to_string(),
                    operator: ConditionOperator::GreaterThan,
                    expected_value: "3.85".to_string(),
                }],
                actions: vec![
                    ActionType::NotifyCollaborator {
                        role: "dean".to_string(),
                        message_template: "Candidate approved for fellowship funding".to_string(),
                    },
                    ActionType::CreateLedgerAuditEntry {
                        summary: "Automated fellowship approval recorded".to_string(),
                        oscal_control: "AC-03".to_string(),
                    },
                ],
            }],
        );

        let entry_0 = LedgerEntry::new(NewLedgerEntryParams {
            sequence: 0,
            timestamp_iso: now.clone(),
            previous_hash: GENESIS_PREVIOUS_HASH.to_string(),
            principal: "prof.curie@science.state.edu".to_string(),
            organization_code: "DIV-SCIENCES".to_string(),
            app_slug: Some("biology-lab-inventory".to_string()),
            decision_type: DecisionType::AppPublished,
            oscal_control_id: "CM-03".to_string(),
            rationale: "Initial publication of Biology Research Chemical Inventory".to_string(),
            payload: &json!({"status": "Published", "domain": "inventory.biology.state.edu"}),
        });

        let entry_1 = LedgerEntry::new(NewLedgerEntryParams {
            sequence: 1,
            timestamp_iso: now.clone(),
            previous_hash: entry_0.entry_hash.clone(),
            principal: "prof.curie@science.state.edu".to_string(),
            organization_code: "DIV-SCIENCES".to_string(),
            app_slug: Some("biology-lab-inventory".to_string()),
            decision_type: DecisionType::VanityDnsBound,
            oscal_control_id: "SC-07".to_string(),
            rationale: "Vanity DNS alias bound with sovereign gateway validation".to_string(),
            payload: &json!({"domain": "inventory.biology.state.edu", "verified": true}),
        });

        let entry_2 = LedgerEntry::new(NewLedgerEntryParams {
            sequence: 2,
            timestamp_iso: now.clone(),
            previous_hash: entry_1.entry_hash.clone(),
            principal: "dr.watson@science.state.edu".to_string(),
            organization_code: "DIV-COMPLIANCE".to_string(),
            app_slug: Some("physics-admissions-review".to_string()),
            decision_type: DecisionType::WorkflowRuleApproved,
            oscal_control_id: "AC-03".to_string(),
            rationale: "FERPA compliance and GPA threshold auto-admit rule approved".to_string(),
            payload: &json!({"rule": "auto-physics-honors-admit", "policy": "policy-ferpa-34cfr99"}),
        });

        let entry_3 = LedgerEntry::new(NewLedgerEntryParams {
            sequence: 3,
            timestamp_iso: now,
            previous_hash: entry_2.entry_hash.clone(),
            principal: "dr.watson@science.state.edu".to_string(),
            organization_code: "DIV-COMPLIANCE".to_string(),
            app_slug: Some("compliance-ferpa-requests".to_string()),
            decision_type: DecisionType::StatutoryAttestation,
            oscal_control_id: "AU-02".to_string(),
            rationale: "Institutional statutory compliance attestation for 34 CFR Part 99".to_string(),
            payload: &json!({"framework": "FERPA", "standard": "34 CFR Part 99"}),
        });

        let ledger = vec![entry_0, entry_1, entry_2, entry_3];

        Ok(Self {
            users: RwLock::new(HashMap::new()),
            groups: RwLock::new(HashMap::new()),
            workspaces: RwLock::new(HashMap::new()),
            collaborators: RwLock::new(HashMap::new()),
            records: RwLock::new(HashMap::new()),
            datasets: RwLock::new(datasets),
            relationships: RwLock::new(relationships),
            automations: RwLock::new(automations),
            ledger: RwLock::new(ledger),
            engine: RwLock::new(engine),
            policy_engine,
        })
    }
}

#[derive(Debug, Clone)]
pub struct RecordDecisionInput<'a> {
    pub principal: String,
    pub organization_code: String,
    pub app_slug: Option<String>,
    pub decision_type: DecisionType,
    pub oscal_control_id: String,
    pub rationale: String,
    pub payload: &'a Value,
}

impl ServerState {
    pub fn append_ledger_entry(
        &self,
        input: RecordDecisionInput<'_>,
    ) -> Result<LedgerEntry, LedgerError> {
        let mut ledger = self.ledger.write().unwrap();
        let sequence = ledger.len() as u64;
        let previous_hash = if sequence == 0 {
            GENESIS_PREVIOUS_HASH.to_string()
        } else {
            ledger.last().unwrap().entry_hash.clone()
        };
        let now = chrono::Utc::now().to_rfc3339();
        let entry = LedgerEntry::new(NewLedgerEntryParams {
            sequence,
            timestamp_iso: now,
            previous_hash,
            principal: input.principal,
            organization_code: input.organization_code,
            app_slug: input.app_slug,
            decision_type: input.decision_type,
            oscal_control_id: input.oscal_control_id,
            rationale: input.rationale,
            payload: input.payload,
        });
        ledger.push(entry.clone());
        Ok(entry)
    }

    pub fn verify_ledger(&self) -> Result<bool, LedgerError> {
        let ledger = self.ledger.read().unwrap();
        verify_ledger_chain(&ledger)
    }

    pub fn export_oscal_component_definition(&self) -> Value {
        let ledger = self.ledger.read().unwrap();
        let now = chrono::Utc::now().to_rfc3339();

        let implemented_requirements: Vec<Value> = ledger
            .iter()
            .map(|entry| {
                json!({
                    "uuid": uuid::Uuid::new_v4().to_string(),
                    "control-id": entry.oscal_control_id.to_lowercase(),
                    "description": entry.rationale.clone(),
                    "props": [
                        {
                            "name": "ledger-sequence",
                            "value": entry.sequence.to_string()
                        },
                        {
                            "name": "ledger-principal",
                            "value": entry.principal.clone()
                        },
                        {
                            "name": "ledger-org-code",
                            "value": entry.organization_code.clone()
                        },
                        {
                            "name": "ledger-entry-hash",
                            "value": entry.entry_hash.clone()
                        },
                        {
                            "name": "ledger-previous-hash",
                            "value": entry.previous_hash.clone()
                        }
                    ]
                })
            })
            .collect();

        json!({
            "component-definition": {
                "uuid": uuid::Uuid::new_v4().to_string(),
                "metadata": {
                    "title": "Scaffoldry Sovereign Application Platform Component Definition",
                    "last-modified": now,
                    "version": "1.0.0",
                    "oscal-version": "1.1.2"
                },
                "components": [
                    {
                        "uuid": uuid::Uuid::new_v4().to_string(),
                        "type": "software",
                        "title": "Scaffoldry Sovereign Application Platform",
                        "description": "Governed collaborative workspace, tabular engine, and Cedar authorization lattice",
                        "control-implementations": [
                            {
                                "uuid": uuid::Uuid::new_v4().to_string(),
                                "source": "https://doi.org/10.6028/NIST.SP.800-53r5",
                                "description": "Automated institutional control implementation and cryptographic verification ledger",
                                "implemented-requirements": implemented_requirements
                            }
                        ]
                    }
                ]
            }
        })
    }
}

pub type SharedState = Arc<ServerState>;
