import React, { useState, useEffect, useRef } from "react";
import { ManifestRenderer } from "./ManifestRenderer";
import { AppManifest, Persona, RegisteredApp, SourceRule } from "./types";

const PERSONAS: Persona[] = [
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
    status: "Published",
    updatedAt: "2026-10-02",
    recordsCount: 38,
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
  },
];

const sampleManifest: AppManifest = {
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
  // Discreet URL Path Routing State
  const [currentPath, setCurrentPath] = useState<string>(() => {
    if (typeof window !== "undefined") {
      return window.location.pathname;
    }
    return "/";
  });

  const isAdminPath = currentPath === "/admin" || currentPath.startsWith("/admin/");

  // Navigation & View State
  const [activeTab, setActiveTab] = useState<"policy" | "apps" | "exemplar" | "admin" | "cloud">(() => {
    if (typeof window !== "undefined" && (window.location.pathname === "/admin" || window.location.pathname.startsWith("/admin/"))) {
      return "admin";
    }
    return "apps";
  });

  const [navRailExpanded, setNavRailExpanded] = useState<boolean>(true);
  const [userMenuOpen, setUserMenuOpen] = useState<boolean>(false);
  const userMenuRef = useRef<HTMLDivElement>(null);

  const [darkMode, setDarkMode] = useState<boolean>(() => {
    if (typeof window !== "undefined") {
      return localStorage.getItem("scaffoldry-theme") === "dark" ||
        (!localStorage.getItem("scaffoldry-theme") && window.matchMedia("(prefers-color-scheme: dark)").matches);
    }
    return true;
  });

  // Persona State
  const [activePersona, setActivePersona] = useState<Persona>(PERSONAS[0]);
  const [notificationToast, setNotificationToast] = useState<string | null>(null);

  // Data & Grid States
  const [apps, setApps] = useState<RegisteredApp[]>(INITIAL_APPS);
  const [sourceRules] = useState<SourceRule[]>(INITIAL_SOURCE_RULES);
  const [selectedApp, setSelectedApp] = useState<RegisteredApp | null>(null);
  const [selectedRule, setSelectedRule] = useState<SourceRule | null>(null);
  const [inspectorOpen, setInspectorOpen] = useState<boolean>(false);
  const [searchQuery, setSearchQuery] = useState<string>("");
  const [deptFilter, setDeptFilter] = useState<string>("all");
  const [statusFilter, setStatusFilter] = useState<string>("all");
  const [viewMode, setViewMode] = useState<"grid" | "cards" | "oscal">("grid");

  // Simulator State
  const [simAction, setSimAction] = useState<"read" | "write" | "export">("export");
  const [simFerpa, setSimFerpa] = useState<boolean>(true);

  // Sync browser URL via HTML5 History API
  const navigateTo = (path: string) => {
    if (typeof window !== "undefined") {
      window.history.pushState(null, "", path);
      setCurrentPath(path);
      if (path === "/admin" || path.startsWith("/admin/")) {
        setActiveTab("admin");
      } else if (activeTab === "admin") {
        setActiveTab("apps");
      }
    }
  };

  // Popstate listener for browser back/forward buttons
  useEffect(() => {
    const handlePopState = () => {
      const p = window.location.pathname;
      setCurrentPath(p);
      if (p === "/admin" || p.startsWith("/admin/")) {
        setActiveTab("admin");
      }
    };
    window.addEventListener("popstate", handlePopState);
    return () => window.removeEventListener("popstate", handlePopState);
  }, []);

  // Sync dark mode class with DOM
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

  // Click outside to close user menu
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

  // Evaluate Cedar Decision for simulator or record inspector
  const evaluateCedarDecision = (targetDept: string, isFerpaSensitive: boolean, action: "read" | "write" | "export") => {
    // Cross department boundary denial
    if (activePersona.department !== targetDept && activePersona.department !== "compliance") {
      return {
        decision: "DENY" as const,
        reason: "Boundary Isolation Policy: Principal department does not match resource realm",
        rule: "rule-nist-ac3",
      };
    }

    // FERPA export guard
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

  // Filtered applications
  const filteredApps = apps.filter((app) => {
    const matchesSearch =
      app.title.toLowerCase().includes(searchQuery.toLowerCase()) ||
      app.slug.toLowerCase().includes(searchQuery.toLowerCase()) ||
      app.customDomain.toLowerCase().includes(searchQuery.toLowerCase()) ||
      app.orgCode.toLowerCase().includes(searchQuery.toLowerCase());
    const matchesDept = deptFilter === "all" || app.department === deptFilter;
    const matchesStatus = statusFilter === "all" || app.status === statusFilter;
    return matchesSearch && matchesDept && matchesStatus;
  });

  const handleSelectApp = (app: RegisteredApp) => {
    setSelectedApp(app);
    setSelectedRule(null);
    setInspectorOpen(true);
  };

  const handleSelectRule = (rule: SourceRule) => {
    setSelectedRule(rule);
    setSelectedApp(null);
    setInspectorOpen(true);
  };

  const handleUpdateSelectedApp = (updated: RegisteredApp) => {
    setApps((prev) => prev.map((a) => (a.slug === updated.slug ? updated : a)));
    setSelectedApp(updated);
    showToast(`Updated "${updated.title}" successfully.`);
  };

  return (
    <div className="min-h-screen bg-slate-50 dark:bg-slate-950 text-slate-900 dark:text-slate-100 flex flex-col font-sans transition-colors duration-200">
      {/* Toast Notification */}
      {notificationToast && (
        <div className="fixed bottom-6 right-6 z-50 flex items-center gap-2 px-4 py-3 rounded-lg shadow-lg bg-slate-900 text-white dark:bg-white dark:text-slate-900 text-sm font-medium border border-slate-700 animate-fade-in">
          <span className="text-emerald-400 dark:text-emerald-600">✓</span>
          {notificationToast}
        </div>
      )}

      {/* TOP GLOBAL COMMAND BAR */}
      <header className="sticky top-0 z-40 h-14 bg-white dark:bg-slate-900 border-b border-slate-200 dark:border-slate-800 px-4 flex items-center justify-between shadow-xs">
        {/* Left: Brand & Realm Selector */}
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

          <div className="flex items-center gap-2">
            <span className="font-mono text-xs font-bold px-2 py-0.5 rounded bg-blue-600 text-white tracking-wider">
              SCAFFOLDRY
            </span>
            <div className="h-4 w-px bg-slate-200 dark:bg-slate-800 hidden sm:block" />
            <button
              type="button"
              onClick={() => navigateTo("/")}
              className="text-sm font-semibold tracking-tight text-slate-800 dark:text-slate-100 hover:text-blue-600 dark:hover:text-blue-400 hidden sm:inline cursor-pointer"
            >
              The Sovereign Desk
            </button>
            {isAdminPath && (
              <span className="text-[11px] font-mono px-2 py-0.5 rounded bg-amber-500/10 text-amber-700 dark:text-amber-400 border border-amber-500/30">
                /admin
              </span>
            )}
          </div>

          <div className="hidden lg:flex items-center gap-1.5 ml-3 px-2 py-1 rounded-md bg-slate-100 dark:bg-slate-800/80 text-xs text-slate-600 dark:text-slate-300 border border-slate-200 dark:border-slate-700/60">
            <span className="w-2 h-2 rounded-full bg-emerald-500" />
            <span>State University (IPEDS 234076)</span>
            <span className="text-slate-400">/</span>
            <span className="font-mono font-medium text-blue-600 dark:text-blue-400">science.state.edu</span>
          </div>
        </div>

        {/* Center: Search Command Bar */}
        <div className="flex-1 max-w-md mx-4 hidden md:block">
          <div className="relative">
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder="Search apps, rules, CEDS elements, DNS aliases... (/)"
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

        {/* Right: Dark/Light Mode & User Badge with Discreet Menu */}
        <div className="flex items-center gap-2">
          {/* Dark/Light Mode Switcher */}
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

          {/* USER BADGE WITH DISCREET DROPDOWN MENU */}
          <div className="relative" ref={userMenuRef}>
            <button
              type="button"
              onClick={() => setUserMenuOpen(!userMenuOpen)}
              className="flex items-center gap-2 p-1.5 rounded-lg border border-slate-200 dark:border-slate-800 hover:bg-slate-100 dark:hover:bg-slate-800/80 cursor-pointer transition-colors"
              title="User Account & Persona Menu"
            >
              <div className="w-6 h-6 rounded-full bg-blue-600 text-white font-bold text-xs flex items-center justify-center uppercase shrink-0">
                {activePersona.name.split(" ").map(n => n[0]).slice(0, 2).join("")}
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

            {/* User Dropdown Menu */}
            {userMenuOpen && (
              <div className="absolute right-0 mt-2 w-72 bg-white dark:bg-slate-900 rounded-lg shadow-xl border border-slate-200 dark:border-slate-800 p-2 z-50 animate-fade-in text-xs">
                {/* Profile Header */}
                <div className="p-2 border-b border-slate-100 dark:border-slate-800">
                  <div className="font-semibold text-slate-900 dark:text-white">{activePersona.name}</div>
                  <div className="text-[11px] font-mono text-slate-500 dark:text-slate-400 break-all">{activePersona.eppn}</div>
                  <div className="mt-1 inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-medium bg-blue-50 text-blue-700 dark:bg-blue-950/60 dark:text-blue-300">
                    {activePersona.roleTitle}
                  </div>
                </div>

                {/* Identity Switcher */}
                <div className="py-2 border-b border-slate-100 dark:border-slate-800">
                  <div className="text-[10px] uppercase font-bold tracking-wider text-slate-400 px-2 mb-1.5">
                    Switch InCommon Identity
                  </div>
                  <div className="space-y-1">
                    {PERSONAS.map((p) => (
                      <button
                        key={p.eppn}
                        type="button"
                        onClick={() => {
                          setActivePersona(p);
                          showToast(`Switched active identity to ${p.name}`);
                          setUserMenuOpen(false);
                        }}
                        className={`w-full text-left px-2 py-1.5 rounded flex items-center justify-between transition-colors cursor-pointer ${
                          p.eppn === activePersona.eppn
                            ? "bg-slate-100 dark:bg-slate-800 text-blue-600 dark:text-blue-400 font-semibold"
                            : "hover:bg-slate-50 dark:hover:bg-slate-800/50 text-slate-700 dark:text-slate-300"
                        }`}
                      >
                        <div className="truncate">
                          <div>{p.name}</div>
                          <div className="text-[10px] text-slate-400">{p.affiliation} · {p.department}</div>
                        </div>
                        {p.eppn === activePersona.eppn && (
                          <span className="text-blue-600 dark:text-blue-400">✓</span>
                        )}
                      </button>
                    ))}
                  </div>
                </div>

                {/* DISCREET ADMIN PATH ENTRY */}
                <div className="pt-2">
                  {!isAdminPath ? (
                    <button
                      type="button"
                      onClick={() => {
                        setUserMenuOpen(false);
                        navigateTo("/admin");
                        showToast("Navigated to discreet path: /admin");
                      }}
                      className="w-full text-left px-2 py-2 rounded-md hover:bg-amber-50 dark:hover:bg-amber-950/40 text-amber-800 dark:text-amber-300 transition-colors flex items-center gap-2.5 cursor-pointer"
                    >
                      <svg className="w-4 h-4 text-amber-500 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
                      </svg>
                      <div>
                        <div className="font-semibold text-xs">Institutional Admin Console</div>
                        <div className="text-[10px] text-amber-600/80 dark:text-amber-400/80 font-mono">
                          Discreet path: /admin
                        </div>
                      </div>
                    </button>
                  ) : (
                    <button
                      type="button"
                      onClick={() => {
                        setUserMenuOpen(false);
                        navigateTo("/");
                        showToast("Returned to General Desk");
                      }}
                      className="w-full text-left px-2 py-2 rounded-md hover:bg-slate-100 dark:hover:bg-slate-800 text-slate-700 dark:text-slate-300 transition-colors flex items-center gap-2.5 cursor-pointer"
                    >
                      <svg className="w-4 h-4 text-slate-400 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M10 19l-7-7m0 0l7-7m-7 7h18" />
                      </svg>
                      <div>
                        <div className="font-semibold text-xs">Return to General Desk</div>
                        <div className="text-[10px] text-slate-400 font-mono">
                          Path: /
                        </div>
                      </div>
                    </button>
                  )}
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
            {/* If on /admin, show dedicated admin rail */}
            {isAdminPath ? (
              <div>
                {navRailExpanded && (
                  <div className="text-[11px] font-semibold uppercase tracking-wider text-amber-600 dark:text-amber-400 px-2 mb-2 flex items-center gap-1.5">
                    <span className="w-2 h-2 rounded-full bg-amber-500 animate-pulse" />
                    <span>Admin Operations (/admin)</span>
                  </div>
                )}
                <nav className="space-y-1">
                  <button
                    type="button"
                    onClick={() => setActiveTab("admin")}
                    className={`w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-xs font-medium transition-colors cursor-pointer ${
                      activeTab === "admin"
                        ? "bg-amber-50 text-amber-800 dark:bg-amber-950/40 dark:text-amber-300 font-semibold"
                        : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
                    }`}
                  >
                    <svg className="w-4 h-4 shrink-0 text-amber-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M12 6V4m0 2a2 2 0 100 4m0-4a2 2 0 110 4m-6 8a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4m6 6v10m6-2a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4" />
                    </svg>
                    {navRailExpanded && <span>Control Desk Overview</span>}
                  </button>

                  <button
                    type="button"
                    onClick={() => setActiveTab("cloud")}
                    className={`w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-xs font-medium transition-colors cursor-pointer ${
                      activeTab === "cloud"
                        ? "bg-amber-50 text-amber-800 dark:bg-amber-950/40 dark:text-amber-300 font-semibold"
                        : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
                    }`}
                  >
                    <svg className="w-4 h-4 shrink-0 text-amber-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M3 15a4 4 0 004 4h9a5 5 0 10-.1-9.999 5.002 5.002 0 00-9.78 2.096A4.001 4.001 0 003 15z" />
                    </svg>
                    {navRailExpanded && <span>Cloud Run Topology</span>}
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
                      {navRailExpanded && <span>Return to Desk (/)</span>}
                    </button>
                  </div>
                </nav>
              </div>
            ) : (
              /* Standard Desk Rail */
              <>
                <div>
                  {navRailExpanded && (
                    <div className="text-[11px] font-semibold uppercase tracking-wider text-slate-400 dark:text-slate-500 px-2 mb-2">
                      Governance &amp; Policy
                    </div>
                  )}
                  <nav className="space-y-1">
                    <button
                      type="button"
                      onClick={() => setActiveTab("apps")}
                      className={`w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-xs font-medium transition-colors cursor-pointer ${
                        activeTab === "apps"
                          ? "bg-blue-50 text-blue-700 dark:bg-blue-900/30 dark:text-blue-300 font-semibold"
                          : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
                      }`}
                      title="Applications & DNS Registry"
                    >
                      <svg className="w-4 h-4 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M4 6a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2H6a2 2 0 01-2-2V6zM14 6a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2h-2a2 2 0 01-2-2V6zM4 16a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2H6a2 2 0 01-2-2v-2zM14 16a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2h-2a2 2 0 01-2-2v-2z" />
                      </svg>
                      {navRailExpanded && (
                        <span className="flex-1 text-left flex items-center justify-between">
                          <span>App Registry &amp; DNS</span>
                          <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-slate-100 dark:bg-slate-800 text-slate-500">
                            {apps.length}
                          </span>
                        </span>
                      )}
                    </button>

                    <button
                      type="button"
                      onClick={() => setActiveTab("policy")}
                      className={`w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-xs font-medium transition-colors cursor-pointer ${
                        activeTab === "policy"
                          ? "bg-blue-50 text-blue-700 dark:bg-blue-900/30 dark:text-blue-300 font-semibold"
                          : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
                      }`}
                      title="Source Rules & Policy Simulator"
                    >
                      <svg className="w-4 h-4 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
                      </svg>
                      {navRailExpanded && (
                        <span className="flex-1 text-left flex items-center justify-between">
                          <span>Source Rules &amp; OSCAL</span>
                          <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-slate-100 dark:bg-slate-800 text-slate-500">
                            {sourceRules.length}
                          </span>
                        </span>
                      )}
                    </button>

                    <button
                      type="button"
                      onClick={() => setActiveTab("exemplar")}
                      className={`w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-xs font-medium transition-colors cursor-pointer ${
                        activeTab === "exemplar"
                          ? "bg-blue-50 text-blue-700 dark:bg-blue-900/30 dark:text-blue-300 font-semibold"
                          : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
                      }`}
                      title="Live Exemplar Manifest"
                    >
                      <svg className="w-4 h-4 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2" />
                      </svg>
                      {navRailExpanded && <span>Live Form Exemplar</span>}
                    </button>
                  </nav>
                </div>

                <div>
                  {navRailExpanded && (
                    <div className="text-[11px] font-semibold uppercase tracking-wider text-slate-400 dark:text-slate-500 px-2 mb-2">
                      Cloud Platform
                    </div>
                  )}
                  <nav className="space-y-1">
                    <button
                      type="button"
                      onClick={() => setActiveTab("cloud")}
                      className={`w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-xs font-medium transition-colors cursor-pointer ${
                        activeTab === "cloud"
                          ? "bg-blue-50 text-blue-700 dark:bg-blue-900/30 dark:text-blue-300 font-semibold"
                          : "text-slate-600 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800/60"
                      }`}
                      title="Google Cloud Run Deployment"
                    >
                      <svg className="w-4 h-4 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M3 15a4 4 0 004 4h9a5 5 0 10-.1-9.999 5.002 5.002 0 00-9.78 2.096A4.001 4.001 0 003 15z" />
                      </svg>
                      {navRailExpanded && <span>Cloud Run Topology</span>}
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
                <span>Kernel:</span>
                <span className="font-mono text-slate-700 dark:text-slate-300">Cedar v4.13</span>
              </div>
              <div className="flex justify-between items-center">
                <span>Relational:</span>
                <span className="font-mono text-slate-700 dark:text-slate-300">PostgreSQL 17</span>
              </div>
              <div className="flex justify-between items-center">
                <span>DNS Routing:</span>
                <span className="font-mono text-slate-700 dark:text-slate-300">&lt; 1 ms</span>
              </div>
            </div>
          )}
        </aside>

        {/* MAIN WORKSPACE CONTENT */}
        <main className="flex-1 overflow-y-auto p-4 md:p-6 bg-slate-100/50 dark:bg-slate-950">
          {/* DISCREET ADMIN PATH VIEW (/admin) */}
          {isAdminPath ? (
            <div className="space-y-6 max-w-6xl mx-auto animate-fade-in">
              <div className="flex flex-wrap items-center justify-between gap-4 pb-2 border-b border-slate-200 dark:border-slate-800">
                <div>
                  <div className="flex items-center gap-2">
                    <span className="w-2.5 h-2.5 rounded-full bg-amber-500 animate-pulse" />
                    <h1 className="text-xl font-bold tracking-tight text-slate-900 dark:text-white">
                      Institutional Administrative Console
                    </h1>
                  </div>
                  <p className="text-xs text-slate-500 dark:text-slate-400 mt-1">
                    Discreet administrative management path: <code className="font-mono text-amber-600 dark:text-amber-400">/admin</code>. Enforcing raw Cedar policy proofs, DNS routing health probes, and application manifests.
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  <button
                    type="button"
                    onClick={() => {
                      const newApp: RegisteredApp = {
                        slug: `dept-app-${Date.now().toString().slice(-4)}`,
                        title: "New Departmental Manifest",
                        orgCode: "DEPT-NEW",
                        department: activePersona.department,
                        customDomain: `app-${Date.now().toString().slice(-4)}.${activePersona.department}.state.edu`,
                        verified: false,
                        hermCapability: "2.1.0 (Academic Operations)",
                        cedsDomain: "PostsecondaryStudent",
                        status: "Draft",
                        updatedAt: new Date().toISOString().split("T")[0],
                        recordsCount: 0,
                      };
                      setApps([newApp, ...apps]);
                      setSelectedApp(newApp);
                      setInspectorOpen(true);
                      showToast("Created new draft manifest. Configure in inspector.");
                    }}
                    className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold rounded-md bg-blue-600 hover:bg-blue-700 text-white shadow-xs cursor-pointer transition-colors"
                  >
                    <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2.5" d="M12 4v16m8-8H4" />
                    </svg>
                    New Manifest
                  </button>
                  <button
                    type="button"
                    onClick={() => navigateTo("/")}
                    className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-900 text-slate-700 dark:text-slate-300 hover:bg-slate-50 dark:hover:bg-slate-800 shadow-xs cursor-pointer transition-colors"
                  >
                    ← Exit to Desk
                  </button>
                </div>
              </div>

              {/* Elevated Operations Desk Panels */}
              <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg p-5 shadow-xs space-y-3">
                  <h3 className="text-sm font-semibold text-slate-900 dark:text-white">
                    Raw Cedar Policy Verification
                  </h3>
                  <p className="text-xs text-slate-500">
                    Compile and inspect active Cedar Policy definitions enforced by `scaffoldry-policy`.
                  </p>
                  <pre className="p-3 rounded bg-slate-950 text-sky-300 font-mono text-[11px] overflow-x-auto max-h-48 border border-slate-800">
{`// Institutional Sovereign Policy Set
@id("ferpa-export-forbid-guard")
forbid (
    principal,
    action == Action::"export",
    resource
) when {
    resource.is_ferpa_sensitive &&
    !(principal.scoped_affiliation in ["staff", "compliance"])
};`}
                  </pre>
                  <button
                    type="button"
                    onClick={() => showToast("Cedar Policy formal proofs verified via Lean 4 lattice.")}
                    className="px-3 py-1.5 text-xs font-semibold rounded bg-slate-800 hover:bg-slate-700 text-white cursor-pointer"
                  >
                    Verify Formal Proofs
                  </button>
                </div>

                <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg p-5 shadow-xs space-y-3">
                  <h3 className="text-sm font-semibold text-slate-900 dark:text-white">
                    DNS Routing Health &amp; Ingress Audit
                  </h3>
                  <p className="text-xs text-slate-500">
                    Simulate sub-millisecond host-header resolution without open ports.
                  </p>
                  <div className="space-y-2 text-xs">
                    <div className="flex justify-between p-2 rounded bg-slate-50 dark:bg-slate-800/60 border border-slate-200 dark:border-slate-700/60">
                      <span className="font-mono text-blue-600 dark:text-blue-400">bio-inventory.science.state.edu</span>
                      <span className="text-emerald-600 dark:text-emerald-400 font-medium">0.42 ms · OK</span>
                    </div>
                    <div className="flex justify-between p-2 rounded bg-slate-50 dark:bg-slate-800/60 border border-slate-200 dark:border-slate-700/60">
                      <span className="font-mono text-blue-600 dark:text-blue-400">travel.science.state.edu</span>
                      <span className="text-emerald-600 dark:text-emerald-400 font-medium">0.38 ms · OK</span>
                    </div>
                    <div className="flex justify-between p-2 rounded bg-slate-50 dark:bg-slate-800/60 border border-slate-200 dark:border-slate-700/60">
                      <span className="font-mono text-blue-600 dark:text-blue-400">lasers.physics.state.edu</span>
                      <span className="text-emerald-600 dark:text-emerald-400 font-medium">0.51 ms · OK</span>
                    </div>
                  </div>
                  <button
                    type="button"
                    onClick={() => showToast("Validated all 3 DNS routing rules against Cloudflare edge.")}
                    className="px-3 py-1.5 text-xs font-semibold rounded bg-slate-800 hover:bg-slate-700 text-white cursor-pointer"
                  >
                    Run Full Ingress Probe
                  </button>
                </div>
              </div>

              {/* Cloud Run Live Topology Card */}
              <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg p-5 shadow-xs space-y-3">
                <h3 className="text-sm font-semibold text-slate-900 dark:text-white">
                  Cloud Run Infrastructure Deployment
                </h3>
                <div className="grid grid-cols-1 md:grid-cols-3 gap-3 text-xs">
                  <div className="p-3 rounded bg-slate-50 dark:bg-slate-800/60 border border-slate-200 dark:border-slate-700/60">
                    <span className="text-slate-400 text-[10px] uppercase font-bold block">Service Endpoint</span>
                    <span className="font-mono text-blue-600 dark:text-blue-400 break-all">https://scaffoldry-desk-ljbhpnq7oa-uc.a.run.app</span>
                  </div>
                  <div className="p-3 rounded bg-slate-50 dark:bg-slate-800/60 border border-slate-200 dark:border-slate-700/60">
                    <span className="text-slate-400 text-[10px] uppercase font-bold block">Region &amp; Project</span>
                    <span className="font-semibold text-slate-800 dark:text-slate-200">us-central1 (scaffoldry-io)</span>
                  </div>
                  <div className="p-3 rounded bg-slate-50 dark:bg-slate-800/60 border border-slate-200 dark:border-slate-700/60">
                    <span className="text-slate-400 text-[10px] uppercase font-bold block">Authentication Plane</span>
                    <span className="font-semibold text-slate-800 dark:text-slate-200">Workload Identity Federation</span>
                  </div>
                </div>
              </div>
            </div>
          ) : (
            /* STANDARD GENERAL DESK (/) */
            <>
              {/* TAB 1: RELATIONAL DATA GRID (APPLICATIONS & DNS) */}
              {activeTab === "apps" && (
                <div className="space-y-4 max-w-7xl mx-auto">
                  {/* Header & Controls Strip */}
                  <div className="flex flex-wrap items-center justify-between gap-4 pb-2">
                    <div>
                      <h1 className="text-xl font-bold tracking-tight text-slate-900 dark:text-white">
                        Departmental Applications &amp; DNS Routing
                      </h1>
                      <p className="text-xs text-slate-500 dark:text-slate-400 mt-0.5">
                        Interactive relational grid for departmental metadata manifests, DNS vanity aliases, and CEDS element mappings.
                      </p>
                    </div>
                    <div className="flex items-center gap-2">
                      <button
                        type="button"
                        onClick={() => showToast("Exported all records to CEDS/OSCAL bundle.")}
                        className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-900 text-slate-700 dark:text-slate-300 hover:bg-slate-50 dark:hover:bg-slate-800 shadow-xs cursor-pointer transition-colors"
                      >
                        Export View
                      </button>
                    </div>
                  </div>

                  {/* Data Grid Toolbar */}
                  <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg p-2.5 flex flex-wrap items-center justify-between gap-3 shadow-xs">
                    {/* Search & Filters */}
                    <div className="flex flex-wrap items-center gap-2 flex-1">
                      <div className="relative min-w-[200px]">
                        <input
                          type="text"
                          placeholder="Filter records..."
                          value={searchQuery}
                          onChange={(e) => setSearchQuery(e.target.value)}
                          className="w-full pl-7 pr-3 py-1 text-xs rounded border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-800/80 text-slate-800 dark:text-slate-200 focus:outline-none focus:ring-1 focus:ring-blue-500"
                        />
                        <svg className="w-3.5 h-3.5 text-slate-400 absolute left-2 top-1.5 pointer-events-none" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
                        </svg>
                      </div>

                      <div className="flex items-center gap-1 text-xs">
                        <span className="text-slate-400 text-[11px]">Dept:</span>
                        <select
                          value={deptFilter}
                          onChange={(e) => setDeptFilter(e.target.value)}
                          className="text-xs bg-slate-50 dark:bg-slate-800 border border-slate-200 dark:border-slate-700 rounded py-1 px-2 text-slate-700 dark:text-slate-300 focus:outline-none"
                        >
                          <option value="all">All Departments</option>
                          <option value="biology">Biology</option>
                          <option value="physics">Physics</option>
                          <option value="compliance">Compliance</option>
                        </select>
                      </div>

                      <div className="flex items-center gap-1 text-xs">
                        <span className="text-slate-400 text-[11px]">Status:</span>
                        <select
                          value={statusFilter}
                          onChange={(e) => setStatusFilter(e.target.value)}
                          className="text-xs bg-slate-50 dark:bg-slate-800 border border-slate-200 dark:border-slate-700 rounded py-1 px-2 text-slate-700 dark:text-slate-300 focus:outline-none"
                        >
                          <option value="all">All Statuses</option>
                          <option value="Published">Published</option>
                          <option value="Draft">Draft</option>
                        </select>
                      </div>
                    </div>

                    {/* View Switchers */}
                    <div className="flex items-center gap-1 border-l border-slate-200 dark:border-slate-800 pl-3">
                      <button
                        type="button"
                        onClick={() => setViewMode("grid")}
                        className={`p-1.5 rounded text-xs ${viewMode === "grid" ? "bg-slate-200 dark:bg-slate-800 text-blue-600 dark:text-blue-400 font-bold" : "text-slate-500 hover:text-slate-800 dark:hover:text-slate-200"}`}
                        title="Tabular Grid View"
                      >
                        Grid View
                      </button>
                      <button
                        type="button"
                        onClick={() => setViewMode("cards")}
                        className={`p-1.5 rounded text-xs ${viewMode === "cards" ? "bg-slate-200 dark:bg-slate-800 text-blue-600 dark:text-blue-400 font-bold" : "text-slate-500 hover:text-slate-800 dark:hover:text-slate-200"}`}
                        title="Card Gallery View"
                      >
                        Card View
                      </button>
                    </div>
                  </div>

                  {/* TABULAR DATA GRID */}
                  {viewMode === "grid" ? (
                    <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg overflow-hidden shadow-xs">
                      <div className="overflow-x-auto">
                        <table className="w-full text-left text-xs border-collapse">
                          <thead>
                            <tr className="bg-slate-50 dark:bg-slate-800/70 border-b border-slate-200 dark:border-slate-800 text-slate-500 dark:text-slate-400 font-semibold tracking-wider uppercase text-[10px]">
                              <th className="py-2.5 px-3 w-8">#</th>
                              <th className="py-2.5 px-3">Application Title &amp; Slug</th>
                              <th className="py-2.5 px-3">Department</th>
                              <th className="py-2.5 px-3">DNS Vanity Alias</th>
                              <th className="py-2.5 px-3">HERM Capability</th>
                              <th className="py-2.5 px-3">CEDS Domain</th>
                              <th className="py-2.5 px-3">Records</th>
                              <th className="py-2.5 px-3">Status</th>
                              <th className="py-2.5 px-3 text-right">Actions</th>
                            </tr>
                          </thead>
                          <tbody className="divide-y divide-slate-100 dark:divide-slate-800/80">
                            {filteredApps.map((app, idx) => {
                              const isSelected = selectedApp?.slug === app.slug;
                              return (
                                <tr
                                  key={app.slug}
                                  onClick={() => handleSelectApp(app)}
                                  className={`cursor-pointer transition-colors ${
                                    isSelected
                                      ? "bg-blue-50/70 dark:bg-blue-950/40"
                                      : "hover:bg-slate-50 dark:hover:bg-slate-800/50"
                                  }`}
                                >
                                  <td className="py-2.5 px-3 font-mono text-[11px] text-slate-400">{idx + 1}</td>
                                  <td className="py-2.5 px-3">
                                    <div className="font-semibold text-slate-800 dark:text-slate-200">{app.title}</div>
                                    <div className="font-mono text-[11px] text-slate-400">{app.slug}</div>
                                  </td>
                                  <td className="py-2.5 px-3">
                                    <span className="inline-flex items-center px-2 py-0.5 rounded text-[11px] font-medium bg-slate-100 dark:bg-slate-800 text-slate-700 dark:text-slate-300">
                                      {app.department.toUpperCase()}
                                    </span>
                                  </td>
                                  <td className="py-2.5 px-3 font-mono">
                                    <span className="text-blue-600 dark:text-blue-400 font-medium">{app.customDomain}</span>
                                  </td>
                                  <td className="py-2.5 px-3 text-slate-600 dark:text-slate-400">{app.hermCapability}</td>
                                  <td className="py-2.5 px-3 text-slate-600 dark:text-slate-400">{app.cedsDomain}</td>
                                  <td className="py-2.5 px-3 font-mono text-slate-700 dark:text-slate-300">{app.recordsCount}</td>
                                  <td className="py-2.5 px-3">
                                    <span
                                      className={`inline-flex items-center px-2 py-0.5 rounded text-[10px] font-semibold tracking-wide uppercase ${
                                        app.status === "Published"
                                          ? "bg-emerald-50 text-emerald-700 dark:bg-emerald-950/50 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800"
                                          : "bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300 border border-slate-300 dark:border-slate-700"
                                      }`}
                                    >
                                      {app.status}
                                    </span>
                                  </td>
                                  <td className="py-2.5 px-3 text-right">
                                    <button
                                      type="button"
                                      onClick={(e) => {
                                        e.stopPropagation();
                                        handleSelectApp(app);
                                      }}
                                      className="px-2 py-1 text-[11px] rounded font-medium border border-slate-200 dark:border-slate-700 hover:bg-slate-100 dark:hover:bg-slate-800 text-slate-600 dark:text-slate-300"
                                    >
                                      Inspect
                                    </button>
                                  </td>
                                </tr>
                              );
                            })}
                          </tbody>
                        </table>
                      </div>
                      {filteredApps.length === 0 && (
                        <div className="py-12 text-center text-xs text-slate-500">
                          No applications match the current filter criteria.
                        </div>
                      )}
                    </div>
                  ) : (
                    /* CARD GALLERY VIEW */
                    <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
                      {filteredApps.map((app) => (
                        <div
                          key={app.slug}
                          onClick={() => handleSelectApp(app)}
                          className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 hover:border-blue-400 dark:hover:border-blue-600 rounded-lg p-4 cursor-pointer transition-all shadow-xs flex flex-col justify-between"
                        >
                          <div>
                            <div className="flex items-center justify-between mb-2">
                              <span className="text-[10px] uppercase font-bold tracking-wider px-2 py-0.5 rounded bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-400">
                                {app.department}
                              </span>
                              <span className="text-[10px] font-semibold text-emerald-600 dark:text-emerald-400">
                                ✓ {app.status}
                              </span>
                            </div>
                            <h3 className="font-semibold text-sm text-slate-900 dark:text-white mb-1">{app.title}</h3>
                            <p className="font-mono text-xs text-blue-600 dark:text-blue-400 mb-3">{app.customDomain}</p>
                            <div className="text-xs text-slate-500 dark:text-slate-400 space-y-1">
                              <div>HERM: {app.hermCapability}</div>
                              <div>CEDS: {app.cedsDomain}</div>
                            </div>
                          </div>
                          <div className="mt-4 pt-3 border-t border-slate-100 dark:border-slate-800 flex justify-between items-center text-xs text-slate-400">
                            <span>{app.recordsCount} records</span>
                            <span className="text-blue-600 dark:text-blue-400 font-medium">Inspect Record →</span>
                          </div>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              )}

              {/* TAB 2: SOURCE RULES & OSCAL LATTICE */}
              {activeTab === "policy" && (
                <div className="space-y-6 max-w-7xl mx-auto">
                  <div>
                    <h1 className="text-xl font-bold tracking-tight text-slate-900 dark:text-white">
                      Institutional Source Rules &amp; OSCAL Lattice
                    </h1>
                    <p className="text-xs text-slate-500 dark:text-slate-400 mt-0.5">
                      Bidirectional mapping linking statutory rules (FERPA, NIST 800-53) to executable Cedar Policy syntax and OSCAL 1.1.2 controls.
                    </p>
                  </div>

                  {/* Rules Grid */}
                  <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg overflow-hidden shadow-xs">
                    <table className="w-full text-left text-xs border-collapse">
                      <thead>
                        <tr className="bg-slate-50 dark:bg-slate-800/70 border-b border-slate-200 dark:border-slate-800 text-slate-500 dark:text-slate-400 font-semibold tracking-wider uppercase text-[10px]">
                          <th className="py-2.5 px-3">Statutory Rule / Authority</th>
                          <th className="py-2.5 px-3">OSCAL Control</th>
                          <th className="py-2.5 px-3">Executable Cedar Policy</th>
                          <th className="py-2.5 px-3">Classification</th>
                          <th className="py-2.5 px-3">Status</th>
                          <th className="py-2.5 px-3 text-right">Inspect</th>
                        </tr>
                      </thead>
                      <tbody className="divide-y divide-slate-100 dark:divide-slate-800">
                        {sourceRules.map((rule) => (
                          <tr
                            key={rule.id}
                            onClick={() => handleSelectRule(rule)}
                            className="hover:bg-slate-50 dark:hover:bg-slate-800/50 cursor-pointer transition-colors"
                          >
                            <td className="py-3 px-3">
                              <div className="font-semibold text-slate-900 dark:text-white">{rule.source}</div>
                              <div className="text-[11px] text-slate-500">{rule.title}</div>
                            </td>
                            <td className="py-3 px-3">
                              <span className="inline-flex items-center px-2 py-0.5 rounded text-[11px] font-mono bg-purple-50 text-purple-700 dark:bg-purple-950/60 dark:text-purple-300 border border-purple-200 dark:border-purple-800">
                                {rule.oscalControl}
                              </span>
                            </td>
                            <td className="py-3 px-3">
                              <code className="block max-w-md p-1.5 rounded font-mono text-[11px] bg-slate-900 text-sky-300 dark:bg-slate-950 border border-slate-800 overflow-x-auto">
                                {rule.cedarSnippet}
                              </code>
                            </td>
                            <td className="py-3 px-3">
                              <span className="inline-flex items-center px-2 py-0.5 rounded text-[11px] font-medium bg-amber-50 text-amber-700 dark:bg-amber-950/60 dark:text-amber-300 border border-amber-200 dark:border-amber-800">
                                {rule.targetSensitivity}
                              </span>
                            </td>
                            <td className="py-3 px-3">
                              <span className="inline-flex items-center px-2 py-0.5 rounded text-[11px] font-semibold text-emerald-700 dark:text-emerald-300 bg-emerald-50 dark:bg-emerald-950/60 border border-emerald-200 dark:border-emerald-800">
                                ✓ {rule.status}
                              </span>
                            </td>
                            <td className="py-3 px-3 text-right">
                              <button
                                type="button"
                                className="px-2 py-1 text-[11px] rounded font-medium border border-slate-200 dark:border-slate-700 hover:bg-slate-100 dark:hover:bg-slate-800 text-slate-600 dark:text-slate-300"
                              >
                                Details
                              </button>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>

                  {/* Policy Decision Simulator */}
                  <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg p-5 shadow-xs space-y-4">
                    <div className="flex items-center justify-between">
                      <div>
                        <h2 className="text-base font-bold text-slate-900 dark:text-white">
                          Interactive Cedar Decision Simulator
                        </h2>
                        <p className="text-xs text-slate-500 dark:text-slate-400">
                          Simulate Cedar authorization checks under identity:{" "}
                          <strong className="text-slate-700 dark:text-slate-300">{activePersona.eppn}</strong> ({activePersona.roleTitle}).
                        </p>
                      </div>
                      <span className="text-[10px] font-mono uppercase px-2 py-0.5 rounded bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-300">
                        Cedar Engine v4.13
                      </span>
                    </div>

                    <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
                      <div>
                        <label className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1">
                          Action Requested:
                        </label>
                        <select
                          value={simAction}
                          onChange={(e) => setSimAction(e.target.value as "read" | "write" | "export")}
                          className="w-full text-xs rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 p-2 text-slate-800 dark:text-slate-200 focus:outline-none focus:ring-1 focus:ring-blue-500"
                        >
                          <option value="read">Action::&quot;read&quot;</option>
                          <option value="write">Action::&quot;write&quot;</option>
                          <option value="export">Action::&quot;export&quot; (Special Safeguard)</option>
                        </select>
                      </div>

                      <div>
                        <label className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1">
                          Record Classification:
                        </label>
                        <select
                          value={simFerpa ? "true" : "false"}
                          onChange={(e) => setSimFerpa(e.target.value === "true")}
                          className="w-full text-xs rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 p-2 text-slate-800 dark:text-slate-200 focus:outline-none focus:ring-1 focus:ring-blue-500"
                        >
                          <option value="false">Standard Departmental Record</option>
                          <option value="true">FERPA Sensitive Student Record (34 CFR § 99.30)</option>
                        </select>
                      </div>

                      <div>
                        <label className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1">
                          Target Resource Realm:
                        </label>
                        <input
                          type="text"
                          disabled
                          value="Department of Biology"
                          className="w-full text-xs rounded border border-slate-200 dark:border-slate-700 bg-slate-100 dark:bg-slate-800/50 p-2 text-slate-500 dark:text-slate-400"
                        />
                      </div>
                    </div>

                    {/* Simulation Verdict Card */}
                    <div
                      className={`p-4 rounded-lg border flex flex-col sm:flex-row sm:items-center justify-between gap-3 ${
                        simResult.decision === "ALLOW"
                          ? "bg-emerald-50 dark:bg-emerald-950/30 border-emerald-300 dark:border-emerald-800/80 text-emerald-900 dark:text-emerald-200"
                          : "bg-rose-50 dark:bg-rose-950/30 border-rose-300 dark:border-rose-800/80 text-rose-900 dark:text-rose-200"
                      }`}
                    >
                      <div className="space-y-1">
                        <div className="flex items-center gap-2">
                          <span
                            className={`text-xs font-black tracking-wider uppercase px-2.5 py-0.5 rounded font-mono ${
                              simResult.decision === "ALLOW"
                                ? "bg-emerald-600 text-white"
                                : "bg-rose-600 text-white"
                            }`}
                          >
                            CEDAR {simResult.decision}
                          </span>
                          <span className="font-semibold text-sm">{simResult.reason}</span>
                        </div>
                        <div className="text-xs opacity-80">
                          Evaluated principal: <code>{activePersona.eppn}</code> ({activePersona.affiliation}@{activePersona.department})
                        </div>
                      </div>
                      <div className="text-[11px] font-mono opacity-70">
                        Audit Reference: {simResult.rule}
                      </div>
                    </div>
                  </div>
                </div>
              )}

              {/* TAB 3: LIVE EXEMPLAR FORM */}
              {activeTab === "exemplar" && (
                <div className="space-y-6 max-w-4xl mx-auto">
                  <div>
                    <h1 className="text-xl font-bold tracking-tight text-slate-900 dark:text-white">
                      Live Departmental App (Exemplar)
                    </h1>
                    <p className="text-xs text-slate-500 dark:text-slate-400 mt-0.5">
                      Demonstrating live manifest rendering, field-level CEDS crosswalk annotations, and active Cedar policy protection.
                    </p>
                  </div>

                  <ManifestRenderer
                    manifest={sampleManifest}
                    onSubmitRecord={(_record) => {
                      showToast(`Record successfully registered by ${activePersona.name}!`);
                    }}
                  />
                </div>
              )}

              {/* TAB 4: CLOUD RUN TOPOLOGY */}
              {activeTab === "cloud" && (
                <div className="space-y-6 max-w-6xl mx-auto">
                  <div>
                    <h1 className="text-xl font-bold tracking-tight text-slate-900 dark:text-white">
                      Google Cloud Run Platform Topology
                    </h1>
                    <p className="text-xs text-slate-500 dark:text-slate-400 mt-0.5">
                      Live production topology deployed on Google Cloud Platform with Workload Identity Federation.
                    </p>
                  </div>

                  <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                    <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg p-4 shadow-xs">
                      <div className="text-[11px] uppercase tracking-wider text-slate-400 font-semibold mb-1">
                        Live Service Endpoint
                      </div>
                      <div className="font-mono text-xs text-blue-600 dark:text-blue-400 font-medium break-all">
                        https://scaffoldry-desk-ljbhpnq7oa-uc.a.run.app
                      </div>
                      <div className="mt-2 flex items-center gap-1.5 text-xs text-emerald-600 dark:text-emerald-400">
                        <span className="w-2 h-2 rounded-full bg-emerald-500" />
                        <span>Serving 100% Traffic (Revision 00001-ls5)</span>
                      </div>
                    </div>

                    <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg p-4 shadow-xs">
                      <div className="text-[11px] uppercase tracking-wider text-slate-400 font-semibold mb-1">
                        GCP Region &amp; Project
                      </div>
                      <div className="text-sm font-semibold text-slate-900 dark:text-white">
                        us-central1 (Iowa)
                      </div>
                      <div className="text-xs text-slate-500 font-mono mt-1">
                        scaffoldry-io (#650403699760)
                      </div>
                    </div>

                    <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg p-4 shadow-xs">
                      <div className="text-[11px] uppercase tracking-wider text-slate-400 font-semibold mb-1">
                        Zero-Trust Authentication
                      </div>
                      <div className="text-sm font-semibold text-slate-900 dark:text-white">
                        Workload Identity Federation
                      </div>
                      <div className="text-xs text-slate-500 font-mono mt-1">
                        github-pool / github-provider
                      </div>
                    </div>
                  </div>
                </div>
              )}
            </>
          )}
        </main>

        {/* SLIDE-OVER RECORD INSPECTOR DRAWER */}
        {inspectorOpen && (
          <aside className="w-96 bg-white dark:bg-slate-900 border-l border-slate-200 dark:border-slate-800 flex flex-col justify-between shadow-2xl z-30 shrink-0 animate-slide-left">
            {/* Drawer Header */}
            <div className="p-4 border-b border-slate-200 dark:border-slate-800 flex items-center justify-between">
              <div>
                <span className="text-[10px] font-bold uppercase tracking-wider text-slate-400">
                  Record Detail Inspector
                </span>
                <h2 className="text-sm font-bold text-slate-900 dark:text-white truncate max-w-[240px]">
                  {selectedApp ? selectedApp.title : selectedRule?.title}
                </h2>
              </div>
              <button
                type="button"
                onClick={() => setInspectorOpen(false)}
                className="p-1 rounded-md text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800"
              >
                ✕
              </button>
            </div>

            {/* Drawer Body */}
            <div className="p-4 flex-1 overflow-y-auto space-y-4 text-xs">
              {selectedApp && (
                <div className="space-y-4">
                  <div>
                    <label className="block text-[11px] font-semibold text-slate-500 uppercase mb-1">
                      Application Title
                    </label>
                    <input
                      type="text"
                      value={selectedApp.title}
                      onChange={(e) => handleUpdateSelectedApp({ ...selectedApp, title: e.target.value })}
                      className="w-full px-2.5 py-1.5 text-xs rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white focus:outline-none focus:ring-1 focus:ring-blue-500"
                    />
                  </div>

                  <div>
                    <label className="block text-[11px] font-semibold text-slate-500 uppercase mb-1">
                      DNS Vanity Alias (Host Header)
                    </label>
                    <input
                      type="text"
                      value={selectedApp.customDomain}
                      onChange={(e) => handleUpdateSelectedApp({ ...selectedApp, customDomain: e.target.value })}
                      className="w-full px-2.5 py-1.5 font-mono text-xs rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-blue-600 dark:text-blue-400 focus:outline-none focus:ring-1 focus:ring-blue-500"
                    />
                  </div>

                  <div className="grid grid-cols-2 gap-2">
                    <div>
                      <label className="block text-[11px] font-semibold text-slate-500 uppercase mb-1">
                        Department
                      </label>
                      <input
                        type="text"
                        disabled
                        value={selectedApp.department}
                        className="w-full px-2.5 py-1.5 text-xs rounded border border-slate-200 dark:border-slate-800 bg-slate-100 dark:bg-slate-800/40 text-slate-500"
                      />
                    </div>
                    <div>
                      <label className="block text-[11px] font-semibold text-slate-500 uppercase mb-1">
                        Status
                      </label>
                      <select
                        value={selectedApp.status}
                        onChange={(e) =>
                          handleUpdateSelectedApp({
                            ...selectedApp,
                            status: e.target.value as "Published" | "Draft",
                          })
                        }
                        className="w-full px-2 py-1.5 text-xs rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white"
                      >
                        <option value="Published">Published</option>
                        <option value="Draft">Draft</option>
                      </select>
                    </div>
                  </div>

                  <div className="pt-2 border-t border-slate-200 dark:border-slate-800">
                    <span className="block text-[11px] font-semibold text-slate-500 uppercase mb-1.5">
                      Standards Crosswalk
                    </span>
                    <div className="p-2.5 rounded bg-slate-50 dark:bg-slate-800/60 border border-slate-200 dark:border-slate-700/60 space-y-1.5">
                      <div className="flex justify-between">
                        <span className="text-slate-500">HERM:</span>
                        <span className="font-semibold text-slate-700 dark:text-slate-300">{selectedApp.hermCapability}</span>
                      </div>
                      <div className="flex justify-between">
                        <span className="text-slate-500">CEDS Domain:</span>
                        <span className="font-semibold text-slate-700 dark:text-slate-300">{selectedApp.cedsDomain}</span>
                      </div>
                    </div>
                  </div>

                  {/* Immediate Cedar Access check for this specific record */}
                  <div className="pt-2 border-t border-slate-200 dark:border-slate-800">
                    <span className="block text-[11px] font-semibold text-slate-500 uppercase mb-1.5">
                      Simulated Identity Access
                    </span>
                    {(() => {
                      const check = evaluateCedarDecision(selectedApp.department, false, "read");
                      return (
                        <div
                          className={`p-2.5 rounded border text-xs ${
                            check.decision === "ALLOW"
                              ? "bg-emerald-50 dark:bg-emerald-950/30 border-emerald-300 dark:border-emerald-800 text-emerald-800 dark:text-emerald-300"
                              : "bg-rose-50 dark:bg-rose-950/30 border-rose-300 dark:border-rose-800 text-rose-800 dark:text-rose-300"
                          }`}
                        >
                          <div className="font-bold">CEDAR {check.decision}</div>
                          <div className="text-[11px] opacity-90 mt-0.5">{check.reason}</div>
                        </div>
                      );
                    })()}
                  </div>
                </div>
              )}

              {selectedRule && (
                <div className="space-y-4">
                  <div>
                    <label className="block text-[11px] font-semibold text-slate-500 uppercase mb-1">
                      Statutory Baseline
                    </label>
                    <div className="font-semibold text-slate-900 dark:text-white">{selectedRule.source}</div>
                  </div>
                  <div>
                    <label className="block text-[11px] font-semibold text-slate-500 uppercase mb-1">
                      OSCAL Control ID
                    </label>
                    <span className="inline-flex px-2 py-0.5 rounded font-mono text-xs bg-purple-100 dark:bg-purple-900/50 text-purple-800 dark:text-purple-300">
                      {selectedRule.oscalControl}
                    </span>
                  </div>
                  <div>
                    <label className="block text-[11px] font-semibold text-slate-500 uppercase mb-1">
                      Cedar Policy Syntax
                    </label>
                    <pre className="p-2.5 rounded bg-slate-950 text-sky-300 font-mono text-[11px] overflow-x-auto whitespace-pre-wrap border border-slate-800">
                      {selectedRule.cedarSnippet}
                    </pre>
                  </div>
                </div>
              )}
            </div>

            {/* Drawer Footer */}
            <div className="p-3 border-t border-slate-200 dark:border-slate-800 flex justify-between items-center bg-slate-50 dark:bg-slate-900/80">
              <button
                type="button"
                onClick={() => setInspectorOpen(false)}
                className="px-3 py-1.5 rounded text-xs text-slate-600 dark:text-slate-400 hover:text-slate-900 dark:hover:text-slate-200 cursor-pointer"
              >
                Close Drawer
              </button>
              <button
                type="button"
                onClick={() => {
                  setInspectorOpen(false);
                  showToast("Record changes synchronized.");
                }}
                className="px-3 py-1.5 rounded text-xs font-semibold bg-blue-600 hover:bg-blue-700 text-white cursor-pointer shadow-xs"
              >
                Done
              </button>
            </div>
          </aside>
        )}
      </div>
    </div>
  );
};
