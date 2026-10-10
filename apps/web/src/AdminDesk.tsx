import { UserSettingsModal } from "./UserSettingsModal";
import React, { useState, useEffect, useRef, Suspense, lazy, useMemo } from "react";
import { DatasetExplorer } from "./DatasetExplorer";
import { MultiViewWorkspace } from "./MultiViewWorkspace";
import { CoBuilderStudioModal } from "./CoBuilderStudioModal";
import { WorkspaceSettingsModal } from "./WorkspaceSettingsModal";
import { apiClient, getAuthToken, setAuthToken } from "./api";
import { AppManifest, Collaborator, FieldSpec, Persona, PublishedDataset, RegisteredApp, SourceRule, Workspace, WorkflowAutomationRule, LedgerEntryItem, OrganizationNode } from "./types";

const AppBuilder = lazy(() => import("./AppBuilder").then((m) => ({ default: m.AppBuilder })));
const PublishedAppView = lazy(() => import("./PublishedAppView").then((m) => ({ default: m.PublishedAppView })));
const StandaloneIntakeForm = lazy(() => import("./StandaloneIntakeForm").then((m) => ({ default: m.StandaloneIntakeForm })));
import { AdminConsole } from "./admin/AdminConsole";
import { ProcessDesk } from "./ProcessDesk";

const PERSONAS: Persona[] = [
  {
    eppn: "sarah.connor@state.edu",
    name: "Dr. Sarah Connor",
    affiliation: "faculty",
    department: "Computer Science",
    roleTitle: "Department Chair & Professor",
    isAdmin: false,
  },
  {
    eppn: "marcus.vance@state.edu",
    name: "Marcus Vance",
    affiliation: "staff",
    department: "Office of Sponsored Programs",
    roleTitle: "Senior Research Administrator",
    isAdmin: false,
  },
  {
    eppn: "elena.rodriguez@state.edu",
    name: "Elena Rodriguez",
    affiliation: "compliance",
    department: "Institutional Review Board",
    roleTitle: "IRB & Research Compliance Analyst",
    isAdmin: false,
  },
  {
    eppn: "jordan.lee@state.edu",
    name: "Jordan Lee",
    affiliation: "central_admin",
    department: "Central Enterprise IT",
    roleTitle: "Enterprise Identity & Security Architect",
    isAdmin: true,
  },
  {
    eppn: "prof.curie@science.state.edu",
    name: "Dr. Marie Curie",
    affiliation: "faculty",
    department: "biology",
    roleTitle: "Professor & Lab Director",
    isAdmin: false,
  },
  {
    eppn: "student.smith@science.state.edu",
    name: "Alex Smith",
    affiliation: "student",
    department: "biology",
    roleTitle: "Graduate Research Assistant",
    isAdmin: false,
  },
  {
    eppn: "dr.watson@science.state.edu",
    name: "Dr. Arthur Watson",
    affiliation: "staff",
    department: "compliance",
    roleTitle: "Campus FERPA & Export Officer",
    isAdmin: true,
  },
  {
    eppn: "einstein@physics.state.edu",
    name: "Albert Einstein",
    affiliation: "student",
    department: "physics",
    roleTitle: "Physics Research Fellow",
    isAdmin: false,
  },
];

export const getUserWorkspaceRole = (
  ws: Workspace,
  persona: Persona
): "owner" | "admin" | "editor" | "viewer" | null => {
  const member = ws.collaborators?.find((c) => c.eppn === persona.eppn);
  if (member) return member.role;
  if (persona.affiliation === "central_admin") return "admin";
  return null;
};

export const canUserManageWorkspace = (ws: Workspace, persona: Persona): boolean => {
  if (persona.affiliation === "central_admin") return true;
  const role = getUserWorkspaceRole(ws, persona);
  return role === "owner" || role === "admin";
};

export const canUserAccessApp = (
  app: RegisteredApp,
  persona: Persona,
  allWorkspaces: Workspace[]
): boolean => {
  // 1. Central Admin has universal supervisory access across all institutional resources
  if (persona.affiliation === "central_admin") return true;

  // 2. Direct collaborator on the app is granted access
  if (app.collaborators?.some((c) => c.eppn === persona.eppn)) return true;

  // 3. Parent workspace security rules & Cedar ABAC sharing boundary
  const ws =
    allWorkspaces.find((w) => w.id === app.workspaceId) ||
    INITIAL_WORKSPACES.find((w) => w.id === app.workspaceId);

  if (!ws) {
    // If no parent workspace, fallback to departmental boundary check
    return app.department.toLowerCase() === persona.department.toLowerCase();
  }

  // A. Restricted Workspace Boundary
  if (ws.visibility === "restricted") {
    const isMember = ws.collaborators?.some((c) => c.eppn === persona.eppn);
    if (!isMember) return false;
  }

  // B. Departmental Realm Isolation
  if (ws.visibility === "departmental") {
    const isMember = ws.collaborators?.some((c) => c.eppn === persona.eppn);
    const matchesDept = ws.department.toLowerCase() === persona.department.toLowerCase();
    if (!isMember && !matchesDept) return false;
  }

  // C. Allowed Affiliations Gate
  if (ws.allowed_affiliations && ws.allowed_affiliations.length > 0) {
    if (!ws.allowed_affiliations.includes(persona.affiliation)) {
      const isMember = ws.collaborators?.some((c) => c.eppn === persona.eppn);
      if (!isMember) return false;
    }
  }

  return true;
};

export const INITIAL_WORKSPACES: Workspace[] = [
  {
    id: "ws-bio-lab",
    name: "Biology Research Laboratory",
    code: "BIO",
    organization: "College of Sciences",
    department: "biology",
    description: "Collaborative research protocols, instrumentation registers, and specimen data manifests.",
    icon: "🔬",
    lead: "Dr. Marie Curie",
    appCount: 2,
    visibility: "restricted",
    allowed_affiliations: ["faculty", "staff", "student"],
    data_classification: "Level 4 Restricted",
    cedar_policy_guard: 'forbid (principal, action == Action::"access_workspace", resource) when { resource.visibility == "restricted" && resource.is_member == false };',
    collaborators: [
      { id: "collab-bio-1", eppn: "prof.curie@science.state.edu", name: "Dr. Marie Curie", role: "owner", department: "biology", scoped_affiliation: "faculty" },
      { id: "collab-bio-2", eppn: "student.smith@science.state.edu", name: "Alex Smith", role: "editor", department: "biology", scoped_affiliation: "student" },
      { id: "collab-bio-3", eppn: "marcus.vance@state.edu", name: "Marcus Vance", role: "viewer", department: "Office of Sponsored Programs", scoped_affiliation: "staff" },
    ],
  },
  {
    id: "ws-physics-optics",
    name: "Physics & Quantum Optics",
    code: "PHYS",
    organization: "College of Sciences",
    department: "physics",
    description: "High-energy laser logs, quantum optics sensor arrays, and space utilization manifests.",
    icon: "⚡",
    lead: "Albert Einstein",
    appCount: 1,
    visibility: "restricted",
    allowed_affiliations: ["faculty", "student"],
    data_classification: "Level 3 Internal",
    cedar_policy_guard: 'permit (principal, action, resource) when { resource.is_member || principal.department == resource.department || principal.scoped_affiliation == "central_admin" };',
    collaborators: [
      { id: "collab-phys-1", eppn: "einstein@physics.state.edu", name: "Albert Einstein", role: "owner", department: "physics", scoped_affiliation: "student" },
    ],
  },
  {
    id: "ws-campus-compliance",
    name: "Campus Compliance & Privacy",
    code: "COMPLIANCE",
    organization: "Office of the General Counsel",
    department: "compliance",
    description: "Institutional FERPA disclosure registers, export control logs, and statutory audit records.",
    icon: "🛡️",
    lead: "Dr. Arthur Watson",
    appCount: 1,
    visibility: "departmental",
    allowed_affiliations: ["staff", "compliance"],
    data_classification: "Level 4 Restricted",
    cedar_policy_guard: 'permit (principal, action, resource) when { resource.is_member || principal.department == resource.department || principal.scoped_affiliation == "central_admin" };',
    collaborators: [
      { id: "collab-comp-1", eppn: "dr.watson@science.state.edu", name: "Dr. Arthur Watson", role: "owner", department: "compliance", scoped_affiliation: "staff" },
      { id: "collab-comp-2", eppn: "elena.rodriguez@state.edu", name: "Elena Rodriguez", role: "admin", department: "Institutional Review Board", scoped_affiliation: "compliance" },
    ],
  },
  {
    id: "ws-cs-research",
    name: "Computer Science & Systems Lab",
    code: "CS",
    organization: "College of Engineering",
    department: "Computer Science",
    description: "Distributed systems, sovereign agent computing, and verifiable lattice architectures.",
    icon: "💻",
    lead: "Dr. Sarah Connor",
    appCount: 1,
    visibility: "restricted",
    allowed_affiliations: ["faculty", "staff"],
    data_classification: "Level 2 Campus-Wide",
    cedar_policy_guard: 'permit (principal, action, resource) when { resource.is_member || principal.scoped_affiliation == "central_admin" };',
    collaborators: [
      { id: "collab-cs-1", eppn: "sarah.connor@state.edu", name: "Dr. Sarah Connor", role: "owner", department: "Computer Science", scoped_affiliation: "faculty" },
    ],
  },
];

export const WORKSPACES = INITIAL_WORKSPACES;

const INITIAL_SOURCE_RULES: SourceRule[] = [
  {
    id: "rule-ferpa-30",
    source: "Federal Law: 34 CFR Part 99 § 99.30",
    title: "FERPA Written Consent Requirement for Educational Records",
    oscalControl: "FERPA-34CFR-99.30 / NIST-800-53-AC-3",
    cedarPolicyId: "ferpa-export-forbid-guard",
    cedarSnippet: 'forbid (principal, action == Action::"export", resource) when { resource.is_ferpa_sensitive && !(principal.scoped_affiliation in ["staff", "compliance"]) };',
    targetSensitivity: "FERPA Sensitive",
    status: "Enforced",
    departmentScope: "Institutional Universal",
  },
  {
    id: "rule-nist-ac3",
    source: "NIST SP 800-53 Rev 5 / CMMC",
    title: "Access Enforcement & Departmental Realm Isolation",
    oscalControl: "NIST-800-53-AC-3 / AC-6",
    cedarPolicyId: "dept-realm-boundary",
    cedarSnippet: 'permit (principal, action in [Action::"read", Action::"write"], resource) when { principal.department == resource.department };',
    targetSensitivity: "Institutional Internal",
    status: "Enforced",
    departmentScope: "Departmental Isolation",
  },
  {
    id: "rule-campus-l4",
    source: "University Data Protection Standard v4.2",
    title: "Level 4 Highly Restricted Research & Student Data",
    oscalControl: "INST-DATA-STD-L4",
    cedarPolicyId: "level4-strict-audit",
    cedarSnippet: 'permit (principal, action == Action::"read", resource is Record) when { principal.scoped_affiliation in ["faculty", "staff", "student"] };',
    targetSensitivity: "Level 4 Restricted",
    status: "Enforced",
    departmentScope: "Research Labs",
  },
];

const INITIAL_APPS: RegisteredApp[] = [
  {
    slug: "bio-lab-inventory",
    title: "Biology Lab Equipment & Bioassay Register",
    orgCode: "DEPT-BIO",
    department: "biology",
    customDomain: "bio-inventory.science.state.edu",
    verified: true,
    hermCapability: "2.2.3 (Research Grant Administration)",
    cedsDomain: "Facility & Equipment (FICM 210)",
    status: "Published",
    updatedAt: "2026-10-04",
    recordsCount: 142,
    workspaceId: "ws-bio-lab",
    collaborators: [
      { eppn: "prof.curie@science.state.edu", name: "Dr. Marie Curie", role: "owner", department: "biology" },
      { eppn: "student.smith@science.state.edu", name: "Alex Smith", role: "editor", department: "biology" },
    ],
    manifest: {
      slug: "bio-lab-inventory",
      title: "Biology Lab Equipment & Bioassay Register",
      description: "Departmental research instrumentation register mapped to NCES CEDS v11.0",
      organization_code: "DEPT-BIO",
      department: "biology",
      herm_capability_id: "2.2.3",
      custom_domain: "bio-inventory.science.state.edu",
      custom_domain_verified: true,
      views: [
        {
          id: "lab-form",
          title: "Instrument Registration Form",
          view_type: "Form",
          fields: [
            { name: "item_name", label: "Equipment Name", field_type: "Text", required: true, ferpa_sensitive: false },
            { name: "serial_number", label: "Serial Number", field_type: "Text", required: true, ferpa_sensitive: false },
            { name: "operator_eval", label: "Student Operator Evaluation", field_type: "Text", required: false, ferpa_sensitive: true },
          ],
        },
      ],
      ceds_mappings: {
        item_name: "000185",
      },
    },
  },
  {
    slug: "bio-travel-grants",
    title: "Departmental Graduate Travel Authorizations",
    orgCode: "DEPT-BIO",
    department: "biology",
    customDomain: "travel.science.state.edu",
    verified: true,
    hermCapability: "2.2.1 (Research Operations)",
    cedsDomain: "PostsecondaryStudent",
    status: "Collaborating",
    updatedAt: "2026-10-02",
    recordsCount: 38,
    workspaceId: "ws-bio-lab",
    collaborators: [
      { eppn: "prof.curie@science.state.edu", name: "Dr. Marie Curie", role: "owner", department: "biology" },
      { eppn: "student.smith@science.state.edu", name: "Alex Smith", role: "editor", department: "biology" },
      { eppn: "dr.watson@science.state.edu", name: "Dr. Arthur Watson", role: "viewer", department: "compliance" },
    ],
    manifest: {
      slug: "bio-travel-grants",
      title: "Departmental Graduate Travel Authorizations",
      description: "Student grant requests and travel allowances with FERPA protections.",
      organization_code: "DEPT-BIO",
      department: "biology",
      herm_capability_id: "2.2.1",
      custom_domain: "travel.science.state.edu",
      custom_domain_verified: true,
      views: [
        {
          id: "travel-form",
          title: "Travel Grant Authorization Request",
          view_type: "Form",
          fields: [
            { name: "applicant_name", label: "Graduate Applicant", field_type: "Text", required: true, ferpa_sensitive: true },
            { name: "conference_title", label: "Conference / Symposium", field_type: "Text", required: true, ferpa_sensitive: false },
            { name: "requested_budget", label: "Requested Budget ($)", field_type: "Number", required: true, ferpa_sensitive: false },
          ],
        },
      ],
      ceds_mappings: {
        applicant_name: "000115",
      },
    },
  },
  {
    slug: "physics-laser-safety",
    title: "High-Energy Optics & Laser Safety Log",
    orgCode: "DEPT-PHYSICS",
    department: "physics",
    customDomain: "lasers.physics.state.edu",
    verified: true,
    hermCapability: "4.1.2 (Health & Safety Compliance)",
    cedsDomain: "Facility SpaceUtilization",
    status: "Published",
    updatedAt: "2026-09-28",
    recordsCount: 89,
    workspaceId: "ws-physics-optics",
    collaborators: [
      { eppn: "einstein@physics.state.edu", name: "Albert Einstein", role: "owner", department: "physics" },
    ],
    manifest: {
      slug: "physics-laser-safety",
      title: "High-Energy Optics & Laser Safety Log",
      description: "Safety interlock verifications and laser operator logs.",
      organization_code: "DEPT-PHYSICS",
      department: "physics",
      herm_capability_id: "4.1.2",
      custom_domain: "lasers.physics.state.edu",
      custom_domain_verified: true,
      views: [
        {
          id: "laser-form",
          title: "Laser Safety Checklist",
          view_type: "Form",
          fields: [
            { name: "emitter_id", label: "Laser Emitter Identifier", field_type: "Text", required: true, ferpa_sensitive: false },
            { name: "beam_power_watts", label: "Peak Power (Watts)", field_type: "Number", required: true, ferpa_sensitive: false },
          ],
        },
      ],
      ceds_mappings: {
        emitter_id: "000210",
      },
    },
  },
  {
    slug: "compliance-ferpa-requests",
    title: "Institutional FERPA Disclosure Register",
    orgCode: "DIV-COMPLIANCE",
    department: "compliance",
    customDomain: "ferpa-desk.state.edu",
    verified: true,
    hermCapability: "4.2.1 (Statutory Compliance)",
    cedsDomain: "Governance & Authorization",
    status: "Published",
    updatedAt: "2026-10-04",
    recordsCount: 312,
    workspaceId: "ws-campus-compliance",
    collaborators: [
      { eppn: "dr.watson@science.state.edu", name: "Dr. Arthur Watson", role: "owner", department: "compliance" },
    ],
    manifest: {
      slug: "compliance-ferpa-requests",
      title: "Institutional FERPA Disclosure Register",
      description: "Log of authorized FERPA disclosure requests and parental consents.",
      organization_code: "DIV-COMPLIANCE",
      department: "compliance",
      herm_capability_id: "4.2.1",
      custom_domain: "ferpa-desk.state.edu",
      custom_domain_verified: true,
      views: [
        {
          id: "disclosure-form",
          title: "FERPA Disclosure Request Log",
          view_type: "Form",
          fields: [
            { name: "record_subject", label: "Student EPPN", field_type: "Text", required: true, ferpa_sensitive: true },
            { name: "purpose", label: "Disclosure Purpose", field_type: "Text", required: true, ferpa_sensitive: true },
          ],
        },
      ],
      ceds_mappings: {
        record_subject: "000115",
      },
    },
  },
  {
    slug: "cs-ai-benchmarks",
    title: "Computer Science AI & Distributed Systems Testbed",
    orgCode: "DEPT-CS",
    department: "Computer Science",
    customDomain: "benchmarks.cs.state.edu",
    verified: true,
    hermCapability: "2.1.0 (Academic Operations)",
    cedsDomain: "Research Computing",
    status: "Published",
    updatedAt: "2026-10-05",
    recordsCount: 64,
    workspaceId: "ws-cs-research",
    collaborators: [
      { eppn: "sarah.connor@state.edu", name: "Dr. Sarah Connor", role: "owner", department: "Computer Science" },
    ],
    manifest: {
      slug: "cs-ai-benchmarks",
      title: "Computer Science AI & Distributed Systems Testbed",
      description: "Distributed GPU benchmark datasets and cluster resource allocations.",
      organization_code: "DEPT-CS",
      department: "Computer Science",
      herm_capability_id: "2.1.0",
      custom_domain: "benchmarks.cs.state.edu",
      custom_domain_verified: true,
      views: [
        {
          id: "benchmark-grid",
          title: "Cluster Benchmarks",
          view_type: "Grid",
          fields: [
            { name: "model_id", label: "Model Architecture", field_type: "Text", required: true, ferpa_sensitive: false },
            { name: "latency_ms", label: "Inference Latency (ms)", field_type: "Number", required: true, ferpa_sensitive: false },
          ],
        },
      ],
      ceds_mappings: {},
    },
  },
];

const INITIAL_AUTOMATIONS: Record<string, WorkflowAutomationRule[]> = {
  "physics-admissions-review": [
    {
      id: "auto-physics-honors-admit",
      app_slug: "physics-admissions-review",
      name: "Auto-Admit Notification & Audit for High GPA",
      description: "When applicant GPA is >= 3.85 and status is accepted, dispatch notification and audit to ledger.",
      trigger: { type: "RecordUpdated" },
      predicates: [
        {
          field_name: "gpa",
          operator: "GreaterThan",
          expected_value: "3.84",
        },
      ],
      actions: [
        {
          type: "NotifyCollaborator",
          role: "Physics Department Chair",
          message_template: "High GPA candidate accepted for review",
        },
        {
          type: "CreateLedgerAuditEntry",
          summary: "Automated honors ledger entry per physics faculty criteria",
          oscal_control: "AC-03",
        },
      ],
      enabled: true,
    },
  ],
};

const INITIAL_LEDGER: LedgerEntryItem[] = [
  {
    sequence: 0,
    timestamp_iso: "2026-10-04T12:00:00Z",
    previous_hash: "0000000000000000000000000000000000000000000000000000000000000000",
    principal: "prof.curie@science.state.edu",
    organization_code: "DIV-SCIENCES",
    app_slug: "biology-lab-inventory",
    decision_type: "AppPublished",
    oscal_control_id: "CM-03",
    rationale: "Initial publication of Biology Research Chemical Inventory",
    payload_hash: "9f833a6b22c74d64388b3fdf1b7c3d22b6477e60b134ee7c5980a3c4fcfb7999",
    entry_hash: "c3ab8ff13720e8ad9047dd39466b3c8974e592c2fa383d4a3960714caef0c4f2",
  },
  {
    sequence: 1,
    timestamp_iso: "2026-10-04T12:05:00Z",
    previous_hash: "c3ab8ff13720e8ad9047dd39466b3c8974e592c2fa383d4a3960714caef0c4f2",
    principal: "prof.curie@science.state.edu",
    organization_code: "DIV-SCIENCES",
    app_slug: "biology-lab-inventory",
    decision_type: "VanityDnsBound",
    oscal_control_id: "SC-07",
    rationale: "Vanity DNS alias bound with sovereign gateway validation",
    payload_hash: "5d41402abc4b2a76b9719d911017c592b23a9d91a92e105e6b12a84a27546682",
    entry_hash: "82a7f5a2894b92c431ea014dbd42646f8d839352e0081d6f21271167732a3d0f",
  },
  {
    sequence: 2,
    timestamp_iso: "2026-10-04T12:10:00Z",
    previous_hash: "82a7f5a2894b92c431ea014dbd42646f8d839352e0081d6f21271167732a3d0f",
    principal: "dr.watson@science.state.edu",
    organization_code: "DIV-COMPLIANCE",
    app_slug: "physics-admissions-review",
    decision_type: "WorkflowRuleApproved",
    oscal_control_id: "AC-03",
    rationale: "FERPA compliance and GPA threshold auto-admit rule approved",
    payload_hash: "1bc29b36f623ba82aaf6724fd3b167184451cf2e9336744ab97b664291130f0f",
    entry_hash: "b548b1d9894e63e13d9f0a4fb11894d3ae225501d67f5fa2904746f3640b3c66",
  },
  {
    sequence: 3,
    timestamp_iso: "2026-10-04T12:15:00Z",
    previous_hash: "b548b1d9894e63e13d9f0a4fb11894d3ae225501d67f5fa2904746f3640b3c66",
    principal: "dr.watson@science.state.edu",
    organization_code: "DIV-COMPLIANCE",
    app_slug: "compliance-ferpa-requests",
    decision_type: "StatutoryAttestation",
    oscal_control_id: "AU-02",
    rationale: "Institutional statutory compliance attestation for 34 CFR Part 99",
    payload_hash: "b20c29f45612ba84caf6724fd3b167184451cf2e9336744ab97b664291130f78",
    entry_hash: "a43e8bb435f2126e7a68393e1147a468d601b0b556e4313fa4e183761858c211",
  },
];

export interface AdminDeskProps {
  initialPath?: string;
  initialEppn?: string;
}

export const AdminDesk: React.FC<AdminDeskProps> = ({ initialPath, initialEppn }) => {
  // Discreet URL Path Routing State
  const [currentPath, setCurrentPath] = useState<string>(() => {
    if (initialPath) return initialPath;
    if (typeof window !== "undefined") {
      return window.location.pathname;
    }
    return "/";
  });

  const isAdminPath = currentPath === "/admin" || currentPath.startsWith("/admin/");
  const isBuilderPath = currentPath === "/builder" || currentPath.startsWith("/builder/");
  const isAppPath = currentPath === "/app" || currentPath.startsWith("/app/");
  const isFormPath = currentPath === "/form" || currentPath.startsWith("/form/");

  // Active Workspace & Security State
  const [workspaces, setWorkspaces] = useState<Workspace[]>(INITIAL_WORKSPACES);
  const [activeWorkspaceId, setActiveWorkspaceId] = useState<string>(() => {
    if (typeof window !== "undefined") {
      const match = window.location.pathname.match(/^\/workspace\/([a-zA-Z0-9_-]+)/);
      if (match && match[1]) {
        return match[1];
      }
    }
    return "ws-cs-research";
  });
  const [pinnedWorkspaceIds, setPinnedWorkspaceIds] = useState<string[]>(() => {
    if (typeof window !== "undefined") {
      try {
        const saved = localStorage.getItem("scaffoldry_pinned_workspaces");
        if (saved) return JSON.parse(saved);
      } catch {}
    }
    return [];
  });
  const [workspaceGrouping, setWorkspaceGrouping] = useState<"organization" | "department">("organization");

  const handleTogglePinWorkspace = (e: React.MouseEvent, wsId: string) => {
    e.stopPropagation();
    const isPinned = pinnedWorkspaceIds.includes(wsId);
    const targetWs = workspaces.find((w) => w.id === wsId);
    const updated = isPinned
      ? pinnedWorkspaceIds.filter((id) => id !== wsId)
      : [...pinnedWorkspaceIds, wsId];
    setPinnedWorkspaceIds(updated);
    try {
      localStorage.setItem("scaffoldry_pinned_workspaces", JSON.stringify(updated));
    } catch {}
    showToast(
      isPinned
        ? `Unpinned workspace "${targetWs?.name || wsId}"`
        : `Pinned workspace "${targetWs?.name || wsId}" to top`
    );
  };
  const [workspaceAccessError, setWorkspaceAccessError] = useState<{
    status: number;
    message: string;
    workspaceId: string;
  } | null>(null);
  const [isWorkspaceSettingsOpen, setIsWorkspaceSettingsOpen] = useState<boolean>(false);
  const [navRailExpanded, setNavRailExpanded] = useState<boolean>(true);
  const [userMenuOpen, setUserMenuOpen] = useState<boolean>(false);
  const userMenuRef = useRef<HTMLDivElement>(null);

  // Studio & Co-Builder State
  const [studioOpen, setStudioOpen] = useState<boolean>(false);
  const [activeStudioApp, setActiveStudioApp] = useState<RegisteredApp | null>(null);
  const [studioTab, setStudioTab] = useState<"schema" | "collaborators" | "automations" | "preview" | "publish">("schema");
  const [automations, setAutomations] = useState<Record<string, WorkflowAutomationRule[]>>(INITIAL_AUTOMATIONS);

  const handleSaveRule = (appSlug: string, rule: WorkflowAutomationRule) => {
    setAutomations((prev) => {
      const list = prev[appSlug] || [];
      const exists = list.some((r) => r.id === rule.id);
      const updated = exists ? list.map((r) => (r.id === rule.id ? rule : r)) : [...list, rule];
      return { ...prev, [appSlug]: updated };
    });
    setNotificationToast(`Workflow automation rule "${rule.name}" saved.`);
    setTimeout(() => setNotificationToast(null), 3500);
  };

  const handleDeleteRule = (appSlug: string, ruleId: string) => {
    setAutomations((prev) => ({
      ...prev,
      [appSlug]: (prev[appSlug] || []).filter((r) => r.id !== ruleId),
    }));
    setNotificationToast("Workflow rule deleted.");
    setTimeout(() => setNotificationToast(null), 3500);
  };

  const handleToggleRule = (appSlug: string, ruleId: string) => {
    setAutomations((prev) => ({
      ...prev,
      [appSlug]: (prev[appSlug] || []).map((r) => (r.id === ruleId ? { ...r, enabled: !r.enabled } : r)),
    }));
  };

  // Admin Tab State (when on /admin)
  const [adminTab, setAdminTab] = useState<"overview" | "org" | "policy" | "ledger" | "impersonation" | "settings" | "people">(() => {
    if (typeof window !== "undefined") {
      const p = window.location.pathname;
      if (p === "/admin/org") return "org";
      if (p === "/admin/policy") return "policy";
      if (p === "/admin/ledger") return "ledger";
      if (p === "/admin/impersonation") return "impersonation";
      if (p === "/admin/people") return "people";
      if (p === "/admin/settings") return "settings";
    }
    return "overview";
  });
  const [ledger, setLedger] = useState<LedgerEntryItem[]>(INITIAL_LEDGER);

  const handleDownloadOscal = () => {
    const oscalDoc = {
      "component-definition": {
        uuid: "7b41e891-b384-48f8-b391-49520cb6b621",
        metadata: {
          title: "Scaffoldry Sovereign Application Platform Component Definition",
          "last-modified": new Date().toISOString(),
          version: "1.0.0",
          "oscal-version": "1.1.2",
        },
        components: [
          {
            uuid: "983a48e1-5f21-4f9e-a890-4100c5983b12",
            type: "software",
            title: "Scaffoldry Sovereign Application Platform",
            description: "Governed collaborative workspace, tabular engine, and Cedar authorization lattice",
            "control-implementations": [
              {
                uuid: "3f9821a0-47b2-4d92-9102-39c4a8501234",
                source: "https://doi.org/10.6028/NIST.SP.800-53r5",
                description: "Automated institutional control implementation and cryptographic verification ledger",
                "implemented-requirements": ledger.map((entry) => ({
                  uuid: `550e8400-e29b-41d4-a716-${entry.sequence.toString().padStart(12, "0")}`,
                  "control-id": entry.oscal_control_id.toLowerCase(),
                  description: entry.rationale,
                  props: [
                    { name: "ledger-sequence", value: entry.sequence.toString() },
                    { name: "ledger-principal", value: entry.principal },
                    { name: "ledger-org-code", value: entry.organization_code },
                    { name: "ledger-entry-hash", value: entry.entry_hash },
                    { name: "ledger-previous-hash", value: entry.previous_hash },
                  ],
                })),
              },
            ],
          },
        ],
      },
    };

    const blob = new Blob([JSON.stringify(oscalDoc, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "scaffoldry-oscal-1.1.2.json";
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
    setNotificationToast("Downloaded NIST OSCAL 1.1.2 compliance component definition.");
    setTimeout(() => setNotificationToast(null), 3500);
  };

  // Theme State
  const [darkMode, setDarkMode] = useState<boolean>(() => {
    if (typeof window !== "undefined") {
      return localStorage.getItem("scaffoldry-theme") === "dark" ||
        (!localStorage.getItem("scaffoldry-theme") && typeof window.matchMedia === "function" && window.matchMedia("(prefers-color-scheme: dark)").matches);
    }
    return true;
  });

  // User Persona & Impersonation State
  const [activePersona, setActivePersona] = useState<Persona>(() => {
    if (initialEppn) {
      const found = PERSONAS.find((p) => p.eppn === initialEppn);
      if (found) return found;
      return {
        eppn: initialEppn,
        name: "Fixture User",
        affiliation: initialEppn.includes("chair") || initialEppn.includes("dean") ? "faculty" : (initialEppn.includes("student") ? "student" : "staff"),
        department: "Physics",
        roleTitle: "User",
        isAdmin: false,
      };
    }
    return PERSONAS[0];
  });
  const [userUnits, setUserUnits] = useState<OrganizationNode[]>([]);
  const [selectedUnitId, setSelectedUnitId] = useState<string | null>(null);
  const [isImpersonating, setIsImpersonating] = useState<boolean>(false);
  const [realAdmin, setRealAdmin] = useState<Persona | null>(null);
  const [loginModalOpen, setLoginModalOpen] = useState<boolean>(false);
  const [userSettingsOpen, setUserSettingsOpen] = useState<boolean>(false);
  const [authToken, setAuthTokenState] = useState<string | null>(getAuthToken());
  const [pasteToken, setPasteToken] = useState("");
  const [signInError, setSignInError] = useState<string | null>(null);

  const handleTokenSignIn = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!pasteToken.trim()) return;
    try {
      setSignInError(null);
      setAuthToken(pasteToken.trim());
      const me = await apiClient.getCurrentUser();
      setAuthTokenState(pasteToken.trim());
      if (me?.user) {
        const found = PERSONAS.find((p) => p.eppn === me.user.eppn);
        if (found) {
          setActivePersona(found);
        } else {
          setActivePersona({
            eppn: me.user.eppn,
            name: me.user.name || me.user.eppn,
            affiliation: me.user.affiliation || "faculty",
            department: me.user.department || "General",
            roleTitle: me.user.affiliation || "Member",
            isAdmin: me.user.affiliation === "central_admin",
          });
        }
      }
    } catch (err: any) {
      setAuthToken(null);
      setSignInError(err?.message || "Invalid token");
    }
  };
  const [notificationToast, setNotificationToast] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    const syncWorkspaces = async () => {
      try {
        const serverWorkspaces = await apiClient.listWorkspaces().catch(() => null);
        if (!cancelled && Array.isArray(serverWorkspaces) && serverWorkspaces.length > 0) {
          setWorkspaces(serverWorkspaces);
        }
        const serverLedger = await apiClient.getGovernanceLedger().catch(() => null);
        if (!cancelled && serverLedger && Array.isArray(serverLedger.entries)) {
          setLedger(serverLedger.entries);
        }
        const serverOrgs = await apiClient.listOrganizations().catch(() => null);
        if (!cancelled && Array.isArray(serverOrgs)) {
          setUserUnits(serverOrgs);
        }
      } catch {
        // offline fallback
      }
    };

    syncWorkspaces();

    return () => {
      cancelled = true;
    };
  }, [activePersona.eppn]);

  useEffect(() => {
    let cancelled = false;

    const checkWorkspaceAccess = async () => {
      if (activeWorkspaceId === "all") {
        setWorkspaceAccessError(null);
        return;
      }

      try {
        const wsResp = await apiClient.getWorkspace(activeWorkspaceId);
        if (!cancelled) {
          setWorkspaceAccessError(null);
          if (wsResp && wsResp.workspace) {
            setWorkspaces((prev) =>
              prev.map((w) =>
                w.id === wsResp.workspace.id
                  ? { ...w, ...wsResp.workspace, collaborators: wsResp.collaborators || w.collaborators }
                  : w
              )
            );
          }
        }
      } catch (err: any) {
        if (!cancelled) {
          if (err.status === 403) {
            setWorkspaceAccessError({
              status: 403,
              message: err.message || "403 Forbidden: Cedar Policy restricts access",
              workspaceId: activeWorkspaceId,
            });
          } else {
            // When offline or mocked in unit tests without API server:
            const targetWs = INITIAL_WORKSPACES.find((w) => w.id === activeWorkspaceId);
            const isRestricted =
              targetWs &&
              targetWs.visibility === "restricted" &&
              activePersona.affiliation !== "central_admin" &&
              !targetWs.collaborators.some((c) => c.eppn === activePersona.eppn);

            if (isRestricted) {
              setWorkspaceAccessError({
                status: 403,
                message: `403 Forbidden: Cedar Policy restricts access to ${targetWs.name}`,
                workspaceId: activeWorkspaceId,
              });
            } else {
              setWorkspaceAccessError(null);
            }
          }
        }
      }
    };

    checkWorkspaceAccess();

    return () => {
      cancelled = true;
    };
  }, [activeWorkspaceId, activePersona.eppn]);

  const handleStartImpersonation = (targetPersona: Persona) => {
    const currentAdmin = realAdmin || activePersona;
    if (currentAdmin.affiliation !== "central_admin") {
      showToast("Access Denied: Cedar policy requires central_admin to impersonate users.");
      return;
    }
    setRealAdmin(currentAdmin);
    setIsImpersonating(true);
    setActivePersona(targetPersona);

    const auditEntry: LedgerEntryItem = {
      sequence: ledger.length,
      timestamp_iso: new Date().toISOString(),
      previous_hash: ledger[ledger.length - 1]?.entry_hash || "0".repeat(64),
      principal: currentAdmin.eppn,
      organization_code: "DIV-SECURITY-CENTRAL",
      app_slug: undefined,
      decision_type: "ImpersonationSessionStarted" as any,
      oscal_control_id: "AC-02",
      rationale: `Enterprise administrator ${currentAdmin.name} initiated verified user impersonation of ${targetPersona.name}`,
      payload_hash: `sha256:imp_start:${targetPersona.eppn}:${Date.now()}`,
      entry_hash: `sha256:block:${ledger.length}:${Date.now()}`,
    };
    setLedger((prev) => [...prev, auditEntry]);

    showToast(`Impersonating ${targetPersona.name} (${targetPersona.roleTitle})`);
    navigateTo("/");
  };

  const handleStopImpersonation = () => {
    if (!realAdmin) return;
    const admin = realAdmin;
    const impersonated = activePersona;

    const auditEntry: LedgerEntryItem = {
      sequence: ledger.length,
      timestamp_iso: new Date().toISOString(),
      previous_hash: ledger[ledger.length - 1]?.entry_hash || "0".repeat(64),
      principal: admin.eppn,
      organization_code: "DIV-SECURITY-CENTRAL",
      app_slug: undefined,
      decision_type: "ImpersonationSessionEnded" as any,
      oscal_control_id: "AC-02",
      rationale: `Enterprise administrator ${admin.name} concluded impersonation of ${impersonated.name}`,
      payload_hash: `sha256:imp_end:${impersonated.eppn}:${Date.now()}`,
      entry_hash: `sha256:block:${ledger.length}:${Date.now()}`,
    };
    setLedger((prev) => [...prev, auditEntry]);

    setActivePersona(admin);
    setIsImpersonating(false);
    setRealAdmin(null);
    showToast(`Exited impersonation. Restored administrator session for ${admin.name}`);
    navigateTo("/admin");
  };

  // Applications & Policy Data
  const [apps, setApps] = useState<RegisteredApp[]>(INITIAL_APPS);
  const [sourceRules] = useState<SourceRule[]>(INITIAL_SOURCE_RULES);
  const [searchQuery, setSearchQuery] = useState<string>("");
  const [statusFilter, setStatusFilter] = useState<string>("all");
  const [activeWorkspaceApp, setActiveWorkspaceApp] = useState<RegisteredApp | null>(null);

  // Navigation View State ("workspaces" | "datasets")
  const [mainView, setMainView] = useState<"workspaces" | "datasets">(() => {
    if (typeof window !== "undefined" && window.location.pathname.startsWith("/datasets")) {
      return "datasets";
    }
    return "workspaces";
  });

  // Policy Simulator State (inside /admin policy panel)
  const [simAction, setSimAction] = useState<"read" | "write" | "export">("export");
  const [simFerpa, setSimFerpa] = useState<boolean>(true);

  // Sync browser URL via HTML5 History API
  const navigateTo = (path: string) => {
    if (typeof window !== "undefined") {
      window.history.pushState(null, "", path);
      setCurrentPath(path);
      setActiveWorkspaceApp(null);
      if (path.startsWith("/datasets")) {
        setMainView("datasets");
      } else if (path === "/" || path.startsWith("/workspace")) {
        setMainView("workspaces");
      }
      if (path.startsWith("/workspace/")) {
        const wsId = path.replace("/workspace/", "").split("/")[0];
        if (wsId) {
          setActiveWorkspaceId(wsId);
        }
      }
    }
  };

  useEffect(() => {
    const handlePopState = () => {
      const p = window.location.pathname;
      setCurrentPath(p);
      setActiveWorkspaceApp(null);
      if (p.startsWith("/datasets")) {
        setMainView("datasets");
      } else if (p === "/" || p.startsWith("/workspace")) {
        setMainView("workspaces");
      }
      if (p.startsWith("/workspace/")) {
        const wsId = p.replace("/workspace/", "").split("/")[0];
        if (wsId) {
          setActiveWorkspaceId(wsId);
        }
      }
    };
    window.addEventListener("popstate", handlePopState);
    return () => window.removeEventListener("popstate", handlePopState);
  }, []);

  useEffect(() => {
    const root = document.documentElement;
    if (darkMode) {
      root.classList.add("dark");
      localStorage.setItem("scaffoldry-theme", "dark");
    } else {
      root.classList.remove("dark");
      localStorage.setItem("scaffoldry-theme", "light");
    }
  }, [darkMode]);

  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (userMenuRef.current && !userMenuRef.current.contains(event.target as Node)) {
        setUserMenuOpen(false);
      }
    };
    if (userMenuOpen) {
      document.addEventListener("mousedown", handleClickOutside);
    }
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
    };
  }, [userMenuOpen]);

  const showToast = (msg: string) => {
    setNotificationToast(msg);
    setTimeout(() => setNotificationToast(null), 3500);
  };

  // Evaluate Cedar Decision
  const evaluateCedarDecision = (targetDept: string, isFerpaSensitive: boolean, action: "read" | "write" | "export") => {
    if (activePersona.department !== targetDept && activePersona.department !== "compliance") {
      return {
        decision: "DENY" as const,
        reason: "Boundary Isolation Policy: Principal department does not match resource realm",
        rule: "rule-nist-ac3",
      };
    }
    if (action === "export" && isFerpaSensitive) {
      if (activePersona.affiliation !== "staff" || activePersona.department !== "compliance") {
        return {
          decision: "DENY" as const,
          reason: "FERPA 34 CFR § 99.30 Safeguard: Only designated compliance staff may export sensitive student records",
          rule: "rule-ferpa-30",
        };
      }
    }
    return {
      decision: "ALLOW" as const,
      reason: "Permitted by Cedar Role Policy: Principal holds verified departmental affiliation",
      rule: "rule-campus-l4",
    };
  };

  const simResult = evaluateCedarDecision("biology", simFerpa, simAction);

  const handleCreateUnitWorkspace = async () => {
    if (!selectedUnitId) return;
    const unit = userUnits.find((u) => u.id === selectedUnitId);
    const unitName = unit ? unit.name : "Unit";
    try {
      const newWs = await apiClient.createWorkspace({
        name: `${unitName} Workspace`,
        code: `${unit ? unit.code : "WS"}-${Date.now().toString().slice(-4)}`,
        organization: unitName,
        department: unit ? unit.code.toLowerCase() : "general",
        visibility: "departmental",
        organization_id: selectedUnitId,
      } as any);
      setWorkspaces((prev) => [...prev, newWs]);
      setActiveWorkspaceId(newWs.id);
      showToast(`Created workspace for ${unitName}`);
    } catch (err: any) {
      showToast(err.message || "Failed to create workspace");
    }
  };

  // Active Workspace & Security Access
  const currentWorkspace =
    workspaces.find((w) => w.id === activeWorkspaceId) ||
    INITIAL_WORKSPACES.find((w) => w.id === activeWorkspaceId) ||
    workspaces[0];
  const accessibleWorkspaces = workspaces.filter(
    (w) =>
      activePersona.affiliation === "central_admin" ||
      w.collaborators?.some((c) => c.eppn === activePersona.eppn) ||
      (w.visibility !== "restricted" && (
        w.visibility === "institutional" ||
        (w.visibility === "departmental" && w.department.toLowerCase() === activePersona.department.toLowerCase())
      ))
  );

  // Pinned & Grouped Workspaces (Restricted/inaccessible workspaces are completely excluded from lists)
  const pinnedWorkspaces = accessibleWorkspaces.filter((w) => pinnedWorkspaceIds.includes(w.id));
  const unpinnedWorkspaces = accessibleWorkspaces.filter((w) => !pinnedWorkspaceIds.includes(w.id));

  const workspaceGroups = useMemo(() => {
    const groups: { [key: string]: Workspace[] } = {};
    unpinnedWorkspaces.forEach((ws) => {
      const groupKey =
        workspaceGrouping === "department"
          ? (ws.department ? ws.department.charAt(0).toUpperCase() + ws.department.slice(1) : "Other")
          : (ws.organization || "General Academic Workspaces");
      if (!groups[groupKey]) groups[groupKey] = [];
      groups[groupKey].push(ws);
    });
    return groups;
  }, [unpinnedWorkspaces, workspaceGrouping]);
  const userRoleInCurrentWs = getUserWorkspaceRole(currentWorkspace, activePersona);
  const canManageCurrentWs = canUserManageWorkspace(currentWorkspace, activePersona);
  const isTargetRestricted =
    currentWorkspace.visibility === "restricted" &&
    activePersona.affiliation !== "central_admin" &&
    !currentWorkspace.collaborators?.some((c) => c.eppn === activePersona.eppn);
  const hasAccessToCurrentWs = activeWorkspaceId === "all" || (!workspaceAccessError && !isTargetRestricted);

  const handleSaveWorkspaceSettings = (updatedWs: Workspace, auditEntry: LedgerEntryItem) => {
    setWorkspaces((prev) => prev.map((w) => (w.id === updatedWs.id ? updatedWs : w)));
    setLedger((prev) => [...prev, auditEntry]);
    showToast(`Workspace "${updatedWs.name}" security & configuration updated.`);
  };

  // Workspace and App-Level Security Filtered Applications
  const workspaceApps = apps.filter((app) => {
    // Workspace boundary filter
    if (activeWorkspaceId !== "all" && app.workspaceId !== activeWorkspaceId) {
      return false;
    }

    // App-level Cedar security rules enforcement:
    // Only display applications that the current persona is authorized to access under institutional Cedar ABAC.
    if (!canUserAccessApp(app, activePersona, workspaces)) {
      return false;
    }

    const matchesSearch =
      app.title.toLowerCase().includes(searchQuery.toLowerCase()) ||
      app.slug.toLowerCase().includes(searchQuery.toLowerCase()) ||
      app.customDomain.toLowerCase().includes(searchQuery.toLowerCase());
    const matchesStatus = statusFilter === "all" || app.status === statusFilter;
    return matchesSearch && matchesStatus;
  });

  // Open App in Co-Builder Studio (Full-Page Builder)
  const handleOpenStudio = (app: RegisteredApp) => {
    if (!canUserAccessApp(app, activePersona, workspaces)) {
      showToast(`403 Forbidden: Cedar ABAC restricts access to ${app.title}`);
      return;
    }
    setActiveStudioApp(app);
    setStudioTab("schema");
    navigateTo(`/builder/${app.slug}`);
  };

  // Open App in Standalone Published Runtime
  const handleOpenPublishedApp = (app: RegisteredApp) => {
    if (!canUserAccessApp(app, activePersona, workspaces)) {
      showToast(`403 Forbidden: Cedar ABAC restricts access to ${app.title}`);
      return;
    }
    setActiveStudioApp(app);
    navigateTo(`/app/${app.slug}`);
  };

  const handleOpenApp = (app: RegisteredApp) => {
    if (!canUserAccessApp(app, activePersona, workspaces)) {
      showToast(`403 Forbidden: Cedar ABAC restricts access to ${app.title}`);
      return;
    }
    setActiveWorkspaceApp(app);
  };

  // Create New App in Active Workspace
  const handleCreateNewApp = () => {
    const newSlug = `app-${Date.now().toString().slice(-4)}`;
    const newApp: RegisteredApp = {
      slug: newSlug,
      title: "Untitled Department Application",
      orgCode: `DEPT-${currentWorkspace.department.toUpperCase()}`,
      department: currentWorkspace.department,
      customDomain: `${newSlug}.${currentWorkspace.department}.science.state.edu`,
      verified: false,
      hermCapability: "2.1.0 (Academic Operations)",
      cedsDomain: "PostsecondaryStudent",
      status: "Draft",
      updatedAt: new Date().toISOString().split("T")[0],
      recordsCount: 0,
      workspaceId: currentWorkspace.id,
      collaborators: [
        { eppn: activePersona.eppn, name: activePersona.name, role: "owner", department: activePersona.department },
      ],
      manifest: {
        slug: newSlug,
        title: "Untitled Department Application",
        description: "New collaborative application manifest ready for field design.",
        organization_code: `DEPT-${currentWorkspace.department.toUpperCase()}`,
        department: currentWorkspace.department,
        custom_domain: `${newSlug}.${currentWorkspace.department}.science.state.edu`,
        custom_domain_verified: false,
        views: [
          {
            id: "main-view",
            title: "Data Submission Form",
            view_type: "Form",
            fields: [
              { name: "record_title", label: "Title / Summary", field_type: "Text", required: true, ferpa_sensitive: false },
            ],
          },
        ],
        ceds_mappings: {},
      },
    };
    setApps([newApp, ...apps]);
    setActiveStudioApp(newApp);
    setStudioTab("schema");
    setStudioOpen(true);
    showToast(`Created new app "${newApp.title}". Ready to co-build.`);
  };

  // Add Field in Co-Builder Studio
  const handleAddFieldToStudioApp = () => {
    if (!activeStudioApp) return;
    const currentFields = activeStudioApp.manifest.views[0]?.fields || [];
    const fieldIndex = currentFields.length + 1;
    const newField: FieldSpec = {
      name: `field_${fieldIndex}`,
      label: `Field #${fieldIndex}`,
      field_type: "Text",
      required: false,
      ferpa_sensitive: false,
    };
    const updatedManifest: AppManifest = {
      ...activeStudioApp.manifest,
      views: [
        {
          ...activeStudioApp.manifest.views[0],
          fields: [...currentFields, newField],
        },
      ],
    };
    const updatedApp: RegisteredApp = {
      ...activeStudioApp,
      manifest: updatedManifest,
      status: activeStudioApp.status === "Published" ? "Published" : "Collaborating",
    };
    setActiveStudioApp(updatedApp);
    setApps((prev) => prev.map((a) => (a.slug === updatedApp.slug ? updatedApp : a)));
    showToast(`Added field "${newField.label}" to schema.`);
  };

  // Add Linked Dataset Field in Co-Builder Studio
  const handleAddLinkedDatasetField = (dataset: PublishedDataset) => {
    if (!activeStudioApp) return;
    const fieldKey = `${dataset.id}_ref`;
    const newField: FieldSpec = {
      name: fieldKey,
      label: `Assigned ${dataset.name}`,
      field_type: "Relation",
      required: true,
      ferpa_sensitive: dataset.sensitivity_level.includes("FERPA"),
      linked_dataset_id: dataset.id,
      linked_field: dataset.fields[0]?.name || "id",
    };
    const currentFields = activeStudioApp.manifest.views[0]?.fields || [];
    const updatedManifest: AppManifest = {
      ...activeStudioApp.manifest,
      views: [
        {
          ...activeStudioApp.manifest.views[0],
          fields: [...currentFields, newField],
        },
      ],
    };
    const updatedApp: RegisteredApp = {
      ...activeStudioApp,
      manifest: updatedManifest,
      status: activeStudioApp.status === "Published" ? "Published" : "Collaborating",
    };
    setActiveStudioApp(updatedApp);
    setApps((prev) => prev.map((a) => (a.slug === updatedApp.slug ? updatedApp : a)));
    showToast(`Linked dataset "${dataset.name}" as relation field.`);
  };

  // Toggle FERPA sensitivity on field
  const handleToggleFieldFerpa = (fieldName: string) => {
    if (!activeStudioApp) return;
    const currentFields = activeStudioApp.manifest.views[0]?.fields || [];
    const updatedFields = currentFields.map((f) =>
      f.name === fieldName ? { ...f, ferpa_sensitive: !f.ferpa_sensitive } : f
    );
    const updatedManifest: AppManifest = {
      ...activeStudioApp.manifest,
      views: [{ ...activeStudioApp.manifest.views[0], fields: updatedFields }],
    };
    const updatedApp: RegisteredApp = { ...activeStudioApp, manifest: updatedManifest };
    setActiveStudioApp(updatedApp);
    setApps((prev) => prev.map((a) => (a.slug === updatedApp.slug ? updatedApp : a)));
  };

  // Publish App Workflow
  const handlePublishApp = () => {
    if (!activeStudioApp) return;
    const updatedApp: RegisteredApp = {
      ...activeStudioApp,
      status: "Published",
      verified: true,
      manifest: {
        ...activeStudioApp.manifest,
        custom_domain_verified: true,
      },
    };
    setActiveStudioApp(updatedApp);
    setApps((prev) => prev.map((a) => (a.slug === updatedApp.slug ? updatedApp : a)));
    showToast(`Published "${updatedApp.title}" to ${updatedApp.customDomain}!`);
  };

  // Add collaborator to active app
  const handleAddCollaborator = (peer: Persona) => {
    if (!activeStudioApp) return;
    if (activeStudioApp.collaborators.some((c) => c.eppn === peer.eppn)) {
      showToast(`${peer.name} is already a collaborator on this app.`);
      return;
    }
    const newCollaborator: Collaborator = {
      eppn: peer.eppn,
      name: peer.name,
      role: "editor",
      department: peer.department,
    };
    const updatedApp: RegisteredApp = {
      ...activeStudioApp,
      collaborators: [...activeStudioApp.collaborators, newCollaborator],
      status: "Collaborating",
    };
    setActiveStudioApp(updatedApp);
    setApps((prev) => prev.map((a) => (a.slug === updatedApp.slug ? updatedApp : a)));
    showToast(`Added ${peer.name} as editor.`);
  };

  // Launch Studio linked to a published dataset
  const handleUseDatasetInApp = (dataset: PublishedDataset) => {
    const newSlug = `${dataset.id}-intake-${Date.now().toString().slice(-4)}`;
    const newApp: RegisteredApp = {
      slug: newSlug,
      title: `${dataset.name} Intake`,
      orgCode: dataset.department.slice(0, 4).toUpperCase(),
      department: dataset.department.toLowerCase(),
      customDomain: `${newSlug}.scaffoldry.internal`,
      verified: false,
      hermCapability: dataset.herm_capability_id || "ACA-01-APP",
      cedsDomain: "Higher Education / Academic Affairs",
      status: "Draft",
      updatedAt: "Just now",
      recordsCount: 0,
      workspaceId: activeWorkspaceId === "all" ? "ws-bio-lab" : activeWorkspaceId,
      collaborators: [
        {
          eppn: activePersona.eppn,
          name: activePersona.name,
          role: "owner",
          department: activePersona.department,
        },
      ],
      manifest: {
        slug: newSlug,
        title: `${dataset.name} Intake Application`,
        description: `Connected to published institutional dataset: ${dataset.name}`,
        organization_code: dataset.department.slice(0, 4).toUpperCase(),
        department: dataset.department.toLowerCase(),
        custom_domain: `${newSlug}.scaffoldry.internal`,
        custom_domain_verified: false,
        status: "Draft",
        workspace_id: activeWorkspaceId === "all" ? "ws-bio-lab" : activeWorkspaceId,
        views: [
          {
            id: "intake-form",
            title: "Intake Submission",
            view_type: "Form",
            fields: [
              {
                name: "reference_id",
                label: `Linked ${dataset.name}`,
                field_type: "Relation",
                required: true,
                ferpa_sensitive: dataset.sensitivity_level.includes("FERPA"),
                linked_dataset_id: dataset.id,
                linked_field: dataset.fields[0]?.name || "id",
              },
              {
                name: "notes",
                label: "Submission Notes & Justification",
                field_type: "Text",
                required: true,
                ferpa_sensitive: false,
              },
            ],
          },
        ],
        ceds_mappings: {},
        collaborators: [],
      },
    };
    setApps((prev) => [newApp, ...prev]);
    setActiveStudioApp(newApp);
    setStudioOpen(true);
    showToast(`Configured new application linked to ${dataset.name}`);
  };

  // Dedicated Full-Page App Builder Route (/builder/:slug)
  if (isBuilderPath) {
    const slug = currentPath.replace("/builder/", "").replace("/builder", "").split("/")[0] || apps[0]?.slug;
    const targetApp = apps.find((a) => a.slug === slug) || activeStudioApp || apps[0];
    const targetWs =
      workspaces.find((w) => w.id === targetApp?.workspaceId) ||
      INITIAL_WORKSPACES.find((w) => w.id === targetApp?.workspaceId) ||
      currentWorkspace;

    if (targetApp && !canUserAccessApp(targetApp, activePersona, workspaces)) {
      return (
        <div data-testid="workspace-access-denied" className="p-8 max-w-xl mx-auto my-12 bg-white dark:bg-slate-900 rounded-xl border border-rose-200 dark:border-rose-900/60 shadow-lg text-center space-y-4 animate-fade-in">
          <div className="w-12 h-12 mx-auto rounded-full bg-rose-100 dark:bg-rose-950/60 text-rose-600 dark:text-rose-400 flex items-center justify-center text-xl font-bold">
            🔒
          </div>
          <h2 className="text-lg font-bold text-slate-900 dark:text-white">
            403 Forbidden: Cedar Policy Sharing Boundary
          </h2>
          <p className="text-xs text-slate-600 dark:text-slate-400">
            Access to application <strong>{targetApp.title}</strong> ({targetApp.slug}) in workspace <strong>{targetWs.name}</strong> is restricted under institutional Cedar ABAC policy.
          </p>
          <div className="p-3 bg-slate-50 dark:bg-slate-800/60 rounded-lg text-left text-xs font-mono space-y-1">
            <div className="text-slate-500">Principal: <span className="text-slate-700 dark:text-slate-300 font-semibold">{activePersona.eppn}</span></div>
            <div className="text-slate-500">Affiliation: <span className="text-slate-700 dark:text-slate-300 font-semibold">{activePersona.affiliation}</span></div>
            <div className="text-slate-500">Principal Department: <span className="text-slate-700 dark:text-slate-300 font-semibold">{activePersona.department}</span></div>
            <div className="text-slate-500">Workspace Realm: <span className="text-slate-700 dark:text-slate-300 font-semibold">{targetWs.department}</span></div>
            <div className="text-slate-500">Workspace Visibility: <span className="text-amber-600 dark:text-amber-400 font-bold uppercase">{targetWs.visibility || "restricted"}</span></div>
            <div className="text-slate-500">Governing Policy: <span className="text-purple-600 dark:text-purple-400 font-semibold">Action::&quot;access_app&quot; (OSCAL AC-03)</span></div>
          </div>
          <button
            type="button"
            data-testid="switch-to-accessible-workspace-btn"
            onClick={() => navigateTo("/")}
            className="inline-flex items-center gap-1.5 px-3.5 py-2 text-xs font-semibold rounded-lg bg-blue-600 hover:bg-blue-700 text-white shadow-xs cursor-pointer transition-colors"
          >
            ← Return to Accessible Workspace
          </button>
        </div>
      );
    }

    return (
      <Suspense fallback={<div className="p-8 text-center text-slate-500">Loading App Builder...</div>}>
        <AppBuilder
          app={targetApp}
          onBack={() => navigateTo("/")}
          onOpenPublishedApp={(s) => navigateTo(`/app/${s}`)}
          onOpenIntakeForm={(tId) => navigateTo(tId ? `/form/${targetApp.slug}/${tId}` : `/form/${targetApp.slug}`)}
          onSaveApp={(updated) => {
            setApps((prev) => prev.map((a) => (a.slug === updated.slug ? updated : a)));
            setActiveStudioApp(updated);
          }}
        />
      </Suspense>
    );
  }

  // Dedicated Standalone Published App Runtime Route (/app/:slug)
  if (isAppPath) {
    const slug = currentPath.replace("/app/", "").replace("/app", "").split("/")[0] || apps[0]?.slug;
    const targetApp = apps.find((a) => a.slug === slug) || activeStudioApp || apps[0];
    const targetWs =
      workspaces.find((w) => w.id === targetApp?.workspaceId) ||
      INITIAL_WORKSPACES.find((w) => w.id === targetApp?.workspaceId) ||
      currentWorkspace;

    if (targetApp && !canUserAccessApp(targetApp, activePersona, workspaces)) {
      return (
        <div data-testid="workspace-access-denied" className="p-8 max-w-xl mx-auto my-12 bg-white dark:bg-slate-900 rounded-xl border border-rose-200 dark:border-rose-900/60 shadow-lg text-center space-y-4 animate-fade-in">
          <div className="w-12 h-12 mx-auto rounded-full bg-rose-100 dark:bg-rose-950/60 text-rose-600 dark:text-rose-400 flex items-center justify-center text-xl font-bold">
            🔒
          </div>
          <h2 className="text-lg font-bold text-slate-900 dark:text-white">
            403 Forbidden: Cedar Policy Sharing Boundary
          </h2>
          <p className="text-xs text-slate-600 dark:text-slate-400">
            Access to application <strong>{targetApp.title}</strong> ({targetApp.slug}) in workspace <strong>{targetWs.name}</strong> is restricted under institutional Cedar ABAC policy.
          </p>
          <div className="p-3 bg-slate-50 dark:bg-slate-800/60 rounded-lg text-left text-xs font-mono space-y-1">
            <div className="text-slate-500">Principal: <span className="text-slate-700 dark:text-slate-300 font-semibold">{activePersona.eppn}</span></div>
            <div className="text-slate-500">Affiliation: <span className="text-slate-700 dark:text-slate-300 font-semibold">{activePersona.affiliation}</span></div>
            <div className="text-slate-500">Principal Department: <span className="text-slate-700 dark:text-slate-300 font-semibold">{activePersona.department}</span></div>
            <div className="text-slate-500">Workspace Realm: <span className="text-slate-700 dark:text-slate-300 font-semibold">{targetWs.department}</span></div>
            <div className="text-slate-500">Workspace Visibility: <span className="text-amber-600 dark:text-amber-400 font-bold uppercase">{targetWs.visibility || "restricted"}</span></div>
            <div className="text-slate-500">Governing Policy: <span className="text-purple-600 dark:text-purple-400 font-semibold">Action::&quot;access_app&quot; (OSCAL AC-03)</span></div>
          </div>
          <button
            type="button"
            data-testid="switch-to-accessible-workspace-btn"
            onClick={() => navigateTo("/")}
            className="inline-flex items-center gap-1.5 px-3.5 py-2 text-xs font-semibold rounded-lg bg-blue-600 hover:bg-blue-700 text-white shadow-xs cursor-pointer transition-colors"
          >
            ← Return to Accessible Workspace
          </button>
        </div>
      );
    }

    return (
      <Suspense fallback={<div className="p-8 text-center text-slate-500">Loading Published App...</div>}>
        <PublishedAppView
          app={targetApp}
          onOpenBuilder={(s) => navigateTo(`/builder/${s}`)}
          onOpenIntakeForm={(tId) => navigateTo(tId ? `/form/${targetApp.slug}/${tId}` : `/form/${targetApp.slug}`)}
          onBackToDesk={() => navigateTo("/")}
        />
      </Suspense>
    );
  }

  // Dedicated Standalone Public Intake Form Route (/form/:slug or /form/:slug/:tableId)
  if (isFormPath) {
    const cleanPath = currentPath.replace("/form/", "").replace("/form", "");
    const parts = cleanPath ? cleanPath.split("/").filter(Boolean) : [];
    const slug = parts[0] || apps[0]?.slug;
    const tableId = parts[1] || undefined;
    const targetApp = apps.find((a) => a.slug === slug) || activeStudioApp || apps[0];
    return (
      <Suspense fallback={<div className="p-8 text-center text-slate-500">Loading Intake Form...</div>}>
        <StandaloneIntakeForm
          app={targetApp}
          tableId={tableId}
          onBackToDesk={() => navigateTo("/")}
          onOpenApp={() => navigateTo(`/app/${targetApp.slug}`)}
          onRecordSubmitted={(tblId, rec) => {
            showToast(`Record ${rec.id} submitted to ${targetApp.title} (${tblId}).`);
          }}
        />
      </Suspense>
    );
  }

  if (!authToken) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-slate-100 dark:bg-slate-950 p-4 font-sans">
        <div className="max-w-md w-full bg-white dark:bg-slate-900 rounded-xl shadow-xl border border-slate-200 dark:border-slate-800 p-6 space-y-4">
          <div className="flex items-center gap-3">
            <div className="w-10 h-10 rounded-lg bg-blue-600 text-white flex items-center justify-center font-bold text-lg">
              S
            </div>
            <div>
              <h1 className="text-base font-bold text-slate-900 dark:text-white">Sign in to Scaffoldry</h1>
              <p className="text-xs text-slate-500">Paste your authoritative token</p>
            </div>
          </div>
          <form onSubmit={handleTokenSignIn} className="space-y-4">
            <div>
              <label htmlFor="token-sign-in" className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1">
                Paste your token
              </label>
              <input
                id="token-sign-in"
                data-testid="token-sign-in"
                type="password"
                placeholder="Paste your token"
                value={pasteToken}
                onChange={(e) => setPasteToken(e.target.value)}
                className="w-full px-3 py-2 border border-slate-300 dark:border-slate-700 rounded-lg text-sm bg-white dark:bg-slate-800 text-slate-900 dark:text-white font-mono"
              />
            </div>
            {signInError && (
              <div className="text-xs text-red-600 dark:text-red-400 font-medium">{signInError}</div>
            )}
            <button
              type="submit"
              data-testid="token-sign-in-submit"
              className="w-full py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg text-xs font-semibold cursor-pointer"
            >
              Sign In
            </button>
          </form>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-slate-50 dark:bg-slate-950 text-slate-900 dark:text-slate-100 flex flex-col font-sans transition-colors duration-200">
      {/* Toast Notification */}
      {notificationToast && (
        <div className="fixed bottom-6 right-6 z-50 flex items-center gap-2 px-4 py-3 rounded-lg shadow-lg bg-slate-900 text-white dark:bg-white dark:text-slate-900 text-sm font-medium border border-slate-700 animate-fade-in">
          <span className="text-emerald-400 dark:text-emerald-600">✓</span>
          {notificationToast}
        </div>
      )}

      {/* SECURITY IMPERSONATION BANNER (OSCAL AC-02) */}
      {isImpersonating && realAdmin && (
        <div
          data-testid="impersonation-banner"
          className="bg-amber-500 dark:bg-amber-600 text-slate-950 dark:text-white px-4 py-2 border-b border-amber-600 dark:border-amber-700 flex flex-wrap items-center justify-between gap-3 text-xs font-medium shadow-sm z-50 sticky top-0"
        >
          <div className="flex items-center gap-2.5">
            <span className="bg-amber-900 text-amber-100 text-[10px] uppercase font-bold tracking-wider px-2 py-0.5 rounded">
              Impersonation Active (AC-02)
            </span>
            <span>
              Administrator: <strong>{realAdmin.name}</strong> ({realAdmin.roleTitle}) · Acting as:{" "}
              <strong>{activePersona.name}</strong> ({activePersona.roleTitle}, {activePersona.department})
            </span>
          </div>
          <button
            type="button"
            data-testid="exit-impersonation-btn"
            onClick={handleStopImpersonation}
            className="px-3 py-1 bg-slate-950 text-white hover:bg-slate-800 text-xs font-semibold rounded shadow-sm transition-colors cursor-pointer"
          >
            ✕ Exit Impersonation
          </button>
        </div>
      )}

      {/* TOP COMMAND BAR */}
      <header className="sticky top-0 z-40 h-14 bg-white dark:bg-slate-900 border-b border-slate-200 dark:border-slate-800 px-4 flex items-center justify-between shadow-xs">
        {/* Left: Brand & Workspace Selector */}
        <div className="flex items-center gap-3">
          <button
            type="button"
            onClick={() => setNavRailExpanded(!navRailExpanded)}
            className="p-1.5 rounded-md hover:bg-slate-100 dark:hover:bg-slate-800 text-slate-500 hover:text-slate-700 dark:text-slate-400 dark:hover:text-slate-200 cursor-pointer"
            title="Toggle Navigation Menu"
          >
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M4 6h16M4 12h16M4 18h16" />
            </svg>
          </button>

          <div className="flex items-center gap-2.5">
            <button
              type="button"
              onClick={() => navigateTo("/")}
              className="flex items-center gap-2 text-left cursor-pointer focus:outline-none"
            >
              <img
                src="/logo-mark.png"
                alt="Scaffoldry"
                className="h-7 w-auto object-contain shrink-0"
              />
              <span className="font-extrabold text-base tracking-wider text-slate-900 dark:text-white uppercase font-sans">
                SCAFFOLDRY
              </span>
            </button>
            <div className="h-4 w-px bg-slate-200 dark:bg-slate-800 hidden sm:block" />
            <span className="text-xs font-semibold text-slate-500 dark:text-slate-400 hidden sm:inline">
              Institutional Platform
            </span>
            {isAdminPath && (
              <span className="text-[10px] font-mono font-bold uppercase tracking-wider px-2 py-0.5 rounded bg-amber-500/10 text-amber-700 dark:text-amber-400 border border-amber-500/30">
                /admin console
              </span>
            )}
          </div>

          {/* Active Workspace Switcher (When on workspace view) */}
          {!isAdminPath && (
            <div className="hidden lg:flex items-center gap-2 ml-4">
              <span className="text-xs text-slate-400">Workspace:</span>
              <select
                data-testid="workspace-switcher-select"
                value={activeWorkspaceId}
                onChange={(e) => {
                  const val = e.target.value;
                  setActiveWorkspaceId(val);
                  if (val === "all") {
                    navigateTo("/");
                  } else {
                    navigateTo(`/workspace/${val}`);
                  }
                }}
                className="text-xs font-medium bg-slate-100 dark:bg-slate-800 border border-slate-200 dark:border-slate-700 rounded-md py-1 px-2.5 text-slate-800 dark:text-slate-200 focus:outline-none focus:ring-1 focus:ring-blue-500 cursor-pointer"
              >
                {pinnedWorkspaces.length > 0 && (
                  <optgroup label="📌 Pinned Workspaces" data-testid="pinned-optgroup">
                    {pinnedWorkspaces.map((ws) => (
                      <option key={`pinned-${ws.id}`} value={ws.id}>
                        📌 {ws.icon} {ws.name} ({ws.department})
                      </option>
                    ))}
                  </optgroup>
                )}
                {(Object.entries(workspaceGroups) as [string, Workspace[]][]).map(([groupTitle, wsList]) => (
                  <optgroup key={groupTitle} label={`🏛️ ${groupTitle}`}>
                    {wsList.map((ws) => (
                      <option key={ws.id} value={ws.id}>
                        {ws.icon} {ws.name} ({ws.department})
                      </option>
                    ))}
                  </optgroup>
                ))}
                <optgroup label="Campus-Wide">
                  <option value="all">🌐 All Campus Workspaces</option>
                </optgroup>
              </select>
            </div>
          )}
        </div>

        {/* Center: Search Bar */}
        <div className="flex-1 max-w-md mx-4 hidden md:block">
          <div className="relative">
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder="Search applications, fields, collaborators... (/)"
              className="w-full pl-9 pr-8 py-1.5 text-xs rounded-lg bg-slate-100 dark:bg-slate-800/70 border border-slate-200 dark:border-slate-700 text-slate-800 dark:text-slate-200 placeholder-slate-400 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:bg-white dark:focus:bg-slate-900 transition-colors"
            />
            <svg
              className="w-4 h-4 text-slate-400 absolute left-2.5 top-2 pointer-events-none"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
            </svg>
            {searchQuery && (
              <button
                type="button"
                onClick={() => setSearchQuery("")}
                className="absolute right-2.5 top-1.5 text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 text-xs"
              >
                ✕
              </button>
            )}
          </div>
        </div>

        {/* Right: Actions, Theme & User Badge */}
        <div className="flex items-center gap-2">

          {!isAdminPath && (
            <button
              type="button"
              onClick={handleCreateNewApp}
              className="hidden sm:inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold rounded-md bg-blue-600 hover:bg-blue-700 text-white shadow-xs cursor-pointer transition-colors"
            >
              <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2.5" d="M12 4v16m8-8H4" />
              </svg>
              <span>+ New App</span>
            </button>
          )}

          {/* Theme Toggle */}
          <button
            type="button"
            onClick={() => setDarkMode(!darkMode)}
            className="p-1.5 rounded-md hover:bg-slate-100 dark:hover:bg-slate-800 text-slate-500 hover:text-slate-700 dark:text-slate-400 dark:hover:text-slate-200 cursor-pointer"
            title={darkMode ? "Switch to Light Mode" : "Switch to Dark Mode"}
          >
            {darkMode ? (
              <svg className="w-4 h-4 text-amber-400" fill="currentColor" viewBox="0 0 20 20">
                <path
                  fillRule="evenodd"
                  d="M10 2a1 1 0 011 1v1a1 1 0 11-2 0V3a1 1 0 011-1zm4 8a4 4 0 11-8 0 4 4 0 018 0zm-.464 4.95l.707.707a1 1 0 001.414-1.414l-.707-.707a1 1 0 00-1.414 1.414zm2.12-10.607a1 1 0 010 1.414l-.706.707a1 1 0 11-1.414-1.414l.707-.707a1 1 0 011.414 0zM17 11a1 1 0 100-2h-1a1 1 0 100 2h1zm-7 4a1 1 0 011 1v1a1 1 0 11-2 0v-1a1 1 0 011-1zM5.05 6.464A1 1 0 106.465 5.05l-.708-.707a1 1 0 00-1.414 1.414l.707.707zm1.414 8.486l-.707.707a1 1 0 01-1.414-1.414l.707-.707a1 1 0 011.414 1.414zM4 11a1 1 0 100-2H3a1 1 0 000 2h1z"
                  clipRule="evenodd"
                />
              </svg>
            ) : (
              <svg className="w-4 h-4 text-slate-600" fill="currentColor" viewBox="0 0 20 20">
                <path d="M17.293 13.293A8 8 0 016.707 2.707a8.001 8.001 0 1010.586 10.586z" />
              </svg>
            )}
          </button>

          {/* USER BADGE & DISCREET POP-OVER MENU */}
          <div className="relative" ref={userMenuRef}>
            <button
              type="button"
              onClick={() => setUserMenuOpen(!userMenuOpen)}
              className="flex items-center gap-2 p-1.5 rounded-lg border border-slate-200 dark:border-slate-800 hover:bg-slate-100 dark:hover:bg-slate-800/80 cursor-pointer transition-colors"
              title="User Account & Persona Menu"
            >
              <div className="w-6 h-6 rounded-full bg-blue-600 text-white font-bold text-xs flex items-center justify-center uppercase shrink-0">
                {activePersona.name.split(" ").map((n) => n[0]).slice(0, 2).join("")}
              </div>
              <div className="text-left hidden md:block">
                <div className="text-xs font-semibold text-slate-800 dark:text-slate-200 leading-tight">
                  {activePersona.name}
                </div>
                <div className="text-[10px] text-slate-500 dark:text-slate-400 capitalize leading-tight">
                  {activePersona.affiliation} · {activePersona.department}
                </div>
              </div>
              <svg className={`w-3.5 h-3.5 text-slate-400 transition-transform ${userMenuOpen ? "rotate-180" : ""}`} fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M19 9l-7 7-7-7" />
              </svg>
            </button>

            {userMenuOpen && (
              <div className="absolute right-0 mt-2 w-72 bg-white dark:bg-slate-900 rounded-lg shadow-xl border border-slate-200 dark:border-slate-800 p-2 z-50 animate-fade-in text-xs">
                {/* Profile Header */}
                <div className="p-2 border-b border-slate-100 dark:border-slate-800">
                  <div className="font-semibold text-slate-900 dark:text-white">{activePersona.name}</div>
                  <div className="text-[11px] font-mono text-slate-500 dark:text-slate-400 break-all">{activePersona.eppn}</div>
                  <div className="mt-1 flex items-center gap-2">
                    <span className="inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-medium bg-blue-50 text-blue-700 dark:bg-blue-950/60 dark:text-blue-300">
                      {activePersona.roleTitle}
                    </span>
                    <span className="inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-mono font-medium bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-300 uppercase">
                      {activePersona.affiliation}
                    </span>
                  </div>
                </div>

                {/* Impersonation Status / Controls */}
                {isImpersonating && realAdmin && (
                  <div className="py-2 px-2 border-b border-slate-100 dark:border-slate-800 bg-amber-50/70 dark:bg-amber-950/30 rounded my-1">
                    <div className="text-[10px] uppercase font-bold tracking-wider text-amber-700 dark:text-amber-300 mb-1">
                      ⚠️ Impersonation Active
                    </div>
                    <div className="text-[11px] text-amber-800 dark:text-amber-200">
                      Real Admin: <strong>{realAdmin.name}</strong>
                    </div>
                    <button
                      type="button"
                      onClick={() => {
                        setUserMenuOpen(false);
                        handleStopImpersonation();
                      }}
                      className="mt-2 w-full py-1 text-center bg-amber-600 hover:bg-amber-700 text-white rounded text-xs font-semibold cursor-pointer shadow-2xs"
                    >
                      Exit Impersonation
                    </button>
                  </div>
                )}

                {/* Admin-Only Actions */}
                {activePersona.affiliation === "central_admin" && (
                  <div className="py-2 border-b border-slate-100 dark:border-slate-800 space-y-1">
                    <div className="text-[10px] uppercase font-bold tracking-wider text-slate-400 px-2 mb-1">
                      Administrative Tools
                    </div>
                    <button
                      type="button"
                      data-testid="menu-impersonation-hub-btn"
                      onClick={() => {
                        setUserMenuOpen(false);
                        navigateTo("/admin");
                        setAdminTab("impersonation");
                        showToast("Opened Identity & Impersonation Hub");
                      }}
                      className="w-full text-left px-2 py-1.5 rounded hover:bg-amber-50 dark:hover:bg-amber-950/40 text-amber-800 dark:text-amber-300 transition-colors flex items-center gap-2 cursor-pointer"
                    >
                      <span className="text-amber-500 font-bold">👤</span>
                      <div>
                        <div className="font-semibold text-xs">Directory User Impersonation</div>
                        <div className="text-[10px] text-slate-400">Act as faculty/staff with AC-02 audit</div>
                      </div>
                    </button>
                    {!isAdminPath && (
                      <button
                        type="button"
                        onClick={() => {
                          setUserMenuOpen(false);
                          navigateTo("/admin");
                          showToast("Navigated to discreet path: /admin");
                        }}
                        className="w-full text-left px-2 py-1.5 rounded hover:bg-slate-100 dark:hover:bg-slate-800 text-slate-700 dark:text-slate-300 transition-colors flex items-center gap-2 cursor-pointer"
                      >
                        <span className="text-slate-400">⚙️</span>
                        <div>
                          <div className="font-semibold text-xs">Admin Console (/admin)</div>
                        </div>
                      </button>
                    )}
                  </div>
                )}

                {/* User Settings */}
                <div className="pt-1 border-t border-slate-100 dark:border-slate-800">
                  <button
                    type="button"
                    data-testid="user-settings-btn"
                    onClick={() => {
                      setUserMenuOpen(false);
                      setUserSettingsOpen(true);
                    }}
                    className="w-full text-left px-2 py-1.5 rounded hover:bg-slate-100 dark:hover:bg-slate-800 text-slate-700 dark:text-slate-300 transition-colors flex items-center gap-2 cursor-pointer"
                  >
                    <span className="text-slate-400">⚙️</span>
                    <div>
                      <div className="font-semibold text-xs">User Settings</div>
                      <div className="text-[10px] text-slate-400">Mint agent tokens & preferences</div>
                    </div>
                  </button>
                </div>

                {/* Sign In / Switch Account */}
                <div className="pt-2">
                  <button
                    type="button"
                    data-testid="switch-account-modal-btn"
                    onClick={() => {
                      setUserMenuOpen(false);
                      setLoginModalOpen(true);
                    }}
                    className="w-full text-left px-2 py-1.5 rounded hover:bg-slate-100 dark:hover:bg-slate-800 text-slate-700 dark:text-slate-300 transition-colors flex items-center gap-2 cursor-pointer"
                  >
                    <span className="text-slate-400">🔑</span>
                    <div>
                      <div className="font-semibold text-xs">Sign In / Switch Identity</div>
                      <div className="text-[10px] text-slate-400">Authenticate InCommon credentials</div>
                    </div>
                  </button>
                </div>
              </div>
            )}
          </div>
        </div>
      </header>

      {/* BODY WITH NAVIGATION RAIL & MAIN CONTENT */}
      <div className="flex-1 flex overflow-hidden">
        {/* LEFT NAVIGATION RAIL */}
        <aside
          className={`${
            navRailExpanded ? "w-64" : "w-16"
          } bg-white dark:bg-slate-900 border-r border-slate-200 dark:border-slate-800 flex flex-col justify-between transition-all duration-200 z-20 shrink-0`}
        >
          <div className="p-3 space-y-6 overflow-y-auto">
            {/* IF ON /admin: Administrative Rail */}
            {isAdminPath ? (
              <div>
                {navRailExpanded && (
                  <div className="text-[11px] font-semibold uppercase tracking-wider text-amber-600 dark:text-amber-400 px-2 mb-2 flex items-center gap-1.5">
                    <span className="w-2 h-2 rounded-full bg-amber-500 animate-pulse" />
                    <span>Admin Controls (/admin)</span>
                  </div>
                )}
                <nav className="space-y-1">
                  <button
                    type="button"
                    onClick={() => setAdminTab("org")}
                    data-testid="admin-org-tab-btn"
                    className={`w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-xs font-medium transition-colors cursor-pointer ${
                      adminTab === "org"
                        ? "bg-amber-50 text-amber-800 dark:bg-amber-950/40 dark:text-amber-300 font-semibold"
                        : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
                    }`}
                  >
                    <svg className="w-4 h-4 shrink-0 text-amber-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M19 21V5a2 2 0 00-2-2H7a2 2 0 00-2 2v16m14 0h2m-2 0h-5m-9 0H3m2 0h5M9 7h1m-1 4h1m4-4h1m-1 4h1m-5 10v-5a1 1 0 011-1h2a1 1 0 011 1v5m-4 0h4" />
                    </svg>
                    {navRailExpanded && <span>Organization</span>}
                  </button>

                  <button
                    type="button"
                    onClick={() => setAdminTab("policy")}
                    className={`w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-xs font-medium transition-colors cursor-pointer ${
                      adminTab === "policy"
                        ? "bg-amber-50 text-amber-800 dark:bg-amber-950/40 dark:text-amber-300 font-semibold"
                        : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
                    }`}
                  >
                    <svg className="w-4 h-4 shrink-0 text-amber-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
                    </svg>
                    {navRailExpanded && <span>Policy &amp; OSCAL Lattice</span>}
                  </button>

                  <button
                    type="button"
                    onClick={() => setAdminTab("ledger")}
                    className={`w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-xs font-medium transition-colors cursor-pointer ${
                      adminTab === "ledger"
                        ? "bg-amber-50 text-amber-800 dark:bg-amber-950/40 dark:text-amber-300 font-semibold"
                        : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
                    }`}
                  >
                    <svg className="w-4 h-4 shrink-0 text-amber-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2m-3 7h3m-3 4h3m-6-4h.01M9 16h.01" />
                    </svg>
                    {navRailExpanded && <span>Decision Ledger &amp; OSCAL</span>}
                  </button>

                  <button
                    type="button"
                    onClick={() => {
                      setAdminTab("overview");
                      navigateTo("/admin");
                    }}
                    data-testid="admin-overview-tab-btn"
                    className={`w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-xs font-medium transition-colors cursor-pointer ${
                      adminTab === "overview"
                        ? "bg-amber-50 text-amber-800 dark:bg-amber-950/40 dark:text-amber-300 font-semibold"
                        : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
                    }`}
                  >
                    <svg className="w-4 h-4 shrink-0 text-amber-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M4 6a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2H6a2 2 0 01-2-2V6zM14 6a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2h-2a2 2 0 01-2-2V6zM4 16a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2H6a2 2 0 01-2-2v-2zM14 16a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2h-2a2 2 0 01-2-2v-2z" />
                    </svg>
                    {navRailExpanded && <span>Overview</span>}
                  </button>

                  {activePersona.affiliation === "central_admin" && (
                    <button
                      type="button"
                      data-testid="admin-settings-tab-btn"
                      onClick={() => setAdminTab("settings")}
                      className={`w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-xs font-medium transition-colors cursor-pointer ${
                        adminTab === "settings"
                          ? "bg-amber-50 text-amber-800 dark:bg-amber-950/40 dark:text-amber-300 font-semibold"
                          : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
                      }`}
                    >
                      <svg className="w-4 h-4 shrink-0 text-amber-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                      </svg>
                      {navRailExpanded && <span>Platform Settings</span>}
                    </button>
                  )}

                  <button
                    type="button"
                    data-testid="admin-people-tab-btn"
                    onClick={() => setAdminTab("people")}
                    className={`w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-xs font-medium transition-colors cursor-pointer ${
                      adminTab === "people"
                        ? "bg-amber-50 text-amber-800 dark:bg-amber-950/40 dark:text-amber-300 font-semibold"
                        : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
                    }`}
                  >
                    <svg className="w-4 h-4 shrink-0 text-amber-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0z" />
                    </svg>
                    {navRailExpanded && <span>People</span>}
                  </button>

                  <button
                    type="button"
                    data-testid="admin-impersonation-tab-btn"
                    onClick={() => setAdminTab("impersonation")}
                    className={`w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-xs font-medium transition-colors cursor-pointer ${
                      adminTab === "impersonation"
                        ? "bg-amber-50 text-amber-800 dark:bg-amber-950/40 dark:text-amber-300 font-semibold"
                        : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
                    }`}
                  >
                    <svg className="w-4 h-4 shrink-0 text-amber-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z" />
                    </svg>
                    {navRailExpanded && <span>Identity &amp; Impersonation</span>}
                  </button>

                  <div className="pt-3 border-t border-slate-200 dark:border-slate-800">
                    <button
                      type="button"
                      onClick={() => navigateTo("/")}
                      className="w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-xs font-medium text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60 transition-colors cursor-pointer"
                    >
                      <svg className="w-4 h-4 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M10 19l-7-7m0 0l7-7m-7 7h18" />
                      </svg>
                      {navRailExpanded && <span>Return to Workspace (/)</span>}
                    </button>
                  </div>
                </nav>
              </div>
            ) : (
              /* PRIMARY END-USER WORKSPACE RAIL */
              <>
                {/* ORG UNIT ADMIN BLOCK */}
                {activePersona.affiliation !== "central_admin" && userUnits.length > 0 && (
                  <div data-testid="unit-admin-rail" className="mb-4 p-2.5 bg-amber-50/60 dark:bg-amber-950/30 border border-amber-200/80 dark:border-amber-900/60 rounded-lg space-y-2">
                    <div className="text-[10px] font-bold uppercase tracking-wider text-amber-800 dark:text-amber-400">
                      Org Unit Admin
                    </div>
                    <div className="text-xs font-semibold text-slate-700 dark:text-slate-300">
                      Your units
                    </div>
                    <div className="space-y-1">
                      {userUnits.map((unit) => {
                        const isSelected = selectedUnitId === unit.id;
                        return (
                          <button
                            key={unit.id}
                            type="button"
                            data-testid={`unit-rail-${unit.code}`}
                            onClick={() => setSelectedUnitId(isSelected ? null : unit.id)}
                            className={`w-full flex items-center justify-between px-2 py-1.5 rounded text-xs transition-colors cursor-pointer text-left ${
                              isSelected
                                ? "bg-amber-600 text-white font-semibold shadow-xs"
                                : "bg-white dark:bg-slate-800 text-slate-700 dark:text-slate-300 hover:bg-amber-100/50"
                            }`}
                          >
                            <span className="truncate">{unit.name}</span>
                            <span className="text-[10px] font-mono opacity-80">{unit.code}</span>
                          </button>
                        );
                      })}
                    </div>
                    {selectedUnitId && (
                      <button
                        type="button"
                        data-testid="unit-new-workspace-btn"
                        onClick={handleCreateUnitWorkspace}
                        className="w-full mt-2 px-2.5 py-1.5 text-xs font-semibold rounded bg-amber-600 text-white hover:bg-amber-700 shadow-xs flex items-center justify-center gap-1 cursor-pointer"
                      >
                        <span>+ New workspace</span>
                      </button>
                    )}
                  </div>
                )}
                <div>
                  <div className="flex items-center justify-between px-2 mb-2">
                    {navRailExpanded ? (
                      <>
                        <div className="text-[11px] font-semibold uppercase tracking-wider text-slate-400 dark:text-slate-500">
                          Workspaces
                        </div>
                        <div className="flex items-center gap-1 bg-slate-100 dark:bg-slate-800/80 p-0.5 rounded text-[10px]">
                          <button
                            type="button"
                            data-testid="group-by-org-btn"
                            onClick={() => setWorkspaceGrouping("organization")}
                            className={`px-1.5 py-0.5 rounded font-medium transition-colors cursor-pointer ${
                              workspaceGrouping === "organization"
                                ? "bg-white dark:bg-slate-700 text-blue-600 dark:text-blue-400 shadow-2xs font-semibold"
                                : "text-slate-400 hover:text-slate-600 dark:hover:text-slate-300"
                            }`}
                            title="Group by College / Organization"
                          >
                            College
                          </button>
                          <button
                            type="button"
                            data-testid="group-by-dept-btn"
                            onClick={() => setWorkspaceGrouping("department")}
                            className={`px-1.5 py-0.5 rounded font-medium transition-colors cursor-pointer ${
                              workspaceGrouping === "department"
                                ? "bg-white dark:bg-slate-700 text-blue-600 dark:text-blue-400 shadow-2xs font-semibold"
                                : "text-slate-400 hover:text-slate-600 dark:hover:text-slate-300"
                            }`}
                            title="Group by Department"
                          >
                            Dept
                          </button>
                        </div>
                      </>
                    ) : (
                      <div className="w-full text-center text-[10px] text-slate-400">WS</div>
                    )}
                  </div>
                  <nav className="space-y-2">
                    {/* Pinned Workspaces Section */}
                    {pinnedWorkspaces.length > 0 && (
                      <div className="mb-2" data-testid="pinned-workspaces-group">
                        {navRailExpanded && (
                          <div className="text-[10px] font-bold uppercase tracking-wider text-amber-600 dark:text-amber-400 px-2 mb-1 flex items-center justify-between">
                            <span className="flex items-center gap-1">
                              <span>📌</span>
                              <span>Pinned</span>
                            </span>
                            <span className="text-[9px] font-mono px-1 py-0.2 rounded bg-amber-100 dark:bg-amber-950/60 text-amber-700 dark:text-amber-300">
                              {pinnedWorkspaces.length}
                            </span>
                          </div>
                        )}
                        <div className="space-y-0.5">
                          {pinnedWorkspaces.map((ws) => {
                            const isActive = activeWorkspaceId === ws.id && mainView === "workspaces";
                            return (
                              <div
                                key={`pinned-${ws.id}`}
                                className={`group w-full flex items-center justify-between px-2.5 py-1.5 rounded-lg text-xs font-medium transition-colors ${
                                  isActive
                                    ? "bg-blue-50 text-blue-700 dark:bg-blue-900/30 dark:text-blue-300 font-semibold"
                                    : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
                                }`}
                              >
                                <button
                                  type="button"
                                  data-testid={`workspace-rail-btn-${ws.id}`}
                                  onClick={() => {
                                    setActiveWorkspaceId(ws.id);
                                    setMainView("workspaces");
                                    navigateTo(`/workspace/${ws.id}`);
                                  }}
                                  className="flex-1 flex items-center gap-2.5 text-left truncate cursor-pointer py-0.5"
                                  title={ws.name}
                                >
                                  <span className="text-sm shrink-0">{ws.icon}</span>
                                  {navRailExpanded && (
                                    <span className="truncate">{ws.name}</span>
                                  )}
                                </button>
                                {navRailExpanded && (
                                  <div className="flex items-center gap-1 shrink-0 ml-1">
                                    <span className="text-[10px] px-1.5 py-0.2 rounded-full bg-slate-100 dark:bg-slate-800 text-slate-500">
                                      {apps.filter((a) => a.workspaceId === ws.id).length}
                                    </span>
                                    <button
                                      type="button"
                                      data-testid={`pin-workspace-${ws.id}`}
                                      onClick={(e) => handleTogglePinWorkspace(e, ws.id)}
                                      title="Unpin workspace"
                                      className="p-0.5 rounded hover:bg-slate-200 dark:hover:bg-slate-700 transition-opacity cursor-pointer text-amber-500 dark:text-amber-400 opacity-100"
                                    >
                                      📌
                                    </button>
                                  </div>
                                )}
                              </div>
                            );
                          })}
                        </div>
                      </div>
                    )}

                    {/* Grouped Accessible Workspaces */}
                    {(Object.entries(workspaceGroups) as [string, Workspace[]][]).map(([groupTitle, wsList]) => (
                      <div
                        key={groupTitle}
                        className="mb-2"
                        data-testid={`workspace-group-${groupTitle.toLowerCase().replace(/[^a-z0-9]+/g, "-")}`}
                      >
                        {navRailExpanded && (
                          <div className="text-[10px] font-bold uppercase tracking-wider text-slate-400 dark:text-slate-500 px-2 mb-1 flex items-center justify-between">
                            <span className="truncate">{groupTitle}</span>
                            <span className="text-[9px] font-mono px-1 py-0.2 rounded bg-slate-100 dark:bg-slate-800 text-slate-500 ml-1">
                              {wsList.length}
                            </span>
                          </div>
                        )}
                        <div className="space-y-0.5">
                          {wsList.map((ws) => {
                            const isActive = activeWorkspaceId === ws.id && mainView === "workspaces";
                            return (
                              <div
                                key={ws.id}
                                className={`group w-full flex items-center justify-between px-2.5 py-1.5 rounded-lg text-xs font-medium transition-colors ${
                                  isActive
                                    ? "bg-blue-50 text-blue-700 dark:bg-blue-900/30 dark:text-blue-300 font-semibold"
                                    : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
                                }`}
                              >
                                <button
                                  type="button"
                                  data-testid={`workspace-rail-btn-${ws.id}`}
                                  onClick={() => {
                                    setActiveWorkspaceId(ws.id);
                                    setMainView("workspaces");
                                    navigateTo(`/workspace/${ws.id}`);
                                  }}
                                  className="flex-1 flex items-center gap-2.5 text-left truncate cursor-pointer py-0.5"
                                  title={ws.name}
                                >
                                  <span className="text-sm shrink-0">{ws.icon}</span>
                                  {navRailExpanded && (
                                    <span className="truncate">{ws.name}</span>
                                  )}
                                </button>
                                {navRailExpanded && (
                                  <div className="flex items-center gap-1 shrink-0 ml-1">
                                    <span className="text-[10px] px-1.5 py-0.2 rounded-full bg-slate-100 dark:bg-slate-800 text-slate-500">
                                      {apps.filter((a) => a.workspaceId === ws.id).length}
                                    </span>
                                    <button
                                      type="button"
                                      data-testid={`pin-workspace-${ws.id}`}
                                      onClick={(e) => handleTogglePinWorkspace(e, ws.id)}
                                      title="Pin workspace to top"
                                      className="p-0.5 rounded hover:bg-slate-200 dark:hover:bg-slate-700 transition-opacity cursor-pointer text-slate-400 opacity-0 group-hover:opacity-100 hover:text-amber-500"
                                    >
                                      📍
                                    </button>
                                  </div>
                                )}
                              </div>
                            );
                          })}
                        </div>
                      </div>
                    ))}

                    <div className="pt-1 border-t border-slate-100 dark:border-slate-800">
                      <button
                        type="button"
                        data-testid="all-campus-apps-rail-btn"
                        onClick={() => {
                          setActiveWorkspaceId("all");
                          setMainView("workspaces");
                          navigateTo("/");
                        }}
                        className={`w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-xs font-medium transition-colors cursor-pointer ${
                          activeWorkspaceId === "all" && mainView === "workspaces"
                            ? "bg-blue-50 text-blue-700 dark:bg-blue-900/30 dark:text-blue-300 font-semibold"
                            : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
                        }`}
                      >
                        <span className="text-sm shrink-0">🌐</span>
                        {navRailExpanded && <span>All Campus Apps</span>}
                      </button>
                    </div>
                  </nav>
                </div>

                <div>
                  {navRailExpanded && (
                    <div className="text-[11px] font-semibold uppercase tracking-wider text-slate-400 dark:text-slate-500 px-2 mb-2">
                      Data &amp; Studio Tools
                    </div>
                  )}
                  <nav className="space-y-1">
                    <button
                      type="button"
                      onClick={() => {
                        setMainView("datasets");
                        navigateTo("/datasets");
                      }}
                      className={`w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-xs font-medium transition-colors cursor-pointer ${
                        mainView === "datasets"
                          ? "bg-blue-50 text-blue-700 dark:bg-blue-900/30 dark:text-blue-300 font-semibold"
                          : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
                      }`}
                      title="Published Institutional Datasets & Relational Lattice"
                    >
                      <span className="text-sm shrink-0">🗄️</span>
                      {navRailExpanded && (
                        <span className="flex-1 text-left flex items-center justify-between truncate">
                          <span className="truncate">Datasets &amp; Lattice</span>
                          <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-purple-100 dark:bg-purple-900/40 text-purple-700 dark:text-purple-300 font-mono font-bold">
                            4
                          </span>
                        </span>
                      )}
                    </button>

                    <button
                      type="button"
                      onClick={handleCreateNewApp}
                      className="w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-xs font-medium text-blue-600 dark:text-blue-400 hover:bg-blue-50 dark:hover:bg-blue-950/40 transition-colors cursor-pointer"
                    >
                      <svg className="w-4 h-4 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M12 4v16m8-8H4" />
                      </svg>
                      {navRailExpanded && <span>New App Studio</span>}
                    </button>
                  </nav>
                </div>
              </>
            )}
          </div>

          {/* Rail Footer */}
          {navRailExpanded && (
            <div className="p-3 border-t border-slate-200 dark:border-slate-800 text-[11px] text-slate-500 space-y-1">
              <div className="flex justify-between items-center">
                <span>Identity:</span>
                <span className="font-mono text-slate-700 dark:text-slate-300">{activePersona.affiliation}</span>
              </div>
              <div className="flex justify-between items-center">
                <span>Department:</span>
                <span className="font-mono text-slate-700 dark:text-slate-300">{activePersona.department}</span>
              </div>
              <div className="flex justify-between items-center">
                <span>DNS Ingress:</span>
                <span className="font-mono text-slate-700 dark:text-slate-300">&lt; 1 ms</span>
              </div>
            </div>
          )}
        </aside>

        {/* MAIN CANVAS */}
        <main className="flex-1 overflow-y-auto p-4 md:p-6 bg-slate-100/50 dark:bg-slate-950">
          {/* DISCREET ADMIN CONSOLE VIEW (/admin) */}
          {isAdminPath ? (
            <AdminConsole
                activePersona={activePersona}
                isImpersonating={isImpersonating}
                realAdmin={realAdmin}
                handleStopImpersonation={handleStopImpersonation}
                navigateTo={navigateTo}
                adminTab={adminTab}
                setAdminTab={setAdminTab}
                apps={apps}
                sourceRules={sourceRules}
                simAction={simAction}
                setSimAction={setSimAction}
                simFerpa={simFerpa}
                setSimFerpa={setSimFerpa}
                simResult={simResult}
                ledger={ledger}
                setNotificationToast={setNotificationToast}
                handleDownloadOscal={handleDownloadOscal}
                personas={PERSONAS}
                handleStartImpersonation={handleStartImpersonation}
              />
          ) : mainView === "datasets" ? (
            /* DATASET EXPLORER & RELATIONAL LATTICE VIEW */
            <DatasetExplorer onUseInApp={handleUseDatasetInApp} />
          ) : activeWorkspaceApp ? (
            /* MULTI-VIEW TABULAR WORKSPACE (GRID, KANBAN, CALENDAR, GALLERY, FORM) */
            <MultiViewWorkspace
              app={activeWorkspaceApp}
              onBack={() => setActiveWorkspaceApp(null)}
              onOpenStudio={() => {
                setActiveStudioApp(activeWorkspaceApp);
                setStudioOpen(true);
              }}
              onRecordCreated={() => showToast("Registered new record with Cedar policy validation.")}
            />
          ) : !hasAccessToCurrentWs ? (
            /* CEDAR ABAC 403 FORBIDDEN WORKSPACE SHARING BOUNDARY */
            <div data-testid="workspace-access-denied" className="p-8 max-w-xl mx-auto my-12 bg-white dark:bg-slate-900 rounded-xl border border-rose-200 dark:border-rose-900/60 shadow-lg text-center space-y-4 animate-fade-in">
              <div className="w-12 h-12 mx-auto rounded-full bg-rose-100 dark:bg-rose-950/60 text-rose-600 dark:text-rose-400 flex items-center justify-center text-xl font-bold">
                🔒
              </div>
              <h2 className="text-lg font-bold text-slate-900 dark:text-white">
                403 Forbidden: Cedar Policy Sharing Boundary
              </h2>
              <p className="text-xs text-slate-600 dark:text-slate-400">
                Access to workspace <strong>{currentWorkspace.name}</strong> ({currentWorkspace.id}) is restricted to authorized members and departmental peers under institutional Cedar ABAC policy.
              </p>
              <div className="p-3 bg-slate-50 dark:bg-slate-800/60 rounded-lg text-left text-xs font-mono space-y-1">
                <div className="text-slate-500">Principal: <span className="text-slate-700 dark:text-slate-300 font-semibold">{activePersona.eppn}</span></div>
                <div className="text-slate-500">Affiliation: <span className="text-slate-700 dark:text-slate-300 font-semibold">{activePersona.affiliation}</span></div>
                <div className="text-slate-500">Principal Department: <span className="text-slate-700 dark:text-slate-300 font-semibold">{activePersona.department}</span></div>
                <div className="text-slate-500">Workspace Realm: <span className="text-slate-700 dark:text-slate-300 font-semibold">{currentWorkspace.department}</span></div>
                <div className="text-slate-500">Workspace Visibility: <span className="text-amber-600 dark:text-amber-400 font-bold uppercase">{currentWorkspace.visibility || "restricted"}</span></div>
                <div className="text-slate-500">Governing Policy: <span className="text-purple-600 dark:text-purple-400 font-semibold">Action::&quot;access_workspace&quot; (OSCAL AC-03)</span></div>
              </div>
              <button
                type="button"
                data-testid="switch-to-accessible-workspace-btn"
                onClick={() => {
                  const accessible = accessibleWorkspaces[0];
                  if (accessible) {
                    setActiveWorkspaceId(accessible.id);
                  } else {
                    setActiveWorkspaceId("all");
                  }
                }}
                className="inline-flex items-center gap-1.5 px-3.5 py-2 text-xs font-semibold rounded-lg bg-blue-600 hover:bg-blue-700 text-white shadow-xs cursor-pointer transition-colors"
              >
                Switch to Accessible Workspace
              </button>
            </div>
          ) : (
            /* PRIMARY END-USER WORKSPACE CANVAS (/) */
            <div className="space-y-6 max-w-7xl mx-auto">
              {/* Workspace Header Banner */}
              <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl p-5 shadow-xs flex flex-wrap items-center justify-between gap-4">
                <div className="flex items-center gap-3.5">
                  <div className="w-12 h-12 rounded-xl bg-blue-50 dark:bg-blue-950/60 border border-blue-200 dark:border-blue-800 flex items-center justify-center text-2xl shadow-xs">
                    {currentWorkspace.icon}
                  </div>
                  <div>
                    <div className="flex items-center gap-2 flex-wrap">
                      <h1 className="text-lg font-bold text-slate-900 dark:text-white tracking-tight">
                        {currentWorkspace.name}
                      </h1>
                      <span className={`text-[10px] font-mono font-semibold px-2 py-0.5 rounded border ${
                        currentWorkspace.visibility === "restricted"
                          ? "bg-rose-50 text-rose-700 dark:bg-rose-950/50 dark:text-rose-300 border-rose-200 dark:border-rose-800"
                          : currentWorkspace.visibility === "departmental"
                          ? "bg-amber-50 text-amber-700 dark:bg-amber-950/50 dark:text-amber-300 border-amber-200 dark:border-amber-800"
                          : "bg-emerald-50 text-emerald-700 dark:bg-emerald-950/50 dark:text-emerald-300 border-emerald-200 dark:border-emerald-800"
                      }`}>
                        {currentWorkspace.visibility === "restricted" ? "🔒 Members Only" : currentWorkspace.visibility === "departmental" ? "🏢 Departmental" : "🌐 Institutional"}
                      </span>
                      {currentWorkspace.data_classification && (
                        <span className="text-[10px] font-mono px-2 py-0.5 rounded bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-400 border border-slate-200 dark:border-slate-700">
                          {currentWorkspace.data_classification}
                        </span>
                      )}
                      {userRoleInCurrentWs && (
                        <span className="text-[10px] font-bold uppercase tracking-wider px-2 py-0.5 rounded bg-blue-50 text-blue-700 dark:bg-blue-900/40 dark:text-blue-300 border border-blue-200 dark:border-blue-800">
                          Role: {userRoleInCurrentWs}
                        </span>
                      )}
                      <button
                        type="button"
                        data-testid={`workspace-header-pin-btn-${currentWorkspace.id}`}
                        onClick={(e) => handleTogglePinWorkspace(e, currentWorkspace.id)}
                        className={`inline-flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-medium border cursor-pointer transition-colors ${
                          pinnedWorkspaceIds.includes(currentWorkspace.id)
                            ? "bg-amber-50 text-amber-700 border-amber-300 dark:bg-amber-950/50 dark:text-amber-300 dark:border-amber-800 font-semibold"
                            : "bg-white text-slate-600 border-slate-200 dark:bg-slate-800 dark:text-slate-400 dark:border-slate-700 hover:bg-slate-50 dark:hover:bg-slate-750"
                        }`}
                        title={pinnedWorkspaceIds.includes(currentWorkspace.id) ? "Unpin this workspace" : "Pin this workspace to top"}
                      >
                        <span>📌</span>
                        <span>{pinnedWorkspaceIds.includes(currentWorkspace.id) ? "Pinned" : "Pin"}</span>
                      </button>
                    </div>
                    <p className="text-xs text-slate-500 dark:text-slate-400 mt-1">
                      {currentWorkspace.description}
                    </p>
                  </div>
                </div>

                <div className="flex items-center gap-3">
                  <div className="text-right text-xs hidden sm:block">
                    <div className="text-slate-400">Workspace Lead:</div>
                    <div className="font-semibold text-slate-700 dark:text-slate-300">{currentWorkspace.lead}</div>
                  </div>
                  {canManageCurrentWs && (
                    <button
                      type="button"
                      data-testid="workspace-settings-btn"
                      onClick={() => setIsWorkspaceSettingsOpen(true)}
                      className="inline-flex items-center gap-1.5 px-3 py-2 text-xs font-semibold rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 hover:bg-slate-50 dark:hover:bg-slate-750 text-slate-700 dark:text-slate-200 shadow-xs cursor-pointer transition-colors"
                      title="Manage Workspace Security, Sharing Boundaries & Collaborators"
                    >
                      <svg className="w-4 h-4 text-slate-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                      </svg>
                      <span>Security &amp; Settings</span>
                    </button>
                  )}
                  <button
                    type="button"
                    onClick={handleCreateNewApp}
                    className="inline-flex items-center gap-1.5 px-3.5 py-2 text-xs font-semibold rounded-lg bg-blue-600 hover:bg-blue-700 text-white shadow-xs cursor-pointer transition-colors"
                  >
                    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2.5" d="M12 4v16m8-8H4" />
                    </svg>
                    <span>Co-Build New App</span>
                  </button>
                </div>
              </div>

              {/* Process Desk Decisions Queue Panel */}
              <ProcessDesk
                apps={workspaceApps}
                callerPersona={activePersona}
                callerWorkspaceRole={userRoleInCurrentWs}
              />

              {/* Status Filter Bar */}
              <div className="flex flex-wrap items-center justify-between gap-3 text-xs">
                <div className="flex items-center gap-1.5">
                  <span className="text-slate-400 text-[11px] font-medium">Filter by Status:</span>
                  {(["all", "Published", "Collaborating", "Draft"] as const).map((s) => (
                    <button
                      key={s}
                      type="button"
                      onClick={() => setStatusFilter(s)}
                      className={`px-2.5 py-1 rounded-md capitalize transition-colors cursor-pointer ${
                        statusFilter === s
                          ? "bg-slate-200 dark:bg-slate-800 text-blue-600 dark:text-blue-400 font-bold"
                          : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
                      }`}
                    >
                      {s}
                    </button>
                  ))}
                </div>
                <div className="text-slate-400 text-xs">
                  Showing <strong>{workspaceApps.length}</strong> applications
                </div>
              </div>

              {/* Applications Card Grid */}
              <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
                {workspaceApps.map((app) => (
                  <div
                    key={app.slug}
                    className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 hover:border-blue-400 dark:hover:border-blue-600 rounded-xl p-5 shadow-xs transition-all flex flex-col justify-between"
                  >
                    <div>
                      {/* Card Header */}
                      <div className="flex items-center justify-between mb-3">
                        <span className="text-[10px] uppercase font-bold tracking-wider px-2 py-0.5 rounded bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-400">
                          {app.department}
                        </span>
                        <span
                          className={`text-[10px] font-semibold px-2 py-0.5 rounded ${
                            app.status === "Published"
                              ? "bg-emerald-50 text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800"
                              : app.status === "Collaborating"
                              ? "bg-blue-50 text-blue-700 dark:bg-blue-950/60 dark:text-blue-300 border border-blue-200 dark:border-blue-800"
                              : "bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300 border border-slate-200 dark:border-slate-700"
                          }`}
                        >
                          {app.status === "Published" && "✓ "}
                          {app.status}
                        </span>
                      </div>

                      <h3 className="font-bold text-sm text-slate-900 dark:text-white mb-1">
                        {app.title}
                      </h3>
                      <p className="text-xs text-slate-500 dark:text-slate-400 mb-3 line-clamp-2">
                        {app.manifest.description}
                      </p>

                      {/* Vanity DNS link */}
                      <div className="flex items-center gap-1 font-mono text-[11px] text-blue-600 dark:text-blue-400 mb-3">
                        <span>🔗</span>
                        <span className="truncate">{app.customDomain}</span>
                      </div>

                      {/* Fields Count & CEDS tags */}
                      <div className="text-[11px] text-slate-500 dark:text-slate-400 space-y-1">
                        <div>Fields: <strong>{app.manifest.views[0]?.fields?.length || 0} inputs</strong></div>
                        <div>HERM: <span className="text-slate-600 dark:text-slate-300">{app.hermCapability}</span></div>
                      </div>
                    </div>

                    {/* Card Footer: Collaborator Avatars & Action */}
                    <div className="mt-4 pt-3 border-t border-slate-100 dark:border-slate-800 flex items-center justify-between">
                      {/* Collaborator Avatars */}
                      <div className="flex -space-x-1.5 overflow-hidden">
                        {app.collaborators.map((c) => (
                          <div
                            key={c.eppn}
                            title={`${c.name} (${c.role})`}
                            className="inline-block h-6 w-6 rounded-full ring-2 ring-white dark:ring-slate-900 bg-blue-600 text-white font-bold text-[10px] text-center leading-6 uppercase"
                          >
                            {c.name[0]}
                          </div>
                        ))}
                      </div>

                      <div className="flex items-center gap-1.5">
                        <button
                          type="button"
                          onClick={() => handleOpenPublishedApp(app)}
                          className="inline-flex items-center gap-1 text-xs font-bold px-2 py-1 rounded bg-emerald-50 dark:bg-emerald-950/60 text-emerald-700 dark:text-emerald-300 hover:bg-emerald-100 dark:hover:bg-emerald-900/60 cursor-pointer"
                          title="Open Live Published App"
                        >
                          <span>App ↗</span>
                        </button>
                        <button
                          type="button"
                          onClick={() => handleOpenStudio(app)}
                          className="inline-flex items-center gap-1 text-xs font-semibold px-2 py-1 rounded bg-blue-50 dark:bg-blue-950/60 text-blue-700 dark:text-blue-300 hover:bg-blue-100 dark:hover:bg-blue-900/60 cursor-pointer"
                          title="Open Full-Page App Builder"
                        >
                          <span>Builder ✎</span>
                        </button>
                        <button
                          type="button"
                          onClick={() => handleOpenApp(app)}
                          className="p-1 rounded text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 cursor-pointer"
                          title="Quick Tabular Grid & Views"
                        >
                          <span>▦</span>
                        </button>
                      </div>
                    </div>
                  </div>
                ))}
              </div>

              {workspaceApps.length === 0 && (
                <div className="py-16 text-center bg-white dark:bg-slate-900 rounded-xl border border-slate-200 dark:border-slate-800 p-8">
                  <div className="text-3xl mb-2">📦</div>
                  <h3 className="text-sm font-semibold text-slate-800 dark:text-slate-200">No applications found in this workspace</h3>
                  <p className="text-xs text-slate-500 mt-1 mb-4">Click below to start co-building your first departmental application.</p>
                  <button
                    type="button"
                    onClick={handleCreateNewApp}
                    className="px-3 py-1.5 text-xs font-semibold rounded bg-blue-600 text-white"
                  >
                    + Co-Build First App
                  </button>
                </div>
              )}

              {/* Workspace Security & Configuration Modal */}
              <WorkspaceSettingsModal
                workspace={currentWorkspace}
                activePersona={activePersona}
                allPersonas={PERSONAS}
                isOpen={isWorkspaceSettingsOpen}
                onClose={() => setIsWorkspaceSettingsOpen(false)}
                onSave={handleSaveWorkspaceSettings}
                ledgerEntries={ledger}
              />
            </div>
          )}
        </main>
      </div>

      {/* CO-BUILDER STUDIO MODAL */}
      <CoBuilderStudioModal
        isOpen={studioOpen}
        onClose={() => setStudioOpen(false)}
        activeStudioApp={activeStudioApp!}
        setActiveStudioApp={setActiveStudioApp}
        studioTab={studioTab}
        setStudioTab={setStudioTab}
        automations={automations}
        handleSaveRule={handleSaveRule}
        handleDeleteRule={handleDeleteRule}
        handleToggleRule={handleToggleRule}
        handleAddLinkedDatasetField={handleAddLinkedDatasetField}
        handleAddFieldToStudioApp={handleAddFieldToStudioApp}
        handleToggleFieldFerpa={handleToggleFieldFerpa}
        handleAddCollaborator={handleAddCollaborator}
        handlePublishApp={handlePublishApp}
        showToast={showToast}
        personas={PERSONAS}
        setApps={setApps}
      />

      <UserSettingsModal
        isOpen={userSettingsOpen}
        onClose={() => setUserSettingsOpen(false)}
        eppn={activePersona.eppn}
      />

      {/* LOGIN / IDENTITY AUTHENTICATION MODAL */}
      {loginModalOpen && (
        <div
          data-testid="login-modal"
          className="fixed inset-0 z-50 bg-slate-900/60 backdrop-blur-xs flex items-center justify-center p-4 animate-fade-in"
        >
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl shadow-2xl max-w-md w-full p-6 space-y-4">
            <div className="flex items-center justify-between pb-3 border-b border-slate-100 dark:border-slate-800">
              <div>
                <h2 className="text-base font-bold text-slate-900 dark:text-white">
                  Institutional Sign-In (InCommon Federation)
                </h2>
                <p className="text-xs text-slate-500">
                  Select an institutional identity or authenticate credentials
                </p>
              </div>
              <button
                type="button"
                onClick={() => setLoginModalOpen(false)}
                className="text-slate-400 hover:text-slate-600 text-sm cursor-pointer"
              >
                ✕
              </button>
            </div>

            <div className="space-y-2">
              {PERSONAS.map((p) => (
                <button
                  key={p.eppn}
                  type="button"
                  data-testid={`login-as-${p.eppn}-btn`}
                  onClick={() => {
                    setActivePersona(p);
                    setIsImpersonating(false);
                    setRealAdmin(null);
                    setLoginModalOpen(false);
                    showToast(`Authenticated as ${p.name}`);
                  }}
                  className="w-full text-left p-3 rounded-lg border border-slate-200 dark:border-slate-800 hover:border-blue-500 hover:bg-blue-50/40 dark:hover:bg-blue-950/20 transition-all flex items-center justify-between cursor-pointer"
                >
                  <div>
                    <div className="text-xs font-semibold text-slate-900 dark:text-white">
                      {p.name}
                    </div>
                    <div className="text-[11px] text-slate-500">
                      {p.roleTitle} · {p.department}
                    </div>
                    <div className="text-[10px] font-mono text-slate-400">{p.eppn}</div>
                  </div>
                  <span className="text-xs font-semibold text-blue-600 dark:text-blue-400">
                    Sign In →
                  </span>
                </button>
              ))}
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

export default AdminDesk;
