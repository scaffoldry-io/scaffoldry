import React, { useState } from "react";
import { ManifestRenderer } from "./ManifestRenderer";
import { AppManifest } from "./types";

interface SourceRule {
  id: string;
  source: string;
  title: string;
  oscalControl: string;
  cedarPolicyId: string;
  cedarSnippet: string;
  targetSensitivity: string;
  status: "Enforced" | "Audit-Only";
}

interface Persona {
  eppn: string;
  name: string;
  affiliation: string;
  department: string;
  roleTitle: string;
}

const PERSONAS: Persona[] = [
  {
    eppn: "prof.curie@science.state.edu",
    name: "Dr. Marie Curie",
    affiliation: "faculty",
    department: "biology",
    roleTitle: "Professor & Lab Director",
  },
  {
    eppn: "student.smith@science.state.edu",
    name: "Alex Smith",
    affiliation: "student",
    department: "biology",
    roleTitle: "Graduate Research Assistant",
  },
  {
    eppn: "dr.watson@science.state.edu",
    name: "Dr. Arthur Watson",
    affiliation: "staff",
    department: "compliance",
    roleTitle: "Campus FERPA & Export Compliance Officer",
  },
  {
    eppn: "einstein@physics.state.edu",
    name: "Albert Einstein",
    affiliation: "student",
    department: "physics",
    roleTitle: "Physics Research Fellow",
  },
];

const SOURCE_RULES: SourceRule[] = [
  {
    id: "rule-ferpa-30",
    source: "Federal Law: 34 CFR Part 99 § 99.30",
    title: "FERPA Written Consent Requirement for Educational Records",
    oscalControl: "FERPA-34CFR-99.30 / NIST-800-53-AC-3",
    cedarPolicyId: "ferpa-export-forbid-guard",
    cedarSnippet: "forbid (principal, action == Action::\"export\", resource) when { resource.is_ferpa_sensitive && !(principal.scoped_affiliation in [\"staff\", \"compliance\"]) };",
    targetSensitivity: "FERPA Sensitive",
    status: "Enforced",
  },
  {
    id: "rule-nist-ac3",
    source: "NIST SP 800-53 Rev 5 / CMMC",
    title: "Access Enforcement & Departmental Realm Isolation",
    oscalControl: "NIST-800-53-AC-3 / AC-6",
    cedarPolicyId: "dept-realm-boundary",
    cedarSnippet: "permit (principal, action in [Action::\"read\", Action::\"write\"], resource) when { principal.department == resource.department };",
    targetSensitivity: "Institutional Internal",
    status: "Enforced",
  },
  {
    id: "rule-campus-l4",
    source: "University Data Protection Standard v4.2",
    title: "Level 4 Highly Restricted Research & Student Data",
    oscalControl: "INST-DATA-STD-L4",
    cedarPolicyId: "level4-strict-audit",
    cedarSnippet: "permit (principal, action == Action::\"read\", resource is Record) when { principal.scoped_affiliation in [\"faculty\", \"staff\", \"student\"] };",
    targetSensitivity: "Level 4 Restricted",
    status: "Enforced",
  },
];

const REGISTERED_APPS = [
  {
    slug: "bio-lab-inventory",
    title: "Biology Lab Equipment & Bioassay Register",
    orgCode: "DEPT-BIO",
    customDomain: "bio-inventory.science.state.edu",
    verified: true,
    hermCapability: "2.2.3 (Research Grant Administration)",
    cedsDomain: "Facility & Equipment (FICM 210)",
    status: "Published",
  },
  {
    slug: "bio-travel-grants",
    title: "Departmental Graduate Travel Authorizations",
    orgCode: "DEPT-BIO",
    customDomain: "travel.science.state.edu",
    verified: true,
    hermCapability: "2.2.1 (Research Operations)",
    cedsDomain: "PostsecondaryStudent",
    status: "Published",
  },
  {
    slug: "physics-laser-safety",
    title: "High-Energy Optics & Laser Safety Log",
    orgCode: "DEPT-PHYSICS",
    customDomain: "lasers.physics.state.edu",
    verified: true,
    hermCapability: "4.1.2 (Health & Safety Compliance)",
    cedsDomain: "Facility SpaceUtilization",
    status: "Published",
  },
];

const sampleManifest: AppManifest = {
  slug: "bio-lab-inventory",
  title: "Biology Lab Equipment & Chemical Inventory",
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
        {
          name: "item_name",
          label: "Equipment Name",
          field_type: "Text",
          required: true,
          ferpa_sensitive: false,
        },
        {
          name: "serial_number",
          label: "Serial Number",
          field_type: "Text",
          required: true,
          ferpa_sensitive: false,
        },
        {
          name: "operator_eval",
          label: "Student Operator Evaluation",
          field_type: "Text",
          required: false,
          ferpa_sensitive: true,
        },
      ],
    },
  ],
  ceds_mappings: {
    item_name: "000185",
  },
};

export const AdminDesk: React.FC = () => {
  const [activeTab, setActiveTab] = useState<"policy" | "apps" | "exemplar" | "audit">("policy");
  const [activePersona, setActivePersona] = useState<Persona>(PERSONAS[0]);
  const [simAction, setSimAction] = useState<"read" | "write" | "export">("export");
  const [simFerpa, setSimFerpa] = useState<boolean>(true);

  // Evaluate Cedar Decision client-side for immediate administrative feedback
  const evaluateSimulator = () => {
    // Cross department denial
    if (activePersona.department !== "biology") {
      return {
        decision: "DENY",
        reason: "Boundary Isolation Policy: Principal department does not match resource department",
        rule: "rule-nist-ac3",
      };
    }

    // FERPA export guard
    if (simAction === "export" && simFerpa) {
      if (activePersona.affiliation !== "staff" && activePersona.department !== "compliance") {
        return {
          decision: "DENY",
          reason: "FERPA 34 CFR § 99.30 Safeguard: Only compliance staff may export sensitive student records",
          rule: "rule-ferpa-30",
        };
      }
    }

    return {
      decision: "ALLOW",
      reason: "Permitted by Cedar Role Policy: Principal holds verified departmental affiliation",
      rule: "rule-campus-l4",
    };
  };

  const simResult = evaluateSimulator();

  return (
    <div>
      {/* Top Sovereign Header */}
      <header className="top-header">
        <div className="brand-section">
          <span className="brand-badge">SCAFFOLDRY</span>
          <div>
            <h1 className="brand-title">The Sovereign Desk</h1>
            <span className="brand-subtitle">Institution: State University (IPEDS: 234076) · Realm: science.state.edu</span>
          </div>
        </div>

        <div className="user-selector-container">
          <label htmlFor="persona-select" style={{ fontSize: "0.85rem", color: "var(--text-secondary)", fontWeight: 500 }}>
            Simulated InCommon Identity:
          </label>
          <select
            id="persona-select"
            className="persona-select"
            value={activePersona.eppn}
            onChange={(e) => {
              const p = PERSONAS.find((item) => item.eppn === e.target.value);
              if (p) setActivePersona(p);
            }}
          >
            {PERSONAS.map((p) => (
              <option key={p.eppn} value={p.eppn}>
                {p.name} [{p.affiliation.toUpperCase()} - {p.department.toUpperCase()}]
              </option>
            ))}
          </select>
        </div>
      </header>

      <main className="app-container">
        {/* Metric Badges */}
        <section className="metrics-grid">
          <div className="glass-panel metric-card">
            <span className="metric-label">Institutional Apps</span>
            <span className="metric-value">3 Active</span>
            <span className="metric-subtext">Zero VMs / Pure Metadata Manifests</span>
          </div>
          <div className="glass-panel metric-card">
            <span className="metric-label">Verified Vanity DNS</span>
            <span className="metric-value">3 Domains</span>
            <span className="metric-subtext">Zero-Open-Port Cloudflare Ingress</span>
          </div>
          <div className="glass-panel metric-card">
            <span className="metric-label">Source Rules Crosswalked</span>
            <span className="metric-value">3 Baselines</span>
            <span className="metric-subtext">FERPA § 99.30 · NIST 800-53 · CEDS v11</span>
          </div>
          <div className="glass-panel metric-card">
            <span className="metric-label">Cedar Policy Speed</span>
            <span className="metric-value">&lt; 1 ms</span>
            <span className="metric-subtext">Formally Verified Lean 4 Engine</span>
          </div>
        </section>

        {/* Tab Navigation */}
        <nav className="tab-bar">
          <button
            className={`tab-btn ${activeTab === "policy" ? "active" : ""}`}
            onClick={() => setActiveTab("policy")}
          >
            Institutional Source Rules &amp; OSCAL Lattice
          </button>
          <button
            className={`tab-btn ${activeTab === "apps" ? "active" : ""}`}
            onClick={() => setActiveTab("apps")}
          >
            Applications &amp; DNS Aliasing
          </button>
          <button
            className={`tab-btn ${activeTab === "exemplar" ? "active" : ""}`}
            onClick={() => setActiveTab("exemplar")}
          >
            Live Departmental App (Exemplar)
          </button>
        </nav>

        {/* TAB 1: Policy Mapping & Source Rules */}
        {activeTab === "policy" && (
          <section>
            <div className="glass-panel" style={{ padding: "1.5rem", marginBottom: "2rem" }}>
              <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "1rem" }}>
                <div>
                  <h2 style={{ margin: 0, fontSize: "1.25rem" }}>Regulatory Baselines to Cedar Policy Mapping</h2>
                  <p style={{ margin: "0.25rem 0", color: "var(--text-secondary)", fontSize: "0.85rem" }}>
                    Bidirectional crosswalk linking statutory mandates (FERPA, NIST SP 800-53) to executable Cedar rules via NIST OSCAL 1.1.2.
                  </p>
                </div>
                <span className="badge badge-green">NIST OSCAL 1.1.2 Compliant</span>
              </div>

              <div className="table-container">
                <table className="admin-table">
                  <thead>
                    <tr>
                      <th>Statutory / Source Rule</th>
                      <th>OSCAL Control ID</th>
                      <th>Executable Cedar Policy</th>
                      <th>Target Classification</th>
                      <th>Status</th>
                    </tr>
                  </thead>
                  <tbody>
                    {SOURCE_RULES.map((rule) => (
                      <tr key={rule.id}>
                        <td>
                          <strong>{rule.source}</strong>
                          <div style={{ fontSize: "0.8rem", color: "var(--text-secondary)" }}>{rule.title}</div>
                        </td>
                        <td>
                          <span className="badge badge-purple">{rule.oscalControl}</span>
                        </td>
                        <td>
                          <code className="code-preview" style={{ display: "block", maxWidth: "450px" }}>
                            {rule.cedarSnippet}
                          </code>
                        </td>
                        <td>
                          <span className="badge badge-amber">{rule.targetSensitivity}</span>
                        </td>
                        <td>
                          <span className="badge badge-green">{rule.status}</span>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>

            {/* Interactive Policy Simulator */}
            <div className="glass-panel simulator-panel">
              <h3 style={{ margin: "0 0 0.5rem 0", fontSize: "1.1rem" }}>
                Interactive Cedar Policy Decision Simulator
              </h3>
              <p style={{ fontSize: "0.85rem", color: "var(--text-secondary)", margin: "0 0 1.25rem 0" }}>
                Test access requests in real time under the active InCommon identity: <strong>{activePersona.eppn}</strong> ({activePersona.roleTitle}).
              </p>

              <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(200px, 1fr))", gap: "1rem", marginBottom: "1.5rem" }}>
                <div>
                  <label style={{ display: "block", fontSize: "0.8rem", color: "var(--text-muted)", marginBottom: "0.25rem" }}>
                    Requested Action:
                  </label>
                  <select
                    className="persona-select"
                    style={{ width: "100%" }}
                    value={simAction}
                    onChange={(e) => setSimAction(e.target.value as "read" | "write" | "export")}
                  >
                    <option value="read">Action::&quot;read&quot;</option>
                    <option value="write">Action::&quot;write&quot;</option>
                    <option value="export">Action::&quot;export&quot; (Requires Special Clearance)</option>
                  </select>
                </div>

                <div>
                  <label style={{ display: "block", fontSize: "0.8rem", color: "var(--text-muted)", marginBottom: "0.25rem" }}>
                    Record Sensitivity:
                  </label>
                  <select
                    className="persona-select"
                    style={{ width: "100%" }}
                    value={simFerpa ? "true" : "false"}
                    onChange={(e) => setSimFerpa(e.target.value === "true")}
                  >
                    <option value="false">Standard Operational Record (Non-FERPA)</option>
                    <option value="true">FERPA Sensitive Student Record (34 CFR § 99.30)</option>
                  </select>
                </div>

                <div>
                  <label style={{ display: "block", fontSize: "0.8rem", color: "var(--text-muted)", marginBottom: "0.25rem" }}>
                    Target Department:
                  </label>
                  <input
                    type="text"
                    disabled
                    value="biology"
                    style={{ background: "var(--bg-tertiary)", border: "1px solid var(--border-color)", padding: "0.4rem 0.8rem", borderRadius: "var(--radius-md)", color: "var(--text-muted)", width: "100%" }}
                  />
                </div>
              </div>

              {/* Simulation Result Banner */}
              <div
                style={{
                  padding: "1rem",
                  borderRadius: "var(--radius-md)",
                  border: `1px solid ${simResult.decision === "ALLOW" ? "#10b981" : "#ef4444"}`,
                  background: simResult.decision === "ALLOW" ? "rgba(16, 185, 129, 0.1)" : "rgba(239, 68, 68, 0.1)",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                }}
              >
                <div>
                  <div style={{ display: "flex", alignItems: "center", gap: "0.5rem", marginBottom: "0.25rem" }}>
                    <span className={`badge ${simResult.decision === "ALLOW" ? "badge-green" : "badge-red"}`} style={{ fontSize: "0.85rem", padding: "4px 10px" }}>
                      CEDAR {simResult.decision}
                    </span>
                    <strong style={{ fontSize: "0.95rem" }}>{simResult.reason}</strong>
                  </div>
                  <div style={{ fontSize: "0.8rem", color: "var(--text-secondary)" }}>
                    Evaluated under principal <code>{activePersona.eppn}</code> with scoped affiliation <code>{activePersona.affiliation}@{activePersona.department}</code>
                  </div>
                </div>
              </div>
            </div>
          </section>
        )}

        {/* TAB 2: Applications & DNS Aliases */}
        {activeTab === "apps" && (
          <section className="glass-panel" style={{ padding: "1.5rem" }}>
            <h2 style={{ margin: "0 0 0.5rem 0", fontSize: "1.25rem" }}>Registered Departmental Applications &amp; DNS Vanity Routing</h2>
            <p style={{ margin: "0 0 1.5rem 0", color: "var(--text-secondary)", fontSize: "0.85rem" }}>
              Every application is dynamically rendered from JSON metadata manifests with sub-millisecond host-header DNS routing.
            </p>

            <table className="admin-table">
              <thead>
                <tr>
                  <th>Application Title &amp; Slug</th>
                  <th>Organization</th>
                  <th>Departmental DNS Alias</th>
                  <th>DNS Status</th>
                  <th>EDUCAUSE HERM Capability</th>
                  <th>Action</th>
                </tr>
              </thead>
              <tbody>
                {REGISTERED_APPS.map((app) => (
                  <tr key={app.slug}>
                    <td>
                      <strong>{app.title}</strong>
                      <div style={{ fontSize: "0.8rem", color: "var(--text-muted)" }}>slug: {app.slug}</div>
                    </td>
                    <td>{app.orgCode}</td>
                    <td>
                      <code style={{ color: "var(--badge-blue-text)", fontWeight: 600 }}>{app.customDomain}</code>
                    </td>
                    <td>
                      <span className="badge badge-green">✓ Verified &amp; Routed</span>
                    </td>
                    <td>
                      <span className="badge badge-purple">{app.hermCapability}</span>
                    </td>
                    <td>
                      <button
                        style={{ background: "transparent", border: "1px solid var(--border-color)", color: "var(--text-primary)", padding: "4px 8px", borderRadius: "4px", cursor: "pointer", fontSize: "0.8rem" }}
                        onClick={() => setActiveTab("exemplar")}
                      >
                        Inspect Live
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </section>
        )}

        {/* TAB 3: Live Departmental App (Exemplar) */}
        {activeTab === "exemplar" && (
          <section className="glass-panel" style={{ padding: "2rem" }}>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "1.5rem" }}>
              <div>
                <h2 style={{ margin: 0, fontSize: "1.25rem" }}>Live Departmental Exemplar (Application #1)</h2>
                <p style={{ margin: "0.25rem 0", color: "var(--text-secondary)", fontSize: "0.85rem" }}>
                  Demonstrating live manifest rendering, field-level CEDS crosswalk annotations, and active Cedar policy protection.
                </p>
              </div>
              <span className="badge badge-blue">Host: bio-inventory.science.state.edu</span>
            </div>

            <ManifestRenderer
              manifest={sampleManifest}
              onSubmitRecord={(record) => {
                alert(`Record submitted successfully by ${activePersona.name}! Enriched with CEDS Element 000185.`);
              }}
            />
          </section>
        )}
      </main>
    </div>
  );
};
