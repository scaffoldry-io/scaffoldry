import React, { useState } from "react";
import { AppManifest, McpOverview, McpTool, McpResource, McpPrompt } from "./types";

interface Props {
  isOpen: boolean;
  onClose: () => void;
  onApplyAppProposal: (manifest: AppManifest) => void;
}

interface ChatMessage {
  id: string;
  sender: "user" | "assistant";
  timestamp: string;
  content: string;
  toolCall?: {
    tool: string;
    params: Record<string, unknown>;
    result: Record<string, unknown>;
  };
  proposalManifest?: AppManifest;
}

const SEEDED_MCP_OVERVIEW: McpOverview = {
  protocol: "Model Context Protocol (MCP)",
  protocol_version: "2024-11-05",
  server_info: {
    name: "scaffoldry-sovereign-mcp",
    version: "0.1.0",
    description: "Sovereign institutional decision, tabular dataset, and governance MCP server",
  },
  capabilities: {
    tools: {
      count: 9,
      items: [
        "list_datasets",
        "query_dataset",
        "create_app_proposal",
        "simulate_cedar_policy",
        "calculate_formula",
        "get_governance_posture",
        "record_governance_decision",
        "verify_decision_ledger",
        "export_oscal_compliance",
      ],
    },
    resources: {
      count: 4,
      uris: ["datasets://catalog", "policies://cedar", "compliance://oscal", "scaffoldry://governance/decision-ledger"],
    },
    prompts: {
      count: 2,
      items: ["governed_app_builder", "ferpa_risk_assessment"],
    },
  },
  live_metrics: {
    published_datasets: 4,
    dataset_relationships: 4,
    compliance_frameworks: ["NIST OSCAL 1.1.2", "FERPA 34 CFR § 99.30", "CEDS v11"],
  },
};

const SEEDED_TOOLS: McpTool[] = [
  {
    name: "list_datasets",
    description: "List all published institutional datasets with field schemas, department ownership, and FERPA classifications.",
    inputSchema: { type: "object", properties: { department: { type: "string" } } },
  },
  {
    name: "query_dataset",
    description: "Query tabular records from an authoritative institutional dataset (courses, faculty, grants).",
    inputSchema: { type: "object", properties: { dataset_id: { type: "string" }, limit: { type: "integer" } }, required: ["dataset_id"] },
  },
  {
    name: "create_app_proposal",
    description: "Formulate a new application manifest proposal with automatic FERPA scanning, CEDS element alignment, and Cedar policy binding.",
    inputSchema: { type: "object", properties: { title: { type: "string" }, department: { type: "string" } }, required: ["title", "department"] },
  },
  {
    name: "simulate_cedar_policy",
    description: "Evaluate Cedar ABAC authorization decisions for an action on institutional resources.",
    inputSchema: { type: "object", properties: { eppn: { type: "string" }, action: { type: "string" }, ferpa_sensitive: { type: "boolean" } }, required: ["eppn", "action"] },
  },
  {
    name: "calculate_formula",
    description: "Evaluate rollup and calculation expressions across tabular column values (SUM, AVERAGE, MIN, MAX, COUNT).",
    inputSchema: { type: "object", properties: { formula: { type: "string" }, values: { type: "array" } }, required: ["formula", "values"] },
  },
  {
    name: "get_governance_posture",
    description: "Retrieve NIST OSCAL 1.1.2 compliance metrics, control implementations, and active policy rules.",
    inputSchema: { type: "object", properties: {} },
  },
  {
    name: "record_governance_decision",
    description: "Append an approved governance decision to the immutable SHA-256 cryptographic ledger with NIST OSCAL control mapping.",
    inputSchema: { type: "object", properties: { principal: { type: "string" }, oscal_control_id: { type: "string" }, rationale: { type: "string" } }, required: ["principal", "rationale"] },
  },
  {
    name: "verify_decision_ledger",
    description: "Verify cryptographic integrity and SHA-256 block chain linkage of all recorded governance decisions from genesis.",
    inputSchema: { type: "object", properties: {} },
  },
  {
    name: "export_oscal_compliance",
    description: "Generate and export official NIST OSCAL 1.1.2 JSON component-definition with full cryptographic audit proofs.",
    inputSchema: { type: "object", properties: {} },
  },
];

const SEEDED_RESOURCES: McpResource[] = [
  {
    uri: "datasets://catalog",
    name: "Published Datasets Catalog",
    description: "Authoritative institutional dataset schemas and relationship lattice",
    mimeType: "application/json",
  },
  {
    uri: "policies://cedar",
    name: "Institutional Cedar Policies",
    description: "Active Attribute-Based Access Control policies in Cedar syntax",
    mimeType: "text/plain",
  },
  {
    uri: "compliance://oscal",
    name: "NIST OSCAL 1.1.2 Security Lattice",
    description: "System Security Plan controls and regulatory crosswalks",
    mimeType: "application/json",
  },
  {
    uri: "scaffoldry://governance/decision-ledger",
    name: "Cryptographic Decision Audit Ledger",
    description: "Append-only SHA-256 chained governance decision blocks",
    mimeType: "application/json",
  },
];

const SEEDED_PROMPTS: McpPrompt[] = [
  {
    name: "governed_app_builder",
    description: "Guide an AI assistant to formulate a compliant application schema adhering to CEDS v11 and Cedar access controls.",
    arguments: [
      { name: "app_purpose", description: "The business goal or departmental workflow", required: true },
      { name: "department", description: "Department or faculty unit", required: true },
    ],
  },
  {
    name: "ferpa_risk_assessment",
    description: "Audit an application or dataset schema for FERPA (34 CFR § 99.30) privacy compliance.",
    arguments: [{ name: "schema_json", description: "JSON representation of the schema", required: true }],
  },
];

export const AIAssistantDrawer: React.FC<Props> = ({ isOpen, onClose, onApplyAppProposal }) => {
  const [activeTab, setActiveTab] = useState<"chat" | "mcp-inspector">("chat");
  const [inputPrompt, setInputPrompt] = useState("");
  const [isProcessing, setIsProcessing] = useState(false);
  const [selectedTool, setSelectedTool] = useState<string>("simulate_cedar_policy");
  const [toolTestParam, setToolTestParam] = useState("view");
  const [toolTestResult, setToolTestResult] = useState<string | null>(null);

  const [messages, setMessages] = useState<ChatMessage[]>([
    {
      id: "msg-1",
      sender: "assistant",
      timestamp: "Just now",
      content:
        "Hello! I am your sovereign Co-Builder Assistant connected via Model Context Protocol (MCP). I can query published institutional datasets, formulate compliant application schemas, verify Cedar access policies, or compute rollups.",
    },
  ]);

  if (!isOpen) return null;

  const handleSendMessage = (textToSend?: string) => {
    const text = textToSend || inputPrompt;
    if (!text.trim()) return;

    const userMsg: ChatMessage = {
      id: `msg-${Date.now()}`,
      sender: "user",
      timestamp: new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }),
      content: text,
    };

    setMessages((prev) => [...prev, userMsg]);
    if (!textToSend) setInputPrompt("");
    setIsProcessing(true);

    setTimeout(() => {
      processPromptWithMcp(text);
      setIsProcessing(false);
    }, 450);
  };

  const processPromptWithMcp = (text: string) => {
    const lower = text.toLowerCase();
    const timestamp = new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

    // Scenario 1: Build Application Proposal
    if (lower.includes("build") || lower.includes("create") || lower.includes("app") || lower.includes("admissions")) {
      const generatedManifest: AppManifest = {
        slug: "physics-admissions-review",
        title: "Graduate Admissions & Fellowship Review",
        description: "Departmental applicant evaluation system with FERPA student privacy controls and faculty assignments.",
        organization_code: "UNIV",
        department: "Physics",
        herm_capability_id: "STU-02-ADMISS",
        custom_domain_verified: false,
        ceds_mappings: {
          applicant_id: "000033",
          applicant_name: "000115",
          gpa: "000147",
          reviewer_faculty: "000033",
        },
        views: [
          {
            id: "v-admissions-main",
            title: "Applicant Evaluation Form",
            view_type: "Form",
            fields: [
              { name: "applicant_id", label: "Student National ID", field_type: "Text", required: true, ferpa_sensitive: true },
              { name: "applicant_name", label: "Candidate Full Name", field_type: "Text", required: true, ferpa_sensitive: true },
              { name: "undergrad_institution", label: "Undergraduate Institution", field_type: "Text", required: true, ferpa_sensitive: false },
              { name: "gpa", label: "Cumulative Undergraduate GPA", field_type: "Number", required: true, ferpa_sensitive: true },
              {
                name: "faculty_reviewer",
                label: "Assigned Faculty Reviewer",
                field_type: "Relation",
                required: true,
                ferpa_sensitive: false,
                linked_dataset_id: "faculty",
                linked_field: "full_name",
              },
              { name: "admission_recommendation", label: "Committee Recommendation", field_type: "Select", required: true, ferpa_sensitive: false },
            ],
          },
        ],
      };

      const assistantMsg: ChatMessage = {
        id: `msg-${Date.now()}`,
        sender: "assistant",
        timestamp,
        content:
          "I formulated a compliant application manifest through the MCP tool `create_app_proposal`. All student records carry automated 34 CFR § 99.30 FERPA guardrails, and the faculty reviewer is linked directly to the published Faculty Directory.",
        toolCall: {
          tool: "create_app_proposal",
          params: { title: generatedManifest.title, department: generatedManifest.department },
          result: {
            proposal_branch: "proposal/physics-admissions-review",
            status: "StagedForReview",
            ferpa_scan: "Passed (3 protected fields)",
            cedar_policy: "Enforced",
          },
        },
        proposalManifest: generatedManifest,
      };

      setMessages((prev) => [...prev, assistantMsg]);
      return;
    }

    // Scenario 2: FERPA Privacy Audit
    if (lower.includes("ferpa") || lower.includes("audit") || lower.includes("policy") || lower.includes("privacy")) {
      const assistantMsg: ChatMessage = {
        id: `msg-${Date.now()}`,
        sender: "assistant",
        timestamp,
        content:
          "Executed MCP policy evaluation `simulate_cedar_policy` against institutional Cedar policies. Data export requests on FERPA-sensitive fields are strictly forbidden for non-compliance affiliations.",
        toolCall: {
          tool: "simulate_cedar_policy",
          params: { eppn: "dr.smith@university.edu", action: "export", ferpa_sensitive: true },
          result: {
            decision: "Deny",
            governing_policy: "policy-ferpa-34cfr99",
            reason: "Principal lacks registrar@university.edu or compliance@university.edu affiliation token.",
          },
        },
      };
      setMessages((prev) => [...prev, assistantMsg]);
      return;
    }

    // Scenario 3: Query Published Datasets
    if (lower.includes("query") || lower.includes("faculty") || lower.includes("dataset") || lower.includes("catalog")) {
      const assistantMsg: ChatMessage = {
        id: `msg-${Date.now()}`,
        sender: "assistant",
        timestamp,
        content:
          "Executed MCP tool `query_dataset` for dataset 'faculty'. Retrieved 3 authoritative records from Academic Affairs.",
        toolCall: {
          tool: "query_dataset",
          params: { dataset_id: "faculty", limit: 3 },
          result: {
            records: [
              { eppn: "dr.smith@university.edu", full_name: "Dr. Sarah Smith", title: "Professor", department: "Physics" },
              { eppn: "dr.curie@science.state.edu", full_name: "Dr. Marie Curie", title: "Distinguished Professor", department: "Biology" },
              { eppn: "dr.alan@university.edu", full_name: "Dr. Alan Turing", title: "Chair Professor", department: "Computer Science" },
            ],
          },
        },
      };
      setMessages((prev) => [...prev, assistantMsg]);
      return;
    }

    // Scenario 4: Calculate Rollups / Formulas
    if (lower.includes("calculate") || lower.includes("sum") || lower.includes("budget") || lower.includes("formula")) {
      const assistantMsg: ChatMessage = {
        id: `msg-${Date.now()}`,
        sender: "assistant",
        timestamp,
        content:
          "Executed MCP tool `calculate_formula` with operator SUM over active grant award amounts. Result: $1,950,000 across 2 active projects.",
        toolCall: {
          tool: "calculate_formula",
          params: { formula: "SUM", values: [750000, 1200000] },
          result: { formula: "SUM", input_count: 2, result: 1950000 },
        },
      };
      setMessages((prev) => [...prev, assistantMsg]);
      return;
    }

    // Default conversational reply
    const assistantMsg: ChatMessage = {
      id: `msg-${Date.now()}`,
      sender: "assistant",
      timestamp,
      content:
        `Received prompt: "${text}". You can ask me to build a departmental application, audit FERPA sensitivity, query published institutional datasets, or execute calculation rollups.`,
      toolCall: {
        tool: "get_governance_posture",
        params: {},
        result: { framework: "NIST OSCAL 1.1.2", compliance_score: "98%", status: "Active" },
      },
    };
    setMessages((prev) => [...prev, assistantMsg]);
  };

  const handleRunToolTest = () => {
    if (selectedTool === "simulate_cedar_policy") {
      const allowed = toolTestParam === "view" || toolTestParam === "list";
      setToolTestResult(
        JSON.stringify(
          {
            tool: "simulate_cedar_policy",
            action: toolTestParam,
            decision: allowed ? "Allow" : "Deny",
            governing_policy: "policy-ferpa-34cfr99",
            timestamp: new Date().toISOString(),
          },
          null,
          2
        )
      );
    } else if (selectedTool === "calculate_formula") {
      setToolTestResult(
        JSON.stringify(
          {
            tool: "calculate_formula",
            formula: "AVERAGE",
            values: [85.5, 92.0, 78.5, 94.0],
            result: 87.5,
          },
          null,
          2
        )
      );
    } else if (selectedTool === "list_datasets") {
      setToolTestResult(
        JSON.stringify(
          {
            tool: "list_datasets",
            total: 4,
            datasets: ["courses", "faculty", "programs", "grants"],
          },
          null,
          2
        )
      );
    } else {
      setToolTestResult(
        JSON.stringify(
          {
            tool: selectedTool,
            status: "Executed via JSON-RPC 2.0",
            protocolVersion: "2024-11-05",
          },
          null,
          2
        )
      );
    }
  };

  return (
    <div className="fixed inset-y-0 right-0 z-50 w-full max-w-xl bg-white dark:bg-slate-900 border-l border-slate-200 dark:border-slate-800 shadow-2xl flex flex-col transition-all duration-200 ease-in-out">
      {/* Header */}
      <div className="px-5 py-4 border-b border-slate-200 dark:border-slate-800 bg-slate-50/80 dark:bg-slate-900/80 flex items-center justify-between">
        <div className="flex items-center gap-2.5">
          <div className="w-8 h-8 rounded-lg bg-gradient-to-tr from-purple-600 to-indigo-600 flex items-center justify-center text-white text-base shadow-sm">
            ✨
          </div>
          <div>
            <div className="flex items-center gap-2">
              <h2 className="text-sm font-bold text-slate-900 dark:text-white">
                Sovereign AI Assistant
              </h2>
              <span className="px-1.5 py-0.5 rounded text-[10px] font-mono bg-purple-100 text-purple-800 dark:bg-purple-950 dark:text-purple-300 font-bold border border-purple-200 dark:border-purple-800">
                MCP Native
              </span>
            </div>
            <p className="text-[11px] text-slate-500 dark:text-slate-400">
              Model Context Protocol (JSON-RPC 2.0) · Institutional Engine
            </p>
          </div>
        </div>

        <button
          type="button"
          onClick={onClose}
          className="p-1.5 rounded-lg text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 cursor-pointer transition-colors"
          title="Close Assistant"
        >
          <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      {/* Tabs */}
      <div className="flex border-b border-slate-200 dark:border-slate-800 px-5 bg-white dark:bg-slate-900 text-xs">
        <button
          type="button"
          onClick={() => setActiveTab("chat")}
          className={`py-2.5 px-3 border-b-2 font-semibold cursor-pointer transition-colors ${
            activeTab === "chat"
              ? "border-purple-600 text-purple-600 dark:text-purple-400"
              : "border-transparent text-slate-500 hover:text-slate-700 dark:hover:text-slate-300"
          }`}
        >
          Co-Builder Chat
        </button>
        <button
          type="button"
          onClick={() => setActiveTab("mcp-inspector")}
          className={`py-2.5 px-3 border-b-2 font-semibold cursor-pointer transition-colors flex items-center gap-1.5 ${
            activeTab === "mcp-inspector"
              ? "border-purple-600 text-purple-600 dark:text-purple-400"
              : "border-transparent text-slate-500 hover:text-slate-700 dark:hover:text-slate-300"
          }`}
        >
          <span>MCP Registry</span>
          <span className="px-1.5 py-0.2 rounded-full bg-slate-100 dark:bg-slate-800 text-[10px]">
            {SEEDED_MCP_OVERVIEW.capabilities.tools.count}
          </span>
        </button>
      </div>

      {/* Tab Content: Co-Builder Chat */}
      {activeTab === "chat" && (
        <div className="flex-1 flex flex-col overflow-hidden">
          {/* Messages scroll area */}
          <div className="flex-1 overflow-y-auto p-5 space-y-4">
            {messages.map((msg) => (
              <div
                key={msg.id}
                className={`flex flex-col ${msg.sender === "user" ? "items-end" : "items-start"}`}
              >
                <div
                  className={`max-w-[88%] rounded-xl p-3.5 text-xs leading-relaxed ${
                    msg.sender === "user"
                      ? "bg-purple-600 text-white rounded-br-none"
                      : "bg-slate-100 dark:bg-slate-800 text-slate-800 dark:text-slate-200 rounded-bl-none border border-slate-200 dark:border-slate-700/60"
                  }`}
                >
                  <p>{msg.content}</p>

                  {/* Collapsible MCP Tool Trace */}
                  {msg.toolCall && (
                    <div className="mt-3 pt-2.5 border-t border-slate-200 dark:border-slate-700 text-[11px] font-mono">
                      <div className="flex items-center gap-1 text-purple-700 dark:text-purple-400 font-semibold mb-1">
                        <span>⚡ MCP Tool Executed:</span>
                        <code>{msg.toolCall.tool}</code>
                      </div>
                      <details className="mt-1 bg-white/70 dark:bg-slate-950/60 p-2 rounded border border-slate-200 dark:border-slate-800">
                        <summary className="cursor-pointer text-slate-500 hover:text-slate-700 dark:hover:text-slate-300 select-none">
                          View JSON payload & result
                        </summary>
                        <pre className="mt-2 text-[10px] overflow-x-auto text-slate-700 dark:text-slate-300">
                          {JSON.stringify(msg.toolCall, null, 2)}
                        </pre>
                      </details>
                    </div>
                  )}

                  {/* Staged Proposal Card */}
                  {msg.proposalManifest && (
                    <div className="mt-3 p-3 rounded-lg bg-purple-50 dark:bg-purple-950/40 border border-purple-200 dark:border-purple-800 text-slate-800 dark:text-slate-200">
                      <div className="flex items-center justify-between">
                        <div className="font-semibold text-purple-900 dark:text-purple-300">
                          {msg.proposalManifest.title}
                        </div>
                        <span className="px-1.5 py-0.5 rounded text-[10px] font-mono bg-purple-200 dark:bg-purple-900 text-purple-800 dark:text-purple-200">
                          {msg.proposalManifest.department}
                        </span>
                      </div>
                      <p className="text-[11px] text-slate-600 dark:text-slate-400 mt-1">
                        {msg.proposalManifest.description}
                      </p>

                      <div className="mt-2 text-[11px] text-slate-500 dark:text-slate-400">
                        Fields: {msg.proposalManifest.views[0]?.fields?.length || 0} inputs · FERPA sensitive fields guarded
                      </div>

                      <button
                        type="button"
                        onClick={() => onApplyAppProposal(msg.proposalManifest!)}
                        className="mt-3 w-full py-1.5 px-3 rounded bg-purple-600 hover:bg-purple-700 text-white font-semibold text-xs cursor-pointer transition-colors shadow-sm"
                      >
                        ✓ Install Application to Workspace
                      </button>
                    </div>
                  )}
                </div>
                <span className="text-[10px] text-slate-400 mt-1 px-1">{msg.timestamp}</span>
              </div>
            ))}

            {isProcessing && (
              <div className="flex items-center gap-2 text-xs text-slate-400">
                <div className="w-2 h-2 rounded-full bg-purple-600 animate-ping" />
                <span>Consulting sovereign MCP tools & policies...</span>
              </div>
            )}
          </div>

          {/* Prompt quick suggestions */}
          <div className="px-4 py-2 border-t border-slate-200 dark:border-slate-800 bg-slate-50/50 dark:bg-slate-900/50 flex flex-wrap gap-1.5">
            <button
              type="button"
              onClick={() => handleSendMessage("Build Graduate Admissions Review app for Physics")}
              className="px-2.5 py-1 rounded-full text-[11px] font-medium bg-white dark:bg-slate-800 text-slate-700 dark:text-slate-300 border border-slate-200 dark:border-slate-700 hover:border-purple-400 cursor-pointer transition-colors"
            >
              🛠️ Build Admissions Review App
            </button>
            <button
              type="button"
              onClick={() => handleSendMessage("Audit FERPA sensitivity on student data")}
              className="px-2.5 py-1 rounded-full text-[11px] font-medium bg-white dark:bg-slate-800 text-slate-700 dark:text-slate-300 border border-slate-200 dark:border-slate-700 hover:border-purple-400 cursor-pointer transition-colors"
            >
              🔒 Audit FERPA Policy
            </button>
            <button
              type="button"
              onClick={() => handleSendMessage("Query faculty researchers")}
              className="px-2.5 py-1 rounded-full text-[11px] font-medium bg-white dark:bg-slate-800 text-slate-700 dark:text-slate-300 border border-slate-200 dark:border-slate-700 hover:border-purple-400 cursor-pointer transition-colors"
            >
              📊 Query Faculty Directory
            </button>
            <button
              type="button"
              onClick={() => handleSendMessage("Calculate grant budget totals")}
              className="px-2.5 py-1 rounded-full text-[11px] font-medium bg-white dark:bg-slate-800 text-slate-700 dark:text-slate-300 border border-slate-200 dark:border-slate-700 hover:border-purple-400 cursor-pointer transition-colors"
            >
              🧮 Compute Grant Totals
            </button>
          </div>

          {/* Input Box */}
          <div className="p-4 border-t border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-900">
            <form
              onSubmit={(e) => {
                e.preventDefault();
                handleSendMessage();
              }}
              className="flex items-center gap-2"
            >
              <input
                type="text"
                value={inputPrompt}
                onChange={(e) => setInputPrompt(e.target.value)}
                placeholder="Ask Co-Builder to create an app, check policies, or query data..."
                className="flex-1 px-3.5 py-2 text-xs rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-950 text-slate-900 dark:text-white placeholder-slate-400 focus:outline-none focus:ring-2 focus:ring-purple-500"
              />
              <button
                type="submit"
                disabled={isProcessing || !inputPrompt.trim()}
                className="px-4 py-2 rounded-lg bg-purple-600 hover:bg-purple-700 disabled:opacity-50 text-white font-semibold text-xs cursor-pointer transition-colors shadow-sm"
              >
                Send
              </button>
            </form>
          </div>
        </div>
      )}

      {/* Tab Content: MCP Inspector */}
      {activeTab === "mcp-inspector" && (
        <div className="flex-1 overflow-y-auto p-5 space-y-6 text-xs">
          {/* Server Info Card */}
          <div className="p-4 rounded-xl border border-slate-200 dark:border-slate-800 bg-slate-50 dark:bg-slate-800/40">
            <div className="flex items-center justify-between">
              <div>
                <h3 className="font-bold text-slate-900 dark:text-white">
                  {SEEDED_MCP_OVERVIEW.server_info.name}
                </h3>
                <p className="text-[11px] text-slate-500 dark:text-slate-400 mt-0.5">
                  {SEEDED_MCP_OVERVIEW.server_info.description}
                </p>
              </div>
              <span className="px-2 py-0.5 rounded text-[10px] font-mono bg-emerald-100 text-emerald-800 dark:bg-emerald-950 dark:text-emerald-300 font-bold">
                Online
              </span>
            </div>
            <div className="grid grid-cols-3 gap-2 mt-4 pt-3 border-t border-slate-200 dark:border-slate-700/60 font-mono text-[11px]">
              <div>
                <span className="text-slate-400">Protocol:</span>
                <div className="font-semibold text-slate-800 dark:text-slate-200">
                  {SEEDED_MCP_OVERVIEW.protocol_version}
                </div>
              </div>
              <div>
                <span className="text-slate-400">Tools:</span>
                <div className="font-semibold text-purple-600 dark:text-purple-400">
                  {SEEDED_MCP_OVERVIEW.capabilities.tools.count} Active
                </div>
              </div>
              <div>
                <span className="text-slate-400">Compliance:</span>
                <div className="font-semibold text-emerald-600 dark:text-emerald-400">
                  OSCAL 1.1.2
                </div>
              </div>
            </div>
          </div>

          {/* Tools List */}
          <div>
            <h4 className="font-bold text-slate-900 dark:text-white mb-2 flex items-center gap-1.5">
              <span>🛠️ Exposed MCP Tools</span>
              <span className="text-slate-400 font-normal">({SEEDED_TOOLS.length})</span>
            </h4>
            <div className="space-y-2">
              {SEEDED_TOOLS.map((t) => (
                <div
                  key={t.name}
                  className="p-3 rounded-lg border border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-900"
                >
                  <div className="flex items-center justify-between">
                    <span className="font-mono font-semibold text-purple-700 dark:text-purple-300">
                      {t.name}
                    </span>
                    <span className="text-[10px] font-mono text-slate-400">JSON-RPC 2.0</span>
                  </div>
                  <p className="text-[11px] text-slate-600 dark:text-slate-400 mt-1">
                    {t.description}
                  </p>
                </div>
              ))}
            </div>
          </div>

          {/* Interactive Tool Runner */}
          <div className="p-4 rounded-xl border border-purple-200 dark:border-purple-800/80 bg-purple-50/40 dark:bg-purple-950/20">
            <h4 className="font-bold text-slate-900 dark:text-white mb-2">
              ⚡ Execute Tool Test
            </h4>
            <div className="space-y-3">
              <div className="flex gap-2">
                <select
                  value={selectedTool}
                  onChange={(e) => setSelectedTool(e.target.value)}
                  className="flex-1 px-3 py-1.5 text-xs rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-900 text-slate-900 dark:text-white"
                >
                  {SEEDED_TOOLS.map((t) => (
                    <option key={t.name} value={t.name}>
                      {t.name}
                    </option>
                  ))}
                </select>

                {selectedTool === "simulate_cedar_policy" && (
                  <select
                    value={toolTestParam}
                    onChange={(e) => setToolTestParam(e.target.value)}
                    className="px-3 py-1.5 text-xs rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-900 text-slate-900 dark:text-white"
                  >
                    <option value="view">Action: view</option>
                    <option value="list">Action: list</option>
                    <option value="export">Action: export (FERPA gated)</option>
                  </select>
                )}

                <button
                  type="button"
                  onClick={handleRunToolTest}
                  className="px-3 py-1.5 rounded bg-purple-600 hover:bg-purple-700 text-white font-semibold text-xs cursor-pointer transition-colors"
                >
                  Run
                </button>
              </div>

              {toolTestResult && (
                <div className="mt-2">
                  <span className="text-[11px] font-semibold text-slate-500 dark:text-slate-400">
                    Result:
                  </span>
                  <pre className="mt-1 p-2 rounded bg-slate-900 text-purple-300 font-mono text-[10px] overflow-x-auto">
                    {toolTestResult}
                  </pre>
                </div>
              )}
            </div>
          </div>

          {/* Resources List */}
          <div>
            <h4 className="font-bold text-slate-900 dark:text-white mb-2 flex items-center gap-1.5">
              <span>📦 Registered MCP Resources</span>
              <span className="text-slate-400 font-normal">({SEEDED_RESOURCES.length})</span>
            </h4>
            <div className="space-y-2">
              {SEEDED_RESOURCES.map((r) => (
                <div
                  key={r.uri}
                  className="p-3 rounded-lg border border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-900"
                >
                  <div className="flex items-center justify-between">
                    <span className="font-mono font-semibold text-blue-600 dark:text-blue-400">
                      {r.uri}
                    </span>
                    <span className="text-[10px] font-mono text-slate-400">{r.mimeType}</span>
                  </div>
                  <p className="text-[11px] text-slate-600 dark:text-slate-400 mt-1">
                    {r.description}
                  </p>
                </div>
              ))}
            </div>
          </div>

          {/* Prompts List */}
          <div>
            <h4 className="font-bold text-slate-900 dark:text-white mb-2 flex items-center gap-1.5">
              <span>💬 Prompt Templates</span>
              <span className="text-slate-400 font-normal">({SEEDED_PROMPTS.length})</span>
            </h4>
            <div className="space-y-2">
              {SEEDED_PROMPTS.map((p) => (
                <div
                  key={p.name}
                  className="p-3 rounded-lg border border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-900"
                >
                  <span className="font-mono font-semibold text-emerald-600 dark:text-emerald-400">
                    {p.name}
                  </span>
                  <p className="text-[11px] text-slate-600 dark:text-slate-400 mt-1">
                    {p.description}
                  </p>
                </div>
              ))}
            </div>
          </div>
        </div>
      )}
    </div>
  );
};
