import { ProcessInsightsPanel } from "../ProcessInsights";
import React, { useState, useEffect } from "react";
import { Overview } from "./Overview";
import { Jobs } from "./Jobs";
import { LegalHolds } from "./LegalHolds";
import { Secrets } from "./Secrets";
import { OutboundAccess } from "./OutboundAccess";
import { Connections } from "./Connections";
import { Shares } from "./Shares";
import { Guests } from "./Guests";
import { Flows } from "./Flows";
import { DataGovernance } from "./DataGovernance";
import { SensitiveContent } from "./SensitiveContent";
import { Webhooks } from "./Webhooks";
import { PolicyLibrary } from "./PolicyLibrary";
import { People } from "./People";
import { Positions } from "./Positions";
import { Authority } from "./Authority";
import { OutsideApps } from "./OutsideApps";
import { Access } from "./Access";
import { Workspaces } from "./Workspaces";
import { PolicyOscal } from "./PolicyOscal";
import { AdminConsoleView } from "../AdminConsoleView";
import { apiClient } from "../api";
import { Persona, RegisteredApp, LedgerEntryItem } from "../types";

export interface AdminConsoleProps {
  activePersona?: Persona;
  isImpersonating?: boolean;
  realAdmin?: Persona | null;
  handleStopImpersonation?: () => void;
  navigateTo?: (path: string) => void;
  /** The path the console is showing, for sections with views of their own. */
  currentPath?: string;
  adminTab?: "overview" | "jobs" | "people" | "positions" | "access" | "authority" | "clients" | "holds" | "processes" | "secrets" | "outbound" | "connections" | "shares" | "guests" | "flows" | "governance" | "content" | "webhooks" | "library" | "workspaces" | "org" | "policy" | "ledger" | "impersonation" | "settings";
  setAdminTab?: (tab: "overview" | "jobs" | "people" | "positions" | "access" | "authority" | "clients" | "holds" | "processes" | "secrets" | "outbound" | "connections" | "shares" | "guests" | "flows" | "governance" | "content" | "webhooks" | "library" | "workspaces" | "org" | "policy" | "ledger" | "impersonation" | "settings") => void;
  apps?: RegisteredApp[];
  ledger?: LedgerEntryItem[];
  setNotificationToast?: React.Dispatch<React.SetStateAction<string | null>>;
  setErrorToast?: (message: string) => void;
  personas?: Persona[];
  handleStartImpersonation?: (user: Persona) => void;
}

export function AdminConsole({
  activePersona,
  isImpersonating = false,
  realAdmin = null,
  handleStopImpersonation = () => {},
  navigateTo = () => {},
  currentPath,
  adminTab: controlledTab,
  setAdminTab: _setControlledTab,
  apps = [],
  ledger = [],
  setNotificationToast = () => {},
  setErrorToast,
  personas = [],
  handleStartImpersonation = () => {},
}: AdminConsoleProps) {
  const [internalTab] = useState<
    "overview" | "jobs" | "people" | "positions" | "access" | "authority" | "clients" | "holds" | "processes" | "secrets" | "outbound" | "connections" | "shares" | "guests" | "flows" | "governance" | "content" | "webhooks" | "library" | "workspaces" | "org" | "policy" | "ledger" | "impersonation" | "settings"
  >(controlledTab ?? "overview");

  const currentTab = controlledTab ?? internalTab;

  const [apiAllowed, setApiAllowed] = useState<boolean | null>(null);

  useEffect(() => {
    let active = true;
    apiClient
      .getCurrentUser()
      .then((res) => {
        if (!active) return;
        const allowed = Boolean(
          res.is_platform_admin ||
            res.user?.affiliation === "compliance" ||
            res.user?.affiliation === "central_admin"
        );
        setApiAllowed(allowed);
      })
      .catch(() => {
        if (!active) return;
        setApiAllowed(null);
      });
    return () => {
      active = false;
    };
  }, [activePersona]);

  const isPersonaAllowed = activePersona
    ? Boolean(
        activePersona.isAdmin ||
          activePersona.affiliation === "central_admin" ||
          activePersona.affiliation === "compliance"
      )
    : true;

  const isDenied = !isPersonaAllowed || apiAllowed === false;

  if (isDenied) {
    return (
      <div
        data-testid="admin-access-denied"
        className="p-8 max-w-xl mx-auto my-12 bg-white dark:bg-slate-900 rounded-xl border border-rose-200 dark:border-rose-900/60 shadow-lg text-center space-y-4 animate-fade-in"
      >
        <div className="w-12 h-12 mx-auto rounded-full bg-rose-100 dark:bg-rose-950/60 text-rose-600 dark:text-rose-400 flex items-center justify-center text-xl font-bold">
          🛡️
        </div>
        <h3 className="text-lg font-bold text-slate-900 dark:text-white">
          403 Forbidden: Cedar Policy Authorization Required
        </h3>
        <p className="text-sm text-slate-600 dark:text-slate-400 leading-relaxed">
          The sovereign platform security boundary strictly restricts the Administrative Console to
          Platform Administrators (central_admin) and authorized Compliance Officers.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-6" data-testid="admin-console-shell">
      {currentTab === "overview" ? (
        <Overview />
      ) : currentTab === "jobs" ? (
        <Jobs />
      ) : currentTab === "positions" ? (
        <Positions />
      ) : currentTab === "access" ? (
        <Access />
      ) : currentTab === "authority" ? (
        <Authority />
      ) : currentTab === "clients" ? (
        <OutsideApps />
      ) : currentTab === "holds" ? (
        <LegalHolds />
      ) : currentTab === "processes" ? (
        <ProcessInsightsPanel />
      ) : currentTab === "secrets" ? (
        <Secrets />
      ) : currentTab === "outbound" ? (
        <OutboundAccess />
      ) : currentTab === "connections" ? (
        <Connections />
      ) : currentTab === "shares" ? (
        <Shares />
      ) : currentTab === "guests" ? (
        <Guests />
      ) : currentTab === "flows" ? (
        <Flows />
      ) : currentTab === "governance" ? (
        <DataGovernance />
      ) : currentTab === "content" ? (
        <SensitiveContent />
      ) : currentTab === "webhooks" ? (
        <Webhooks />
      ) : currentTab === "library" ? (
        <PolicyLibrary path={currentPath?.startsWith("/admin/library") ? currentPath : "/admin/library"} navigate={navigateTo ?? (() => {})} />
      ) : currentTab === "workspaces" ? (
        <Workspaces />
      ) : currentTab === "policy" ? (
        <PolicyOscal />
      ) : currentTab === "people" ? (
        <People
          onImpersonate={(row) =>
            handleStartImpersonation({
              eppn: row.user_name,
              name: row.display_name,
              affiliation: (["faculty", "staff", "student", "compliance", "central_admin"] as const).find((a) => a === row.affiliation) ?? "staff",
              department: "",
              roleTitle: "",
              isAdmin: row.platform_admin,
            })
          }
        />
      ) : (
        <AdminConsoleView
          activePersona={activePersona ?? { eppn: "admin@state.edu", name: "Admin", affiliation: "central_admin", department: "IT", roleTitle: "Admin", isAdmin: true }}
          isImpersonating={isImpersonating}
          realAdmin={realAdmin}
          handleStopImpersonation={handleStopImpersonation}
          navigateTo={navigateTo}
          adminTab={currentTab as any}
          apps={apps}
          ledger={ledger}
          setNotificationToast={setNotificationToast}
          setErrorToast={setErrorToast}
          personas={personas}
          handleStartImpersonation={handleStartImpersonation}
        />
      )}
    </div>
  );
}
