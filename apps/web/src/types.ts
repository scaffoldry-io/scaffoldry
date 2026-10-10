export type ViewType =
  | "Grid"
  | "Kanban"
  | "Calendar"
  | "Gallery"
  | "Table"
  | "Form"
  | "Dashboard"
  | "Detail";

export type FilterOperator =
  | "equals"
  | "not_equals"
  | "contains"
  | "not_contains"
  | "greater_than"
  | "less_than"
  | "is_empty"
  | "is_not_empty";

export type FilterConjunction = "AND" | "OR";

export interface FilterClause {
  id: string;
  field_name: string;
  operator: FilterOperator;
  value: string;
}

export interface CompoundFilter {
  conjunction: FilterConjunction;
  clauses: FilterClause[];
}

export type SortDirection = "asc" | "desc";

export interface SortRule {
  id: string;
  field_name: string;
  direction: SortDirection;
}

export type RowDensity = "compact" | "medium" | "tall" | "extra_tall";
export type FieldType =
  | "Text"
  | "Number"
  | "Date"
  | "Select"
  | "Boolean"
  | "Relation"
  | "Checkbox"
  | "MultiSelect"
  | "Currency"
  | "Percent"
  | "Rating"
  | "Email"
  | "Phone"
  | "Url"
  | "Autonumber"
  | "CreatedTime"
  | "LastModifiedTime"
  | "Lookup"
  | "Count"
  | "Rollup"
  | "Formula";

export type LinkCardinality = "single" | "multiple" | "one-to-one" | "one-to-many" | "many-to-many";

export interface LinkFilter {
  field: string;
  operator: "equals" | "not_equals" | "contains" | "greater_than" | "less_than";
  value: string;
}

export interface FieldSpec {
  name: string;
  label: string;
  field_type: FieldType;
  required: boolean;
  ferpa_sensitive: boolean;
  linked_dataset_id?: string;
  linked_field?: string;
  target_table_id?: string;
  target_display_field?: string;
  display_label_override?: string;
  cardinality?: LinkCardinality;
  allow_multiple?: boolean;
  link_filter?: LinkFilter;
  formula_expression?: string;
  rollup_function?: "sum" | "avg" | "min" | "max" | "count" | "concat";
  select_options?: string[];
  currency_symbol?: string;
  precision?: number;
}

export interface AppView {
  id: string;
  table_id?: string;
  title: string;
  view_type: ViewType;
  fields?: FieldSpec[];
  filters?: CompoundFilter;
  sort_rules?: SortRule[];
  group_by_field?: string;
  row_density?: RowDensity;
  kanban_column_field?: string;
  calendar_date_field?: string;
  column_order?: string[];
  column_widths?: [string, number][];
  hidden_columns?: string[];
  frozen_through?: string;
  column_summary?: [string, string][];
}

export interface Collaborator {
  id?: string;
  eppn: string;
  name: string;
  role: "owner" | "admin" | "editor" | "viewer";
  department: string;
  scoped_affiliation?: string;
  added_at?: string;
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

export interface TableRelationship {
  id: string;
  name: string;
  source_table_id: string;
  target_table_id: string;
  source_field: string;
  target_field: string;
  relationship_type: "ManyToOne" | "OneToMany" | "ManyToMany";
  display_field: string;
}

export interface AppTable {
  id: string;
  name: string;
  slug: string;
  description?: string;
  icon?: string;
  primary_field?: string;
  fields: FieldSpec[];
  records?: Record<string, any>[];
  relationships?: TableRelationship[];
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
  tables?: AppTable[];
  relationships?: TableRelationship[];
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
  code?: string;
  organization?: string;
  department: string;
  description: string;
  icon: string;
  lead: string;
  appCount: number;
  visibility: "restricted" | "departmental" | "institutional";
  allowed_affiliations?: string[];
  data_classification?: string;
  collaborators: Collaborator[];
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
  affiliation: "faculty" | "student" | "staff" | "compliance" | "central_admin";
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

export type ProcessStepKind =
  | {
      Service: {
        action:
          | { NotifyCollaborator: { role: string; message_template: string } }
          | { UpdateRecordStatus: { new_status: string } }
          | { CreateLedgerAuditEntry: { summary: string; oscal_control: string } }
          | { WebhookDispatch: { target_url: string } };
      };
    }
  | {
      UserTask: {
        role: string;
        prompt: string;
        approve: any[];
        reject: any[];
      };
    };

export interface ProcessStepItem {
  id: string;
  when: WorkflowPredicate[];
  kind: ProcessStepKind;
}

export interface WorkflowAutomationRule {
  id: string;
  app_slug: string;
  name: string;
  description: string;
  enabled: boolean;
  trigger: WorkflowTriggerEvent;
  predicates: WorkflowPredicate[];
  actions: WorkflowActionItem[];
  steps?: ProcessStepItem[];
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
  payload?: Record<string, any>;
  payload_hash: string;
  entry_hash: string;
}



export interface OrganizationNode {
  id: string;
  parent_id: string | null;
  name: string;
  code: string;
  org_type: string;
}

export interface OrgRole {
  id: string;
  person_id: string;
  eppn: string;
  organization_id: string;
  role_title: string;
  scoped_affiliation: string;
  is_primary: boolean;
  source: string;
}
