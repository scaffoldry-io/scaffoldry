export interface AdminOverviewData {
  server: {
    version: string;
    applied_migrations: string[];
    database_worker_count: number;
  };
  people: {
    active: number;
    on_hold: number;
    inactive: number;
    platform_admins: number;
    active_tokens: Record<string, number>;
  };
  organization: {
    unit_count: number;
  };
  workspaces: {
    workspace_count: number;
    app_count: number;
  };
  processes: {
    waiting_instance_count: number;
  };
  ledger: {
    entry_count: number;
    head_hash: string;
  };
  jobs?: {
    queue_depth: number;
    oldest_queued_age_secs: number | null;
    failed_last_24h: number;
  };
}

export interface AdminJobRow {
  id: string;
  kind: string;
  owner: string;
  state: "queued" | "running" | "done" | "failed" | "cancelled";
  progress: number | null;
  attempts: number;
  created: string;
  age: number;
  last_log_line: string | null;
  error?: string | null;
}

export interface AdminJobsResponse {
  rows: AdminJobRow[];
  next_cursor: string | null;
}

export interface AdminJobDetail extends AdminJobRow {
  progress_done: number;
  progress_total: number | null;
  result?: any;
  error?: string | null;
  log: string[];
  cancel_requested: boolean;
  created_by: string;
  created_at: string;
  started_at?: string | null;
  finished_at?: string | null;
}

import { Collaborator, LedgerEntryItem, OrganizationNode, OrgRole, Workspace } from "./types";
export type { OrganizationNode };

export interface PolicyRef {
  id: string;
  description: string;
}

export interface AdminWorkspaceRow {
  id: string;
  name: string;
  code: string;
  unit_name: string;
  classification: string;
  visibility: string;
  owners: string[];
  collaborator_count: number;
  app_count: number;
  record_count: number;
  created: string;
  organization_id?: string;
  description?: string;
}

export interface AdminAppRow {
  slug: string;
  title: string;
  workspace: string;
  version: string;
  table_count: number;
  page_count: number;
  custom_page_count: number;
  record_count_per_table: Record<string, number>;
  updated: string;
}

export interface AdminWorkspaceDetail {
  id: string;
  name: string;
  code: string;
  unit_name: string;
  classification: string;
  visibility: string;
  owners: string[];
  collaborator_count: number;
  app_count: number;
  record_count: number;
  created: string;
  organization_id?: string;
  description?: string;
  collaborators: {
    id: string;
    workspace_id: string;
    eppn: string;
    name: string;
    role: string;
    scoped_affiliation: string;
    department: string;
    added_at: string;
  }[];
  apps: AdminAppRow[];
}

export interface AdminPatchWorkspacePayload {
  organization_id?: string;
  data_classification?: string;
  visibility?: string;
  reason: string;
}

export interface AdminUserRow {
  id: string;
  user_name: string;
  display_name: string;
  email: string;
  affiliation: string;
  units: { id: string; name: string }[];
  active: boolean;
  hold: boolean;
  platform_admin: boolean;
  unit_admin_of: { id: string; name: string }[];
  active_agent_tokens: number;
  latest_token_use: string | null;
}

export interface AdminUserDetail {
  user: AdminUserRow;
  appointments: {
    organization_id: string;
    unit: string;
    scoped_affiliation: string;
    role_title: string;
    source: string;
  }[];
  workspace_memberships: { workspace_id: string; role: string }[];
  tokens: {
    id: string;
    kind: string;
    label: string;
    created_at: string;
    expires_at: string;
    last_used_at: string | null;
    revoked: boolean;
  }[];
  ledger: { timestamp?: string; decision_type: string; rationale: string }[];
}

export interface NewAdminUser {
  userName: string;
  name: string;
  email: string;
  affiliation: string;
  department: string;
  title: string;
  reason: string;
}

export class ApiError extends Error {
  public status: number;
  public details?: any;
  /** One of the server's closed list of error codes. Absent from an older server. */
  public code?: string;
  /** The policy that decided, on a denial. */
  public policy?: PolicyRef;
  /** The bad inputs, on a bad request. */
  public fields?: Record<string, string>;
  /** What to do about it, when the server knows. */
  public remedy?: string;
  public reason?: string;

  constructor(status: number, message: string, details?: any) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.details = details;
    if (details && typeof details === "object") {
      if (typeof details.code === "string") this.code = details.code;
      if (details.policy && typeof details.policy.description === "string") this.policy = details.policy;
      if (details.fields && typeof details.fields === "object") this.fields = details.fields;
      if (typeof details.remedy === "string") this.remedy = details.remedy;
      if (typeof details.reason === "string") this.reason = details.reason;
    }
  }
}

let activeToken: string | null = null;

export const setAuthToken = (token: string | null) => {
  activeToken = token;
  if (typeof window !== "undefined") {
    if (token) {
      localStorage.setItem("scaffoldry_token", token);
    } else {
      localStorage.removeItem("scaffoldry_token");
    }
  }
};

export const getAuthToken = (): string | null => {
  if (activeToken) return activeToken;
  if (typeof window !== "undefined") {
    return localStorage.getItem("scaffoldry_token");
  }
  return null;
};

const getBaseUrl = (): string => {
  const envUrl = (import.meta as any).env?.VITE_API_BASE_URL;
  return envUrl || "/api/v1";
};

const request = async <T>(path: string, options: RequestInit = {}): Promise<T> => {
  const baseUrl = getBaseUrl();
  const token = getAuthToken();

  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    ...(options.headers as Record<string, string>),
  };

  if (token) {
    headers["Authorization"] = `Bearer ${token}`;
  }

  const url = `${baseUrl}${path}`;
  let response: Response;
  try {
    response = await fetch(url, {
      ...options,
      headers,
    });
  } catch (e) {
    // The request never reached a server. Status 0 means "not reachable".
    throw new ApiError(0, e instanceof Error ? e.message : "The server is not reachable.");
  }

  if (!response.ok) {
    let errorMessage = `HTTP ${response.status} ${response.statusText}`;
    let details: any = null;
    try {
      const data = await response.json();
      details = data;
      if (data.error) {
        errorMessage = data.error;
      }
    } catch {
      // Body not JSON
    }
    throw new ApiError(response.status, errorMessage, details);
  }

  if (response.status === 204) {
    return null as T;
  }

  return response.json();
};

export const apiClient = {
  // Authentication & Sessions
  listTokens: async () => {
    return request<{ tokens: any[] }>("/auth/tokens");
  },

  createToken: async (input: { label: string; days?: number; kind?: string }) => {
    return request<{ token: string; id: string; expires_at: string; label: string; kind: string }>("/auth/tokens", {
      method: "POST",
      body: JSON.stringify(input),
    });
  },

  revokeToken: async (id: string) => {
    return request<{ message: string; revoked_id: string }>(`/auth/tokens/${id}`, {
      method: "DELETE",
    });
  },

  getSettings: async () => {
    return request<{ settings: Record<string, any> }>("/settings");
  },

  updateSetting: async (key: string, value: any) => {
    return request<{ key: string; value: any }>(`/settings/${key}`, {
      method: "PUT",
      body: JSON.stringify(value),
    });
  },

  getCurrentUser: async () => {
    return request<{
      user: any;
      is_impersonating: boolean;
      original_admin: any;
      is_platform_admin: boolean;
    }>("/auth/me");
  },

  adminListWorkspaces: async (params: Record<string, string | undefined> = {}) => {
    const q = new URLSearchParams();
    for (const [k, v] of Object.entries(params)) {
      if (v !== undefined && v !== "") q.set(k, v);
    }
    const qs = q.toString();
    return request<{ rows: AdminWorkspaceRow[]; next_cursor: string | null }>(
      `/admin/workspaces${qs ? `?${qs}` : ""}`
    );
  },

  adminGetWorkspace: async (id: string) => {
    return request<AdminWorkspaceDetail>(`/admin/workspaces/${encodeURIComponent(id)}`);
  },

  adminPatchWorkspace: async (id: string, payload: AdminPatchWorkspacePayload) => {
    return request<AdminWorkspaceDetail>(`/admin/workspaces/${encodeURIComponent(id)}`, {
      method: "PATCH",
      body: JSON.stringify(payload),
    });
  },

  adminTransferWorkspaceOwnership: async (id: string, eppn: string, reason: string) => {
    return request<any>(`/admin/workspaces/${encodeURIComponent(id)}/transfer-ownership`, {
      method: "POST",
      body: JSON.stringify({ eppn, reason }),
    });
  },

  adminListApps: async (params: Record<string, string | undefined> = {}) => {
    const q = new URLSearchParams();
    for (const [k, v] of Object.entries(params)) {
      if (v !== undefined && v !== "") q.set(k, v);
    }
    const qs = q.toString();
    return request<{ rows: AdminAppRow[]; next_cursor: string | null }>(
      `/admin/apps${qs ? `?${qs}` : ""}`
    );
  },

  adminListUsers: async (params: Record<string, string | undefined> = {}) => {
    const q = new URLSearchParams();
    for (const [k, v] of Object.entries(params)) {
      if (v !== undefined && v !== "") q.set(k, v);
    }
    const qs = q.toString();
    return request<{ users: AdminUserRow[]; next_cursor: string | null }>(
      `/admin/users${qs ? `?${qs}` : ""}`
    );
  },

  adminGetUser: async (id: string) => {
    return request<AdminUserDetail>(`/admin/users/${encodeURIComponent(id)}`);
  },

  adminCreateUser: async (body: NewAdminUser) => {
    return request<{ user: AdminUserRow }>("/admin/users", {
      method: "POST",
      body: JSON.stringify(body),
    });
  },

  adminHoldUser: async (id: string, hold: boolean, reason: string) => {
    return request<{ user: AdminUserRow }>(`/admin/users/${encodeURIComponent(id)}/hold`, {
      method: "POST",
      body: JSON.stringify({ hold, reason }),
    });
  },

  adminRevokeTokens: async (id: string, reason: string) => {
    return request<{ revoked: number }>(`/admin/users/${encodeURIComponent(id)}/revoke-tokens`, {
      method: "POST",
      body: JSON.stringify({ reason }),
    });
  },

  adminListGroups: async () => {
    return request<{ groups: { id: string; name: string; member_count: number }[] }>("/admin/groups");
  },

  revokeAppointment: async (unit: string, eppn: string, affiliation: string, reason: string) => {
    return request<{ revoked: boolean }>(
      `/orgs/${encodeURIComponent(unit)}/appointments/${encodeURIComponent(eppn)}/${encodeURIComponent(affiliation)}`,
      { method: "DELETE", body: JSON.stringify({ reason }) }
    );
  },

  getAdminOverview: async () => {
    return request<AdminOverviewData>("/admin/overview");
  },

  getAdminJobs: async (params?: { state?: string; kind?: string; owner?: string; cursor?: string; limit?: number }) => {
    const sp = new URLSearchParams();
    if (params?.state) sp.set("state", params.state);
    if (params?.kind) sp.set("kind", params.kind);
    if (params?.owner) sp.set("owner", params.owner);
    if (params?.cursor) sp.set("cursor", params.cursor);
    if (params?.limit) sp.set("limit", String(params.limit));
    const qs = sp.toString() ? `?${sp.toString()}` : "";
    return request<AdminJobsResponse>(`/admin/jobs${qs}`);
  },

  getAdminJob: async (id: string) => {
    return request<AdminJobDetail>(`/admin/jobs/${id}`);
  },

  cancelAdminJob: async (id: string, reason: string) => {
    return request<AdminJobDetail>(`/admin/jobs/${id}/cancel`, {
      method: "POST",
      body: JSON.stringify({ reason }),
    });
  },

  retryAdminJob: async (id: string, reason: string) => {
    return request<AdminJobDetail>(`/admin/jobs/${id}/retry`, {
      method: "POST",
      body: JSON.stringify({ reason }),
    });
  },

  impersonateUser: async (target_eppn: string) => {
    const data = await request<{
      token: string;
      user: any;
      is_impersonating: boolean;
      original_admin: any;
    }>("/auth/impersonate", {
      method: "POST",
      body: JSON.stringify({ target_eppn }),
    });
    setAuthToken(data.token);
    return data;
  },

  stopImpersonation: async () => {
    const data = await request<{
      token: string;
      user: any;
      is_impersonating: boolean;
    }>("/auth/stop-impersonate", {
      method: "POST",
    });
    setAuthToken(data.token);
    return data;
  },

  logout: async () => {
    try {
      await request("/auth/logout", { method: "POST" });
    } finally {
      setAuthToken(null);
    }
  },

  // Organizations
  getOrgs: async (): Promise<OrganizationNode[]> => {
    return request<OrganizationNode[]>("/orgs");
  },

  listOrganizations: async (): Promise<OrganizationNode[]> => {
    return request<OrganizationNode[]>("/orgs");
  },

  createOrganization: async (payload: {
    name: string;
    code: string;
    org_type: string;
    parent_id?: string | null;
  }): Promise<OrganizationNode> => {
    return request<OrganizationNode>("/orgs", {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },

  patchOrganization: async (
    id: string,
    payload: { name?: string; parent_id?: string | null }
  ): Promise<OrganizationNode> => {
    return request<OrganizationNode>(`/orgs/${encodeURIComponent(id)}`, {
      method: "PATCH",
      body: JSON.stringify(payload),
    });
  },

  appointOrgAdmin: async (
    id: string,
    payload: { eppn: string; scoped_affiliation: string; reason?: string }
  ): Promise<OrgRole> => {
    return request<OrgRole>(`/orgs/${encodeURIComponent(id)}/appointments`, {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },

  listOrgMembers: async (id: string): Promise<any[]> => {
    return request<any[]>(`/orgs/${encodeURIComponent(id)}/members`);
  },

  // Workspaces
  listWorkspaces: async (): Promise<Workspace[]> => {
    return request<Workspace[]>("/workspaces");
  },

  getWorkspace: async (id: string): Promise<{ workspace: Workspace; collaborators: Collaborator[] }> => {
    return request<{ workspace: Workspace; collaborators: Collaborator[] }>(`/workspaces/${encodeURIComponent(id)}`);
  },

  createWorkspace: async (payload: Partial<Workspace>): Promise<Workspace> => {
    return request<Workspace>("/workspaces", {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },

  updateWorkspace: async (id: string, payload: Partial<Workspace>): Promise<Workspace> => {
    return request<Workspace>(`/workspaces/${encodeURIComponent(id)}`, {
      method: "PUT",
      body: JSON.stringify(payload),
    });
  },

  // Workspace Collaborators
  listWorkspaceCollaborators: async (workspaceId: string): Promise<Collaborator[]> => {
    return request<Collaborator[]>(`/workspaces/${encodeURIComponent(workspaceId)}/collaborators`);
  },

  addWorkspaceCollaborator: async (
    workspaceId: string,
    payload: {
      eppn: string;
      name: string;
      role: string;
      department?: string;
      scoped_affiliation?: string;
    }
  ): Promise<Collaborator> => {
    return request<Collaborator>(`/workspaces/${encodeURIComponent(workspaceId)}/collaborators`, {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },

  updateWorkspaceCollaboratorRole: async (
    workspaceId: string,
    eppn: string,
    role: string
  ): Promise<Collaborator> => {
    return request<Collaborator>(
      `/workspaces/${encodeURIComponent(workspaceId)}/collaborators/${encodeURIComponent(eppn)}`,
      {
        method: "PUT",
        body: JSON.stringify({ role }),
      }
    );
  },

  removeWorkspaceCollaborator: async (workspaceId: string, eppn: string): Promise<void> => {
    return request<void>(
      `/workspaces/${encodeURIComponent(workspaceId)}/collaborators/${encodeURIComponent(eppn)}`,
      {
        method: "DELETE",
      }
    );
  },

  // Decision Ledger & Governance
  getGovernanceLedger: async (): Promise<{
    chain_valid: boolean;
    total_entries: number;
    head_hash: string;
    entries: LedgerEntryItem[];
  }> => {
    return request<{
      chain_valid: boolean;
      total_entries: number;
      head_hash: string;
      entries: LedgerEntryItem[];
    }>("/governance/ledger");
  },

  verifyGovernanceLedger: async (): Promise<{
    verified: boolean;
    total_entries: number;
    genesis_previous_hash: string;
    head_hash: string;
    verification_status: string;
  }> => {
    return request<{
      verified: boolean;
      total_entries: number;
      genesis_previous_hash: string;
      head_hash: string;
      verification_status: string;
    }>("/governance/ledger/verify", {
      method: "POST",
    });
  },

  recordGovernanceDecision: async (decision: {
    principal: string;
    organization_code: string;
    app_slug?: string;
    decision_type: string;
    oscal_control_id: string;
    rationale: string;
    payload?: any;
  }): Promise<{ entry: LedgerEntryItem; success: boolean }> => {
    return request<{ entry: LedgerEntryItem; success: boolean }>("/governance/ledger", {
      method: "POST",
      body: JSON.stringify(decision),
    });
  },
};

export const computeSha256 = async (message: string): Promise<string> => {
  if (typeof crypto !== "undefined" && crypto.subtle) {
    try {
      const msgBuffer = new TextEncoder().encode(message);
      const hashBuffer = await crypto.subtle.digest("SHA-256", msgBuffer);
      const hashArray = Array.from(new Uint8Array(hashBuffer));
      return hashArray.map((b) => b.toString(16).padStart(2, "0")).join("");
    } catch {
      // Fallback
    }
  }
  return "0000000000000000000000000000000000000000000000000000000000000000";
};
