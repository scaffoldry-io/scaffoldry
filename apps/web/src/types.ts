export type ViewType = "Table" | "Form" | "Dashboard" | "Detail";
export type FieldType = "Text" | "Number" | "Date" | "Select" | "Boolean" | "Relation";

export interface FieldSpec {
  name: string;
  label: string;
  field_type: FieldType;
  required: boolean;
  ferpa_sensitive: boolean;
  linked_dataset_id?: string;
  linked_field?: string;
}

export interface AppView {
  id: string;
  title: string;
  view_type: ViewType;
  fields: FieldSpec[];
}

export interface Collaborator {
  eppn: string;
  name: string;
  role: "owner" | "editor" | "viewer";
  department: string;
}

export type GovernedComponentType =
  | "stat-metric"
  | "tabular-grid"
  | "kanban-stage"
  | "intake-form"
  | "calendar-view"
  | "rich-banner"
  | "action-toolbar";

export interface GovernedComponentSpec {
  id: string;
  type: GovernedComponentType;
  title: string;
  slot: "header" | "main" | "sidebar" | "footer";
  layout: {
    width: "full" | "half" | "third" | "two-thirds";
    order: number;
  };
  access_guard?: {
    required_affiliation?: "faculty" | "staff" | "student" | "admin";
    classification_max?: "public" | "internal" | "restricted";
  };
  config: Record<string, any>;
}

export interface AppPage {
  id: string;
  slug: string;
  title: string;
  icon: string;
  description?: string;
  components: GovernedComponentSpec[];
}

export interface AppManifest {
  slug: string;
  title: string;
  description: string;
  organization_code: string;
  department: string;
  herm_capability_id?: string;
  custom_domain?: string;
  custom_domain_verified: boolean;
  views: AppView[];
  pages?: AppPage[];
  ceds_mappings: Record<string, string>;
  collaborators?: Collaborator[];
  status?: "Published" | "Collaborating" | "Draft";
  workspace_id?: string;
}

export interface Workspace {
  id: string;
  name: string;
  department: string;
  description: string;
  icon: string;
  lead: string;
  appCount: number;
}

export interface SourceRule {
  id: string;
  source: string;
  title: string;
  oscalControl: string;
  cedarPolicyId: string;
  cedarSnippet: string;
  targetSensitivity: string;
  status: "Enforced" | "Audit-Only";
  departmentScope: string;
}

export interface Persona {
  eppn: string;
  name: string;
  affiliation: "faculty" | "student" | "staff";
  department: string;
  roleTitle: string;
  isAdmin: boolean;
}

export interface RegisteredApp {
  slug: string;
  title: string;
  orgCode: string;
  department: string;
  customDomain: string;
  verified: boolean;
  hermCapability: string;
  cedsDomain: string;
  status: "Published" | "Collaborating" | "Draft";
  updatedAt: string;
  recordsCount: number;
  workspaceId: string;
  collaborators: Collaborator[];
  manifest: AppManifest;
}

export type RelationshipType = "OneToOne" | "OneToMany" | "ManyToMany";

export interface DatasetRelationship {
  id: string;
  name: string;
  source_dataset_id: string;
  target_dataset_id: string;
  source_field: string;
  target_field: string;
  relationship_type: RelationshipType;
  display_field: string;
}

export interface DatasetField {
  name: string;
  label: string;
  field_type: string;
  required: boolean;
  ferpa_sensitive: boolean;
  ceds_code?: string;
}

export interface PublishedDataset {
  id: string;
  name: string;
  description: string;
  department: string;
  organization: string;
  sensitivity_level: string;
  herm_capability_id?: string;
  fields: DatasetField[];
  record_count: number;
  published_at: string;
  relationships?: DatasetRelationship[];
  sample_data?: Record<string, any>[];
}

export interface McpTool {
  name: string;
  description: string;
  inputSchema: Record<string, any>;
}

export interface McpResource {
  uri: string;
  name: string;
  description: string;
  mimeType: string;
}

export interface McpPrompt {
  name: string;
  description: string;
  arguments?: { name: string; description: string; required: boolean }[];
}

export interface McpOverview {
  protocol: string;
  protocol_version: string;
  server_info: {
    name: string;
    version: string;
    description: string;
  };
  capabilities: {
    tools: { count: number; items: string[] };
    resources: { count: number; uris: string[] };
    prompts: { count: number; items: string[] };
  };
  live_metrics: {
    published_datasets: number;
    dataset_relationships: number;
    compliance_frameworks: string[];
  };
}

export interface WorkflowPredicate {
  field_name: string;
  operator: "Equals" | "NotEquals" | "GreaterThan" | "LessThan" | "Contains";
  expected_value: string;
}

export type WorkflowTriggerEvent =
  | { type: "RecordCreated" }
  | { type: "RecordUpdated" }
  | { type: "FieldChanged"; field_name: string }
  | { type: "StatusChanged"; to_status: string };

export type WorkflowActionItem =
  | { type: "NotifyCollaborator"; role: string; message_template: string }
  | { type: "UpdateRecordStatus"; new_status: string }
  | { type: "CreateLedgerAuditEntry"; summary: string; oscal_control: string }
  | { type: "WebhookDispatch"; target_url: string };

export interface WorkflowAutomationRule {
  id: string;
  app_slug: string;
  name: string;
  description: string;
  enabled: boolean;
  trigger: WorkflowTriggerEvent;
  cedar_policy_guard?: string;
  predicates: WorkflowPredicate[];
  actions: WorkflowActionItem[];
}

export interface LedgerEntryItem {
  sequence: number;
  timestamp_iso: string;
  previous_hash: string;
  principal: string;
  organization_code: string;
  app_slug?: string;
  decision_type: string;
  oscal_control_id: string;
  rationale: string;
  payload_hash: string;
  entry_hash: string;
}


