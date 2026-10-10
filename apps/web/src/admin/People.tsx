import { useCallback, useMemo, useState, type FormEvent } from "react";
import {
  apiClient,
  type AdminUserDetail,
  type AdminUserRow,
  type NewAdminUser,
} from "../api";
import {
  ConfirmAction,
  DataTable,
  Drawer,
  ErrorState,
  FormField,
  PageHeader,
  Skeleton,
  StatusBadge,
  useAsync,
  type DataTableColumn,
} from "../ui";

const AFFILIATIONS = ["faculty", "student", "staff", "employee", "member", "affiliate", "alum"];

type FilterKey = "active" | "hold" | "platform_admin" | "unit_admin";
type ActionKind =
  | "hold"
  | "release"
  | "revoke-tokens"
  | "grant-unit-admin"
  | "revoke-unit-admin"
  | "grant-platform-admin"
  | "revoke-platform-admin";

export interface PeopleProps {
  /** Starts a session as this user. Absent when impersonation is not available. */
  onImpersonate?: (user: AdminUserRow) => void;
}

const inputClass =
  "w-full text-sm rounded border border-gray-300 dark:border-gray-700 p-2 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100";

export function People({ onImpersonate }: PeopleProps) {
  const [filters, setFilters] = useState<Record<FilterKey, boolean>>({
    active: false,
    hold: false,
    platform_admin: false,
    unit_admin: false,
  });
  const [selected, setSelected] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [version, setVersion] = useState(0);

  const filterValues = useMemo(() => ({ ...filters, version }), [filters, version]);

  const load = useCallback(
    async (cursor?: string, search?: string, f?: Record<string, unknown>) => {
      const params: Record<string, string | undefined> = { cursor, search, limit: "50" };
      for (const key of ["active", "hold", "platform_admin", "unit_admin"] as FilterKey[]) {
        if (f?.[key]) params[key] = "true";
      }
      const res = await apiClient.adminListUsers(params);
      return { rows: res.users, next_cursor: res.next_cursor ?? undefined };
    },
    []
  );

  const columns: DataTableColumn<AdminUserRow>[] = [
    { id: "user_name", header: "User", accessorKey: "user_name" },
    { id: "display_name", header: "Name", accessorKey: "display_name" },
    { id: "affiliation", header: "Affiliation", accessorKey: "affiliation" },
    {
      id: "units",
      header: "Units",
      cell: ({ row }) => row.original.units.map((u) => u.name).join(", ") || "None",
    },
    {
      id: "status",
      header: "Status",
      cell: ({ row }) =>
        row.original.hold ? (
          <StatusBadge tone="warn" text="On hold" />
        ) : row.original.active ? (
          <StatusBadge tone="ok" text="Active" />
        ) : (
          <StatusBadge tone="neutral" text="Inactive" />
        ),
    },
    {
      id: "roles",
      header: "Admin roles",
      cell: ({ row }) =>
        [row.original.platform_admin ? "Platform Admin" : "", row.original.unit_admin_of.length ? "Org Unit Admin" : ""]
          .filter(Boolean)
          .join(", ") || "None",
    },
    { id: "active_agent_tokens", header: "Agent tokens", accessorKey: "active_agent_tokens" },
  ];

  const toggle = (key: FilterKey, label: string) => (
    <label key={key} className="inline-flex items-center gap-1.5 text-xs text-gray-700 dark:text-gray-300">
      <input
        type="checkbox"
        checked={filters[key]}
        onChange={(e) => setFilters((f) => ({ ...f, [key]: e.target.checked }))}
      />
      {label}
    </label>
  );

  return (
    <div className="space-y-6" data-testid="admin-people">
      <PageHeader
        title="People"
        description="Everyone the platform knows, their appointments, and their access"
        actions={
          <button
            type="button"
            data-testid="admin-user-create-open"
            onClick={() => setCreating(true)}
            className="px-3 py-1.5 text-sm font-medium rounded-md bg-blue-600 text-white hover:bg-blue-700"
          >
            Add user
          </button>
        }
      />
      <DataTable<AdminUserRow>
        columns={columns}
        load={load}
        rowKey="id"
        onRowOpen={(row) => setSelected(row.id)}
        filterValues={filterValues}
        filters={
          <div className="flex flex-wrap items-center gap-3">
            {toggle("active", "Active")}
            {toggle("hold", "On hold")}
            {toggle("platform_admin", "Platform Admin")}
            {toggle("unit_admin", "Org Unit Admin")}
          </div>
        }
      />
      <Drawer open={creating} onClose={() => setCreating(false)} title="Add user">
        <CreateUserForm
          onCreated={() => {
            setCreating(false);
            setVersion((v) => v + 1);
          }}
        />
      </Drawer>
      <Drawer open={selected !== null} onClose={() => setSelected(null)} title="User">
        {selected && (
          <UserDetail
            id={selected}
            onImpersonate={onImpersonate}
            onChanged={() => setVersion((v) => v + 1)}
          />
        )}
      </Drawer>
    </div>
  );
}

function CreateUserForm({ onCreated }: { onCreated: () => void }) {
  const [form, setForm] = useState<NewAdminUser>({
    userName: "",
    name: "",
    email: "",
    affiliation: "staff",
    department: "",
    title: "",
    reason: "",
  });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const set = (key: keyof NewAdminUser) => (value: string) => setForm((f) => ({ ...f, [key]: value }));
  const valid = form.userName.trim() !== "" && form.reason.trim() !== "";

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!valid || busy) return;
    setBusy(true);
    setError(null);
    try {
      await apiClient.adminCreateUser({ ...form, email: form.email || form.userName });
      onCreated();
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Could not add the user");
    } finally {
      setBusy(false);
    }
  };

  const text = (key: keyof NewAdminUser, label: string, required = false) => (
    <FormField id={`create-${key}`} label={label} required={required}>
      {(props) => (
        <input {...props} className={inputClass} value={form[key]} onChange={(e) => set(key)(e.target.value)} />
      )}
    </FormField>
  );

  return (
    <form data-testid="admin-user-create" onSubmit={submit} className="space-y-4">
      {error && <ErrorState message={error} />}
      {text("userName", "User name", true)}
      {text("name", "Name")}
      {text("email", "Email")}
      <FormField id="create-affiliation" label="Affiliation">
        {(props) => (
          <select {...props} className={inputClass} value={form.affiliation} onChange={(e) => set("affiliation")(e.target.value)}>
            {AFFILIATIONS.map((a) => (
              <option key={a} value={a}>
                {a}
              </option>
            ))}
          </select>
        )}
      </FormField>
      {text("department", "Department")}
      {text("title", "Title")}
      {text("reason", "Reason", true)}
      <div className="flex justify-end">
        <button
          type="submit"
          disabled={!valid || busy}
          className="px-4 py-2 text-sm font-medium rounded-md bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50"
        >
          Create user
        </button>
      </div>
    </form>
  );
}

function UserDetail({
  id,
  onImpersonate,
  onChanged,
}: {
  id: string;
  onImpersonate?: (user: AdminUserRow) => void;
  onChanged: () => void;
}) {
  const fetchDetail = useCallback(() => apiClient.adminGetUser(id), [id]);
  const { data, error, loading, reload } = useAsync<AdminUserDetail>(fetchDetail, true);
  const units = useAsync(useCallback(() => apiClient.listOrganizations(), []), true);
  const [action, setAction] = useState<ActionKind | null>(null);
  const [unit, setUnit] = useState("");

  if (loading && !data) return <Skeleton rows={6} />;
  if (error || !data) {
    return <ErrorState error={error ?? new Error("Could not load the user")} onRetry={() => reload().catch(() => {})} />;
  }

  const u = data.user;
  const rootId = (units.data ?? []).find((o) => !o.parent_id)?.id ?? "";
  const unitAdminAppointments = data.appointments.filter((a) => a.scoped_affiliation === "unit_admin");

  const done = () => {
    setAction(null);
    onChanged();
    reload().catch(() => {});
  };

  const run = async (reason: string): Promise<void> => {
    switch (action) {
      case "hold":
        await apiClient.adminHoldUser(id, true, reason);
        return;
      case "release":
        await apiClient.adminHoldUser(id, false, reason);
        return;
      case "revoke-tokens":
        await apiClient.adminRevokeTokens(id, reason);
        return;
      case "grant-unit-admin":
        await apiClient.appointOrgAdmin(unit, { eppn: u.user_name, scoped_affiliation: "unit_admin", reason });
        return;
      case "revoke-unit-admin":
        await apiClient.revokeAppointment(unit, u.user_name, "unit_admin", reason);
        return;
      case "grant-platform-admin":
        await apiClient.appointOrgAdmin(rootId, { eppn: u.user_name, scoped_affiliation: "platform_admin", reason });
        return;
      case "revoke-platform-admin":
        await apiClient.revokeAppointment(rootId, u.user_name, "platform_admin", reason);
        return;
      default:
    }
  };

  const needsUnit = action === "grant-unit-admin" || action === "revoke-unit-admin";
  const unitChoices =
    action === "revoke-unit-admin"
      ? unitAdminAppointments.map((a) => ({ id: a.organization_id, name: a.unit }))
      : (units.data ?? []).map((o) => ({ id: o.id, name: o.name }));

  const describe: Record<ActionKind, { verb: string; target: string; consequence: string }> = {
    hold: { verb: "Confirm", target: "hold", consequence: `${u.user_name} is signed out of every token until the hold is released.` },
    release: { verb: "Confirm", target: "release", consequence: `${u.user_name} can use their tokens again.` },
    "revoke-tokens": { verb: "Confirm", target: "token revocation", consequence: "Every agent and impersonation token this user holds stops working." },
    "grant-unit-admin": { verb: "Confirm", target: "appointment", consequence: "The user becomes an Org Unit Admin of the chosen unit." },
    "revoke-unit-admin": { verb: "Confirm", target: "revocation", consequence: "The user stops being an Org Unit Admin of the chosen unit." },
    "grant-platform-admin": { verb: "Confirm", target: "appointment", consequence: "The user becomes a Platform Admin with access to every unit." },
    "revoke-platform-admin": { verb: "Confirm", target: "revocation", consequence: "The user stops being a Platform Admin." },
  };

  const button = (kind: ActionKind, label: string) => (
    <button
      key={kind}
      type="button"
      data-testid={`admin-user-action-${kind}`}
      onClick={() => {
        setAction(kind);
        setUnit("");
      }}
      className="px-3 py-1.5 text-xs font-medium rounded border border-gray-300 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800 text-gray-700 dark:text-gray-300"
    >
      {label}
    </button>
  );

  return (
    <div data-testid="admin-user-detail" className="space-y-6">
      <section className="space-y-1">
        <div className="text-base font-semibold text-gray-900 dark:text-gray-100">{u.display_name}</div>
        <div className="text-sm text-gray-600 dark:text-gray-400">{u.user_name}</div>
        <div className="flex gap-2 pt-1">
          {u.hold ? <StatusBadge tone="warn" text="On hold" /> : u.active ? <StatusBadge tone="ok" text="Active" /> : <StatusBadge tone="neutral" text="Inactive" />}
          {u.platform_admin && <StatusBadge tone="neutral" text="Platform Admin" />}
        </div>
      </section>

      <section className="flex flex-wrap gap-2">
        {u.hold ? button("release", "Release") : button("hold", "Hold")}
        {button("revoke-tokens", "Revoke tokens")}
        {button("grant-unit-admin", "Grant Org Unit Admin")}
        {unitAdminAppointments.length > 0 && button("revoke-unit-admin", "Revoke Org Unit Admin")}
        {u.platform_admin ? button("revoke-platform-admin", "Revoke Platform Admin") : button("grant-platform-admin", "Grant Platform Admin")}
        {onImpersonate && (
          <button
            type="button"
            data-testid="admin-user-impersonate"
            onClick={() => onImpersonate(u)}
            className="px-3 py-1.5 text-xs font-medium rounded border border-gray-300 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800 text-gray-700 dark:text-gray-300"
          >
            Impersonate
          </button>
        )}
      </section>

      {action && (
        <section className="space-y-3 border border-gray-200 dark:border-gray-800 rounded-lg p-3" data-testid="admin-user-action">
          {needsUnit && (
            <FormField id="action-unit" label="Unit" required>
              {(props) => (
                <select {...props} className={inputClass} value={unit} onChange={(e) => setUnit(e.target.value)}>
                  <option value="">Choose a unit</option>
                  {unitChoices.map((o) => (
                    <option key={o.id} value={o.id}>
                      {o.name}
                    </option>
                  ))}
                </select>
              )}
            </FormField>
          )}
          {(!needsUnit || unit) && (
            <ConfirmAction {...describe[action]} onConfirm={run} onSuccess={done} />
          )}
        </section>
      )}

      <section className="space-y-2">
        <h3 className="text-sm font-semibold text-gray-900 dark:text-gray-100">Appointments</h3>
        {data.appointments.length === 0 ? (
          <p className="text-sm text-gray-500">No appointments.</p>
        ) : (
          <ul className="text-sm space-y-1" data-testid="admin-user-appointments">
            {data.appointments.map((a) => (
              <li key={`${a.organization_id}-${a.scoped_affiliation}`}>
                {a.scoped_affiliation} in {a.unit} <span className="text-xs text-gray-500">({a.source})</span>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="space-y-2">
        <h3 className="text-sm font-semibold text-gray-900 dark:text-gray-100">Tokens</h3>
        {data.tokens.length === 0 ? (
          <p className="text-sm text-gray-500">No tokens.</p>
        ) : (
          <ul className="text-sm space-y-1" data-testid="admin-user-tokens">
            {data.tokens.map((t) => (
              <li key={t.id}>
                {t.kind}: {t.label} {t.revoked ? "(revoked)" : `(expires ${t.expires_at.slice(0, 10)})`}
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="space-y-2">
        <h3 className="text-sm font-semibold text-gray-900 dark:text-gray-100">Workspaces</h3>
        {data.workspace_memberships.length === 0 ? (
          <p className="text-sm text-gray-500">No workspace memberships.</p>
        ) : (
          <ul className="text-sm space-y-1">
            {data.workspace_memberships.map((m) => (
              <li key={m.workspace_id}>
                {m.workspace_id} ({m.role})
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="space-y-2">
        <h3 className="text-sm font-semibold text-gray-900 dark:text-gray-100">Recent ledger entries</h3>
        {data.ledger.length === 0 ? (
          <p className="text-sm text-gray-500">None.</p>
        ) : (
          <ul className="text-sm space-y-1">
            {data.ledger.map((l, i) => (
              <li key={i}>
                {l.decision_type}: {l.rationale}
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
