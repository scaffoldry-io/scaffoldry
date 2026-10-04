export type ViewType = "Table" | "Form" | "Dashboard" | "Detail";
export type FieldType = "Text" | "Number" | "Date" | "Select" | "Boolean";

export interface FieldSpec {
  name: string;
  label: string;
  field_type: FieldType;
  required: boolean;
  ferpa_sensitive: boolean;
}

export interface AppView {
  id: string;
  title: string;
  view_type: ViewType;
  fields: FieldSpec[];
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
  ceds_mappings: Record<string, string>;
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
  status: "Published" | "Staging" | "Draft";
  updatedAt: string;
  recordsCount: number;
}
