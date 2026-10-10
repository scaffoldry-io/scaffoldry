import { useCallback, useEffect, useMemo, useState, type FormEvent } from "react";
import {
  apiClient,
  type AdminAppRow,
  type AdminPatchWorkspacePayload,
  type AdminUserRow,
  type AdminWorkspaceDetail,
  type AdminWorkspaceRow,
  type OrganizationNode,
} from "../api";
import {
  DataTable,
  Drawer,
  EmptyState,
  ErrorState,
  FormField,
  PageHeader,
  Skeleton,
  StatusBadge,
  useAsync,
  type DataTableColumn,
} from "../ui";

const CLASSIFICATIONS = ["Public", "Internal", "Restricted", "FERPA Sensitive"];
const VISIBILITIES = ["restricted", "departmental", "institutional"];

const inputClass =
  "w-full text-sm rounded border border-gray-300 dark:border-gray-700 p-2 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100";

export function Workspaces() {
  const [activeTab, setActiveTab] = useState<"workspaces" | "apps">("workspaces");
  const [selectedWorkspaceId, setSelectedWorkspaceId] = useState<string | null>(null);
  const [detail, setDetail] = useState<AdminWorkspaceDetail | null>(null);
  const [loadingDetail, setLoadingDetail] = useState(false);
  const [detailError, setDetailError] = useState<string | null>(null);

  // Edit controls inside drawer
  const [editUnitId, setEditUnitId] = useState<string>("");
  const [editClassification, setEditClassification] = useState<string>("");
  const [editVisibility, setEditVisibility] = useState<string>("");
  const [editReason, setEditReason] = useState<string>("");
  const [savingEdit, setSavingEdit] = useState(false);
  const [editMessage, setEditMessage] = useState<{ text: string; error?: boolean } | null>(null);

  // Transfer ownership inside drawer
  const [transferSearch, setTransferSearch] = useState<string>("");
  const [userSearchResults, setUserSearchResults] = useState<AdminUserRow[]>([]);
  const [selectedNewOwner, setSelectedNewOwner] = useState<string>("");
  const [transferReason, setTransferReason] = useState<string>("");
  const [transferring, setTransferring] = useState(false);
  const [transferMessage, setTransferMessage] = useState<{ text: string; error?: boolean } | null>(null);

  const [version, setVersion] = useState(0);

  // Load organization units for the unit dropdown
  const { data: orgs } = useAsync<OrganizationNode[]>(() => apiClient.getOrgs(), true);

  // Load selected workspace details
  const loadDetail = useCallback(async (id: string) => {
    setLoadingDetail(true);
    setDetailError(null);
    setEditMessage(null);
    setTransferMessage(null);
    try {
      const data = await apiClient.adminGetWorkspace(id);
      setDetail(data);
      setEditUnitId(data.organization_id ?? "");
      setEditClassification(data.classification);
      setEditVisibility(data.visibility);
      setEditReason("");
      setSelectedNewOwner("");
      setTransferReason("");
      setTransferSearch("");
      setUserSearchResults([]);
    } catch (err: any) {
      setDetailError(err.message || "Failed to load workspace details");
    } finally {
      setLoadingDetail(false);
    }
  }, []);

  useEffect(() => {
    if (selectedWorkspaceId) {
      loadDetail(selectedWorkspaceId);
    } else {
      setDetail(null);
    }
  }, [selectedWorkspaceId, loadDetail]);

  // People search for ownership transfer
  useEffect(() => {
    if (!transferSearch.trim() || transferSearch.length < 2) {
      setUserSearchResults([]);
      return;
    }
    let active = true;
    const timer = setTimeout(() => {
      apiClient
        .adminListUsers({ search: transferSearch.trim(), limit: "10" })
        .then((res) => {
          if (active) {
            setUserSearchResults(res.users);
          }
        })
        .catch(() => {
          if (active) setUserSearchResults([]);
        });
    }, 250);

    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [transferSearch]);

  // Handle Save Changes in Drawer
  const handleSaveChanges = async (e: FormEvent) => {
    e.preventDefault();
    if (!detail) return;

    const trimmedReason = editReason.trim();
    if (!trimmedReason) {
      setEditMessage({ text: "A reason is required to update workspace settings", error: true });
      return;
    }

    // Build payload containing ONLY changed fields
    const payload: AdminPatchWorkspacePayload = { reason: trimmedReason };
    let hasChanges = false;

    const currentOrgId = detail.organization_id ?? "";
    if (editUnitId !== currentOrgId && editUnitId !== "") {
      payload.organization_id = editUnitId;
      hasChanges = true;
    }
    if (editClassification !== detail.classification) {
      payload.data_classification = editClassification;
      hasChanges = true;
    }
    if (editVisibility !== detail.visibility) {
      payload.visibility = editVisibility;
      hasChanges = true;
    }

    if (!hasChanges) {
      setEditMessage({ text: "No fields were modified", error: true });
      return;
    }

    setSavingEdit(true);
    setEditMessage(null);
    try {
      await apiClient.adminPatchWorkspace(detail.id, payload);
      setEditMessage({ text: "Workspace updated successfully" });
      setVersion((v) => v + 1);
      await loadDetail(detail.id);
    } catch (err: any) {
      setEditMessage({ text: err.message || "Failed to update workspace", error: true });
    } finally {
      setSavingEdit(false);
    }
  };

  // Handle Transfer Ownership in Drawer
  const handleTransferOwnership = async (e: FormEvent) => {
    e.preventDefault();
    if (!detail) return;

    if (!selectedNewOwner) {
      setTransferMessage({ text: "Please select a user to transfer ownership to", error: true });
      return;
    }
    const trimmedReason = transferReason.trim();
    if (!trimmedReason) {
      setTransferMessage({ text: "A reason is required to transfer ownership", error: true });
      return;
    }

    setTransferring(true);
    setTransferMessage(null);
    try {
      await apiClient.adminTransferWorkspaceOwnership(detail.id, selectedNewOwner, trimmedReason);
      setTransferMessage({ text: `Ownership successfully transferred to ${selectedNewOwner}` });
      setVersion((v) => v + 1);
      await loadDetail(detail.id);
    } catch (err: any) {
      setTransferMessage({ text: err.message || "Failed to transfer ownership", error: true });
    } finally {
      setTransferring(false);
    }
  };

  // Workspaces Data Table loader
  const loadWorkspaces = useCallback(
    async (cursor?: string, search?: string) => {
      const res = await apiClient.adminListWorkspaces({
        cursor,
        search,
        limit: "50",
      });
      return { rows: res.rows, next_cursor: res.next_cursor ?? undefined };
    },
    []
  );

  // Apps Data Table loader
  const loadApps = useCallback(
    async (cursor?: string, search?: string) => {
      const res = await apiClient.adminListApps({
        cursor,
        search,
        limit: "50",
      });
      return { rows: res.rows, next_cursor: res.next_cursor ?? undefined };
    },
    []
  );

  const workspaceColumns = useMemo<DataTableColumn<AdminWorkspaceRow, unknown>[]>(
    () => [
      {
        id: "name",
        header: "Workspace",
        cell: ({ row }) => (
          <div>
            <div className="font-semibold text-gray-900 dark:text-gray-100">{row.original.name}</div>
            <div className="text-xs text-gray-500 font-mono">{row.original.code}</div>
          </div>
        ),
      },
      {
        id: "unit_name",
        header: "Organization Unit",
        cell: ({ row }) => <span className="text-sm">{row.original.unit_name || "—"}</span>,
      },
      {
        id: "classification",
        header: "Classification",
        cell: ({ row }) => (
          <StatusBadge
            text={row.original.classification}
            tone={
              row.original.classification === "Public"
                ? "neutral"
                : row.original.classification === "Internal"
                ? "ok"
                : "warn"
            }
          />
        ),
      },
      {
        id: "visibility",
        header: "Visibility",
        cell: ({ row }) => (
          <span className="text-xs font-mono uppercase bg-gray-100 dark:bg-gray-800 px-2 py-0.5 rounded text-gray-700 dark:text-gray-300">
            {row.original.visibility}
          </span>
        ),
      },
      {
        id: "owners",
        header: "Owners",
        cell: ({ row }) => (
          <span className="text-sm text-gray-700 dark:text-gray-300 truncate max-w-xs block">
            {row.original.owners.join(", ") || "—"}
          </span>
        ),
      },
      {
        id: "counts",
        header: "Inventory",
        cell: ({ row }) => (
          <div className="text-xs space-y-0.5 text-gray-600 dark:text-gray-400">
            <div>Apps: {row.original.app_count}</div>
            <div>Records: {row.original.record_count}</div>
            <div>Collaborators: {row.original.collaborator_count}</div>
          </div>
        ),
      },
      {
        id: "actions",
        header: "",
        cell: ({ row }) => (
          <button
            type="button"
            onClick={() => setSelectedWorkspaceId(row.original.id)}
            data-testid={`view-workspace-${row.original.id}`}
            className="text-xs px-2.5 py-1 rounded border border-gray-300 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800 text-gray-700 dark:text-gray-300"
          >
            Manage
          </button>
        ),
      },
    ],
    []
  );

  const appColumns = useMemo<DataTableColumn<AdminAppRow, unknown>[]>(
    () => [
      {
        id: "title",
        header: "Application",
        cell: ({ row }) => (
          <div>
            <div className="font-semibold text-gray-900 dark:text-gray-100">{row.original.title}</div>
            <div className="text-xs text-gray-500 font-mono">{row.original.slug}</div>
          </div>
        ),
      },
      {
        id: "workspace",
        header: "Workspace ID",
        cell: ({ row }) => <span className="text-xs font-mono text-gray-600 dark:text-gray-400">{row.original.workspace || "—"}</span>,
      },
      {
        id: "version",
        header: "Version",
        cell: ({ row }) => <span className="text-xs font-mono">{row.original.version}</span>,
      },
      {
        id: "counts",
        header: "Structure",
        cell: ({ row }) => (
          <div className="text-xs text-gray-600 dark:text-gray-400">
            {row.original.table_count} tables · {row.original.page_count} pages
          </div>
        ),
      },
      {
        id: "records",
        header: "Records Per Table",
        cell: ({ row }) => {
          const entries = Object.entries(row.original.record_count_per_table ?? {});
          if (entries.length === 0) return <span className="text-xs text-gray-400">0</span>;
          return (
            <div className="text-xs font-mono space-y-0.5">
              {entries.map(([tbl, count]) => (
                <div key={tbl}>
                  {tbl}: <span className="font-semibold">{count}</span>
                </div>
              ))}
            </div>
          );
        },
      },
      {
        id: "actions",
        header: "",
        cell: ({ row }) => (
          <a
            href={`/apps/${row.original.slug}`}
            data-testid={`open-app-${row.original.slug}`}
            className="text-xs font-medium text-amber-600 dark:text-amber-400 hover:underline inline-flex items-center gap-1"
          >
            Open ↗
          </a>
        ),
      },
    ],
    []
  );

  const filterValues = useMemo(() => ({ version }), [version]);

  return (
    <div className="space-y-6" data-testid="admin-workspaces-panel">
      <PageHeader
        title="Workspaces and Applications"
        description="Govern institutional workspaces, data classification overlays, visibility boundaries, and application inventory."
      />

      <div className="flex gap-2 border-b border-gray-200 dark:border-gray-800">
        <button
          type="button"
          onClick={() => setActiveTab("workspaces")}
          data-testid="admin-workspaces-subtab-workspaces"
          className={`px-4 py-2 text-sm font-medium border-b-2 -mb-px transition-colors cursor-pointer ${
            activeTab === "workspaces"
              ? "border-amber-600 text-amber-700 dark:text-amber-400 font-semibold"
              : "border-transparent text-gray-500 hover:text-gray-700 dark:hover:text-gray-300"
          }`}
        >
          Workspaces
        </button>
        <button
          type="button"
          onClick={() => setActiveTab("apps")}
          data-testid="admin-workspaces-subtab-apps"
          className={`px-4 py-2 text-sm font-medium border-b-2 -mb-px transition-colors cursor-pointer ${
            activeTab === "apps"
              ? "border-amber-600 text-amber-700 dark:text-amber-400 font-semibold"
              : "border-transparent text-gray-500 hover:text-gray-700 dark:hover:text-gray-300"
          }`}
        >
          Applications
        </button>
      </div>

      {activeTab === "workspaces" ? (
        <DataTable
          columns={workspaceColumns}
          load={loadWorkspaces}
          rowKey="id"
          onRowOpen={(row) => setSelectedWorkspaceId(row.id)}
          emptyState={
            <EmptyState
              title="No workspaces found"
              description="No institutional workspaces match your query."
            />
          }
          filterValues={filterValues}
        />
      ) : (
        <DataTable
          columns={appColumns}
          load={loadApps}
          rowKey="slug"
          emptyState={
            <EmptyState
              title="No applications found"
              description="No applications are currently registered in this workspace inventory."
            />
          }
          filterValues={filterValues}
        />
      )}

      {/* Detail Drawer */}
      <Drawer
        open={Boolean(selectedWorkspaceId)}
        onClose={() => setSelectedWorkspaceId(null)}
        title={detail ? detail.name : "Workspace Details"}
      >
        <div data-testid="admin-workspace-detail" className="space-y-6">
          {loadingDetail && (
            <div className="space-y-4">
              <Skeleton className="h-6 w-3/4" />
              <Skeleton className="h-24 w-full" />
              <Skeleton className="h-48 w-full" />
            </div>
          )}

          {detailError && <ErrorState message="Failed to load details" next={detailError} />}

          {detail && (
            <>
              {/* Overview summary */}
              <div className="p-4 bg-gray-50 dark:bg-gray-800/50 rounded-lg space-y-2 text-sm">
                <div className="flex justify-between text-xs text-gray-500">
                  <span>ID: <span className="font-mono">{detail.id}</span></span>
                  <span>Code: <span className="font-mono">{detail.code}</span></span>
                </div>
                {detail.description && (
                  <p className="text-gray-700 dark:text-gray-300 text-xs italic">{detail.description}</p>
                )}
                <div className="flex items-center gap-2 pt-2">
                  <span className="text-xs text-gray-500">Owners:</span>
                  <span className="text-xs font-medium text-gray-900 dark:text-gray-100">
                    {detail.owners.join(", ") || "None"}
                  </span>
                </div>
              </div>

              {/* Edit Controls: Unit, Classification, Visibility */}
              <form onSubmit={handleSaveChanges} className="space-y-4 border-t border-gray-200 dark:border-gray-800 pt-4">
                <h3 className="text-sm font-semibold text-gray-900 dark:text-gray-100">Governance & Classification</h3>

                <FormField id="edit-unit" label="Organization Unit">
                  {(props) => (
                    <select
                      {...props}
                      data-testid="edit-workspace-unit"
                      value={editUnitId}
                      onChange={(e) => setEditUnitId(e.target.value)}
                      className={inputClass}
                    >
                      <option value="">(No specific unit assigned)</option>
                      {(orgs ?? []).map((o) => (
                        <option key={o.id} value={o.id}>
                          {o.name} ({o.code})
                        </option>
                      ))}
                    </select>
                  )}
                </FormField>

                <FormField id="edit-classification" label="Data Classification">
                  {(props) => (
                    <select
                      {...props}
                      data-testid="edit-workspace-classification"
                      value={editClassification}
                      onChange={(e) => setEditClassification(e.target.value)}
                      className={inputClass}
                    >
                      {CLASSIFICATIONS.map((cls) => (
                        <option key={cls} value={cls}>
                          {cls}
                        </option>
                      ))}
                    </select>
                  )}
                </FormField>

                <FormField id="edit-visibility" label="Visibility">
                  {(props) => (
                    <select
                      {...props}
                      data-testid="edit-workspace-visibility"
                      value={editVisibility}
                      onChange={(e) => setEditVisibility(e.target.value)}
                      className={inputClass}
                    >
                      {VISIBILITIES.map((v) => (
                        <option key={v} value={v}>
                          {v}
                        </option>
                      ))}
                    </select>
                  )}
                </FormField>

                <FormField id="edit-reason" label="Reason for change" required>
                  {(props) => (
                    <input
                      {...props}
                      data-testid="edit-workspace-reason"
                      type="text"
                      placeholder="Rationale recorded in the immutable audit ledger..."
                      value={editReason}
                      onChange={(e) => setEditReason(e.target.value)}
                      className={inputClass}
                    />
                  )}
                </FormField>

                {editMessage && (
                  <div
                    data-testid="edit-workspace-message"
                    className={`text-xs p-2 rounded ${
                      editMessage.error
                        ? "bg-rose-50 text-rose-700 dark:bg-rose-950/40 dark:text-rose-300"
                        : "bg-emerald-50 text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300"
                    }`}
                  >
                    {editMessage.text}
                  </div>
                )}

                <button
                  type="submit"
                  disabled={savingEdit}
                  data-testid="save-workspace-settings-btn"
                  className="w-full text-xs font-semibold py-2 px-3 rounded bg-amber-600 hover:bg-amber-700 text-white cursor-pointer disabled:opacity-50"
                >
                  {savingEdit ? "Saving changes..." : "Save Governance Settings"}
                </button>
              </form>

              {/* Transfer Ownership */}
              <form onSubmit={handleTransferOwnership} className="space-y-4 border-t border-gray-200 dark:border-gray-800 pt-4">
                <h3 className="text-sm font-semibold text-gray-900 dark:text-gray-100">Transfer Ownership</h3>
                <p className="text-xs text-gray-500">
                  Transfers primary workspace ownership. The new owner must be an active person. Existing owners are demoted to administrators.
                </p>

                <FormField id="transfer-search" label="Search user for ownership transfer">
                  {(props) => (
                    <input
                      {...props}
                      data-testid="transfer-search-input"
                      type="text"
                      placeholder="Type person name or email..."
                      value={transferSearch}
                      onChange={(e) => setTransferSearch(e.target.value)}
                      className={inputClass}
                    />
                  )}
                </FormField>

                {userSearchResults.length > 0 && (
                  <div className="border border-gray-200 dark:border-gray-700 rounded max-h-36 overflow-y-auto divide-y divide-gray-100 dark:divide-gray-800">
                    {userSearchResults.map((u) => (
                      <button
                        type="button"
                        key={u.id}
                        onClick={() => {
                          setSelectedNewOwner(u.user_name);
                          setTransferSearch(`${u.display_name} (${u.user_name})`);
                          setUserSearchResults([]);
                        }}
                        data-testid={`select-user-${u.user_name}`}
                        className="w-full text-left p-2 text-xs hover:bg-gray-50 dark:hover:bg-gray-800 flex justify-between"
                      >
                        <span className="font-medium text-gray-900 dark:text-gray-100">{u.display_name}</span>
                        <span className="text-gray-500 font-mono">{u.user_name}</span>
                      </button>
                    ))}
                  </div>
                )}

                {selectedNewOwner && (
                  <div className="text-xs text-amber-700 dark:text-amber-400 font-medium">
                    Selected new owner: <span className="font-mono">{selectedNewOwner}</span>
                  </div>
                )}

                <FormField id="transfer-reason" label="Reason for ownership transfer" required>
                  {(props) => (
                    <input
                      {...props}
                      data-testid="transfer-reason-input"
                      type="text"
                      placeholder="Administrative rationale for transfer..."
                      value={transferReason}
                      onChange={(e) => setTransferReason(e.target.value)}
                      className={inputClass}
                    />
                  )}
                </FormField>

                {transferMessage && (
                  <div
                    data-testid="transfer-message"
                    className={`text-xs p-2 rounded ${
                      transferMessage.error
                        ? "bg-rose-50 text-rose-700 dark:bg-rose-950/40 dark:text-rose-300"
                        : "bg-emerald-50 text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300"
                    }`}
                  >
                    {transferMessage.text}
                  </div>
                )}

                <button
                  type="submit"
                  disabled={transferring || !selectedNewOwner}
                  data-testid="confirm-transfer-btn"
                  className="w-full text-xs font-semibold py-2 px-3 rounded bg-gray-800 hover:bg-gray-900 dark:bg-gray-700 dark:hover:bg-gray-600 text-white cursor-pointer disabled:opacity-50"
                >
                  {transferring ? "Transferring..." : "Confirm Ownership Transfer"}
                </button>
              </form>

              {/* Collaborators list */}
              <div className="space-y-2 border-t border-gray-200 dark:border-gray-800 pt-4">
                <h3 className="text-sm font-semibold text-gray-900 dark:text-gray-100">
                  Collaborators ({detail.collaborators.length})
                </h3>
                <div className="space-y-1 max-h-40 overflow-y-auto">
                  {detail.collaborators.map((c) => (
                    <div
                      key={c.id}
                      className="p-2 text-xs bg-gray-50 dark:bg-gray-800/40 rounded flex justify-between items-center"
                    >
                      <div>
                        <div className="font-medium text-gray-900 dark:text-gray-100">{c.name || c.eppn}</div>
                        <div className="text-gray-500 font-mono text-[11px]">{c.eppn}</div>
                      </div>
                      <span className="uppercase font-mono px-1.5 py-0.5 rounded bg-gray-200 dark:bg-gray-700 text-gray-700 dark:text-gray-300 text-[10px]">
                        {c.role}
                      </span>
                    </div>
                  ))}
                </div>
              </div>

              {/* Apps in workspace */}
              <div className="space-y-2 border-t border-gray-200 dark:border-gray-800 pt-4">
                <h3 className="text-sm font-semibold text-gray-900 dark:text-gray-100">
                  Applications ({detail.apps.length})
                </h3>
                <div className="space-y-1 max-h-40 overflow-y-auto">
                  {detail.apps.map((a) => (
                    <div
                      key={a.slug}
                      className="p-2 text-xs bg-gray-50 dark:bg-gray-800/40 rounded flex justify-between items-center"
                    >
                      <div>
                        <div className="font-medium text-gray-900 dark:text-gray-100">{a.title}</div>
                        <div className="text-gray-500 font-mono text-[11px]">{a.slug}</div>
                      </div>
                      <a
                        href={`/apps/${a.slug}`}
                        data-testid={`drawer-open-app-${a.slug}`}
                        className="text-amber-600 dark:text-amber-400 hover:underline font-medium text-xs"
                      >
                        Open ↗
                      </a>
                    </div>
                  ))}
                </div>
              </div>
            </>
          )}
        </div>
      </Drawer>
    </div>
  );
}
