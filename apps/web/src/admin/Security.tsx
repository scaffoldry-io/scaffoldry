import { useCallback, useMemo, useState, type FormEvent } from "react";
import { apiClient, type AdminPageRow, type AdminToolRow, type AdminTokenRow } from "../api";
import { Banner, ConfirmAction, DataTable, Drawer, EmptyState, ErrorState, FormField, PageHeader, Skeleton, StatusBadge, Tabs, useAsync, type DataTableColumn } from "../ui";

type TabId = "tokens" | "agents";

const buttonClass = "px-3 py-1.5 text-sm font-medium rounded border border-gray-300 dark:border-gray-700 disabled:opacity-50";
const smallButton = "px-2 py-1 text-xs font-medium rounded border border-gray-300 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800";
const inputClass = "w-full text-sm rounded border border-gray-300 dark:border-gray-700 p-2 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100";

function when(iso: string | null): string {
  if (!iso) return "never";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString("en-US", { dateStyle: "medium", timeStyle: "short" });
}

// The Security section of the console (admin console phase 7). Tokens and impersonation sessions can be ended. The agent
// controls only take capability away: a tool that may not run, a custom page that may not run. None turns a check off.
export function Security() {
  const [tab, setTab] = useState<TabId>("tokens");
  return (
    <div className="space-y-6" data-testid="admin-security">
      <PageHeader
        title="Security"
        description="Tokens and impersonation sessions, and what AI tools and custom pages are allowed to do. A change here is recorded in the audit ledger with its reason."
      />
      <Tabs
        ariaLabel="Security"
        selectedId={tab}
        onSelect={(id) => setTab(id as TabId)}
        tabs={[
          { id: "tokens", label: "Tokens" },
          { id: "agents", label: "Agent access" },
        ]}
      />
      {tab === "tokens" && <Tokens />}
      {tab === "agents" && <AgentAccess />}
    </div>
  );
}

// --- Tokens ---------------------------------------------------------------------------------------------------------------

const KINDS = ["agent", "scim", "impersonation", "session", "setup"];

function tone(state: AdminTokenRow["state"]) {
  return state === "active" ? "ok" : state === "expired" ? "neutral" : "bad";
}

function Tokens() {
  const [kind, setKind] = useState("");
  const [state, setState] = useState("");
  const [version, setVersion] = useState(0);
  const [ending, setEnding] = useState<AdminTokenRow | null>(null);
  const filterValues = useMemo(() => ({ kind, state, version }), [kind, state, version]);

  const load = useCallback(async (cursor?: string, owner?: string, f?: Record<string, unknown>) => {
    const res = await apiClient.adminTokens({ cursor, owner, kind: (f?.kind as string) || undefined, state: (f?.state as string) || undefined, limit: "50" });
    return { rows: res.rows, next_cursor: res.next_cursor ?? undefined };
  }, []);

  const columns: DataTableColumn<AdminTokenRow>[] = [
    { id: "kind", header: "Kind", accessorKey: "kind" },
    { id: "owner", header: "Owner", accessorKey: "owner" },
    { id: "label", header: "Label", accessorKey: "label" },
    { id: "state", header: "State", cell: ({ row }) => <StatusBadge tone={tone(row.original.state)} text={row.original.state} /> },
    { id: "created_at", header: "Created", cell: ({ row }) => when(row.original.created_at) },
    { id: "expires_at", header: "Expires", cell: ({ row }) => when(row.original.expires_at) },
    { id: "last_used_at", header: "Last used", cell: ({ row }) => when(row.original.last_used_at) },
    { id: "original_admin", header: "Started by", cell: ({ row }) => row.original.original_admin ?? "" },
    {
      id: "actions",
      header: "",
      cell: ({ row }) =>
        row.original.state === "revoked" ? null : (
          <button
            type="button"
            className={smallButton}
            aria-label={`${row.original.kind === "impersonation" ? "End session" : "Revoke"} ${row.original.label || row.original.owner}`}
            onClick={(e) => {
              e.stopPropagation();
              setEnding(row.original);
            }}
          >
            {row.original.kind === "impersonation" ? "End session" : "Revoke"}
          </button>
        ),
    },
  ];

  const impersonation = kind === "impersonation";
  return (
    <div className="space-y-4">
      <MintScim onMinted={() => setVersion((v) => v + 1)} />
      <DataTable<AdminTokenRow>
        columns={columns}
        load={load}
        rowKey="id"
        filterValues={filterValues}
        emptyState={<EmptyState title="No token matches" description="A token is made by its owner, or by an administrator for a SCIM client." />}
        filters={
          <div className="flex flex-wrap items-center gap-2">
            <select aria-label="Kind" className={inputClass} value={kind} onChange={(e) => setKind(e.target.value)}>
              <option value="">Every kind</option>
              {KINDS.map((k) => (
                <option key={k} value={k}>
                  {k}
                </option>
              ))}
            </select>
            <select aria-label="State" className={inputClass} value={state} onChange={(e) => setState(e.target.value)}>
              <option value="">Any state</option>
              <option value="active">Active</option>
              <option value="expired">Expired</option>
              <option value="revoked">Revoked</option>
            </select>
            <button type="button" className={smallButton} aria-pressed={impersonation} onClick={() => setKind(impersonation ? "" : "impersonation")}>
              Impersonation sessions
            </button>
          </div>
        }
      />
      <Drawer open={ending !== null} onClose={() => setEnding(null)} title={ending?.kind === "impersonation" ? "End session" : "Revoke token"} testId="admin-token-revoke" size="md">
        {ending && (
          <ConfirmAction
            verb={ending.kind === "impersonation" ? "End session of" : "Revoke"}
            target={ending.label || ending.owner}
            consequence={
              ending.kind === "impersonation"
                ? `${ending.owner} is no longer acted as with this session. The next request it makes is refused.`
                : `The ${ending.kind} token of ${ending.owner} stops working. The next request it makes is refused.`
            }
            onConfirm={async (reason) => {
              await apiClient.revokeAdminToken(ending.id, reason);
            }}
            onSuccess={() => {
              setEnding(null);
              setVersion((v) => v + 1);
            }}
          />
        )}
      </Drawer>
    </div>
  );
}

// A SCIM token is made with the same route as every token. It is shown once. The server keeps only its hash.
function MintScim({ onMinted }: { onMinted: () => void }) {
  const [open, setOpen] = useState(false);
  const [label, setLabel] = useState("");
  const [days, setDays] = useState("90");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [minted, setMinted] = useState<{ token: string; label: string; expires_at: string } | null>(null);

  const mint = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const made = await apiClient.mintScimToken(label.trim(), Number(days) || undefined);
      setMinted(made);
      setOpen(false);
      setLabel("");
      onMinted();
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-3">
      <button type="button" className={buttonClass} onClick={() => setOpen((o) => !o)}>
        Mint SCIM token
      </button>
      {open && (
        <form onSubmit={mint} aria-label="Mint SCIM token" className="rounded border p-3 space-y-3 max-w-md">
          <p className="text-xs text-gray-500">A SCIM token lets the institution's directory create and change people. It is shown once, here, and never again.</p>
          <FormField id="scim-label" label="Label" required>
            {(props) => <input {...props} className={inputClass} value={label} onChange={(e) => setLabel(e.target.value)} placeholder="Registrar directory sync" />}
          </FormField>
          <FormField id="scim-days" label="Days it lasts">
            {(props) => <input {...props} type="number" min={1} max={365} className={inputClass} value={days} onChange={(e) => setDays(e.target.value)} />}
          </FormField>
          <button type="submit" className={buttonClass} disabled={busy || label.trim() === ""}>
            Create token
          </button>
          {error !== null && <ErrorState error={error} />}
        </form>
      )}
      {minted && (
        <Banner kind="warning" label="Token created">
          <p>
            Copy the token for {minted.label} now. It expires {when(minted.expires_at)} and cannot be shown again.
          </p>
          <code aria-label="New SCIM token" className="block mt-1 break-all rounded bg-white/60 dark:bg-black/30 p-2 text-xs">
            {minted.token}
          </code>
          <button type="button" className={`${smallButton} mt-2`} onClick={() => setMinted(null)}>
            I have copied it
          </button>
        </Banner>
      )}
    </div>
  );
}

// --- Agent access ---------------------------------------------------------------------------------------------------------

type Change = { setting: "mcp.disabled_tools" | "pages.disabled"; value: string[]; verb: string; target: string; consequence: string };

function AgentAccess() {
  const tools = useAsync(() => apiClient.adminTools(), true);
  const pages = useAsync(() => apiClient.adminPages(), true);
  const [change, setChange] = useState<Change | null>(null);

  if (tools.error) return <ErrorState error={tools.error} onRetry={() => void tools.reload()} />;
  if (pages.error) return <ErrorState error={pages.error} onRetry={() => void pages.reload()} />;
  if (!tools.data || !pages.data) return <Skeleton rows={5} />;

  const offTools = tools.data.tools.filter((t) => t.disabled).map((t) => t.name);
  const offPages = pages.data.pages.filter((p) => p.disabled).map((p) => `${p.app_slug}/${p.page_id}`);

  const toggleTool = (t: AdminToolRow) =>
    setChange({
      setting: "mcp.disabled_tools",
      value: t.disabled ? offTools.filter((n) => n !== t.name) : [...offTools, t.name],
      verb: t.disabled ? "Turn on" : "Turn off",
      target: t.name,
      consequence: t.disabled ? "AI tools can see and call it again." : "AI tools no longer see it, and a call to it is refused. Nothing else changes.",
    });

  const togglePage = (p: AdminPageRow) => {
    const key = `${p.app_slug}/${p.page_id}`;
    setChange({
      setting: "pages.disabled",
      value: p.disabled ? offPages.filter((k) => k !== key) : [...offPages, key],
      verb: p.disabled ? "Turn on" : "Turn off",
      target: key,
      consequence: p.disabled ? "The page runs again." : "The page shows a notice that an administrator turned it off, and answers no call.",
    });
  };

  return (
    <div className="space-y-8">
      <Banner kind="info" label="Agent access">
        These switches only remove capability. Turning something off never turns a check off. A person's own rights still apply to everything that stays on.
      </Banner>

      <section className="space-y-2" aria-label="Tools">
        <h3 className="text-base font-semibold">Tools</h3>
        <div className="overflow-x-auto border border-gray-200 dark:border-gray-800 rounded-lg">
          <table className="min-w-full text-left text-sm">
            <thead className="bg-gray-50 dark:bg-gray-900/80 font-medium">
              <tr>
                <th className="px-4 py-2">Tool</th>
                <th className="px-4 py-2">What it does</th>
                <th className="px-4 py-2">Kind</th>
                <th className="px-4 py-2">State</th>
                <th className="px-4 py-2" />
              </tr>
            </thead>
            <tbody>
              {tools.data.tools.map((t) => (
                <tr key={t.name} className="border-t border-gray-100 dark:border-gray-800">
                  <td className="px-4 py-2 font-mono text-xs">{t.name}</td>
                  <td className="px-4 py-2 text-xs">{t.description}</td>
                  <td className="px-4 py-2 text-xs">{t.read_only ? "Read only" : "Can change data"}</td>
                  <td className="px-4 py-2">{t.disabled ? <StatusBadge tone="bad" text="Off" /> : <StatusBadge tone="ok" text="On" />}</td>
                  <td className="px-4 py-2">
                    <button type="button" className={smallButton} aria-label={`${t.disabled ? "Turn on" : "Turn off"} ${t.name}`} onClick={() => toggleTool(t)}>
                      {t.disabled ? "Turn on" : "Turn off"}
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>

      <section className="space-y-2" aria-label="Custom pages">
        <h3 className="text-base font-semibold">Custom pages</h3>
        {pages.data.pages.length === 0 ? (
          <EmptyState title="No custom page" description="An AI tool can propose a custom page. A person approves it, and it then appears here." />
        ) : (
          <div className="overflow-x-auto border border-gray-200 dark:border-gray-800 rounded-lg">
            <table className="min-w-full text-left text-sm">
              <thead className="bg-gray-50 dark:bg-gray-900/80 font-medium">
                <tr>
                  <th className="px-4 py-2">App</th>
                  <th className="px-4 py-2">Page</th>
                  <th className="px-4 py-2">What it may do</th>
                  <th className="px-4 py-2">Approved by</th>
                  <th className="px-4 py-2">State</th>
                  <th className="px-4 py-2" />
                </tr>
              </thead>
              <tbody>
                {pages.data.pages.map((p) => (
                  <tr key={`${p.app_slug}/${p.page_id}`} className="border-t border-gray-100 dark:border-gray-800">
                    <td className="px-4 py-2 text-xs">{p.app_title || p.app_slug}</td>
                    <td className="px-4 py-2 text-xs">{p.title}</td>
                    <td className="px-4 py-2 text-xs">{p.grants}</td>
                    <td className="px-4 py-2 text-xs">{p.approved_by ?? "Not recorded"}</td>
                    <td className="px-4 py-2">{p.disabled ? <StatusBadge tone="bad" text="Off" /> : <StatusBadge tone="ok" text="On" />}</td>
                    <td className="px-4 py-2">
                      <button type="button" className={smallButton} aria-label={`${p.disabled ? "Turn on" : "Turn off"} ${p.app_slug}/${p.page_id}`} onClick={() => togglePage(p)}>
                        {p.disabled ? "Turn on" : "Turn off"}
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      <Drawer open={change !== null} onClose={() => setChange(null)} title="Agent access" testId="admin-agent-change" size="md">
        {change && (
          <ConfirmAction
            verb={change.verb}
            target={change.target}
            consequence={change.consequence}
            onConfirm={async (reason) => {
              await apiClient.updateSetting(change.setting, change.value, reason);
            }}
            onSuccess={() => {
              setChange(null);
              void tools.reload();
              void pages.reload();
            }}
          />
        )}
      </Drawer>
    </div>
  );
}
