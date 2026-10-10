import React, { useState, useEffect } from "react";
import { Overview } from "./Overview";
import { AdminConsoleView } from "../AdminConsoleView";
import { apiClient } from "../api";
import { Persona, RegisteredApp, SourceRule, LedgerEntryItem } from "../types";

export interface AdminConsoleProps {
  activePersona?: Persona;
  isImpersonating?: boolean;
  realAdmin?: Persona | null;
  handleStopImpersonation?: () => void;
  navigateTo?: (path: string) => void;
  adminTab?: "overview" | "org" | "policy" | "ledger" | "impersonation" | "settings";
  setAdminTab?: (tab: "overview" | "org" | "policy" | "ledger" | "impersonation" | "settings") => void;
  apps?: RegisteredApp[];
  sourceRules?: SourceRule[];
  simAction?: "read" | "write" | "export";
  setSimAction?: React.Dispatch<React.SetStateAction<"read" | "write" | "export">>;
  simFerpa?: boolean;
  setSimFerpa?: React.Dispatch<React.SetStateAction<boolean>>;
  simResult?: { decision: string; reason: string; rule: string };
  ledger?: LedgerEntryItem[];
  setNotificationToast?: React.Dispatch<React.SetStateAction<string | null>>;
  handleDownloadOscal?: () => void;
  personas?: Persona[];
  handleStartImpersonation?: (user: Persona) => void;
}

export function AdminConsole({
  activePersona,
  isImpersonating = false,
  realAdmin = null,
  handleStopImpersonation = () => {},
  navigateTo = () => {},
  adminTab: controlledTab,
  setAdminTab: _setControlledTab,
  apps = [],
  sourceRules = [],
  simAction = "read",
  setSimAction = () => {},
  simFerpa = false,
  setSimFerpa = () => {},
  simResult = { decision: "", reason: "", rule: "" },
  ledger = [],
  setNotificationToast = () => {},
  handleDownloadOscal = () => {},
  personas = [],
  handleStartImpersonation = () => {},
}: AdminConsoleProps) {
  const [internalTab] = useState<
    "overview" | "org" | "policy" | "ledger" | "impersonation" | "settings"
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
      ) : (
        <AdminConsoleView
          activePersona={activePersona ?? { eppn: "admin@state.edu", name: "Admin", affiliation: "central_admin", department: "IT", roleTitle: "Admin", isAdmin: true }}
          isImpersonating={isImpersonating}
          realAdmin={realAdmin}
          handleStopImpersonation={handleStopImpersonation}
          navigateTo={navigateTo}
          adminTab={currentTab as any}
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
          personas={personas}
          handleStartImpersonation={handleStartImpersonation}
        />
      )}
    </div>
  );
}
