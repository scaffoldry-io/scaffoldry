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
  proposals?: {
    pending: number;
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
  files?: {
    scanner: { configured: boolean; reachable: boolean; last_result: string | null };
    error: number;
    skipped: number;
    pending: number;
    infected: number;
  };
}

export interface ProposalCheck {
  name: string;
  outcome: "pass" | "warn" | "fail";
  messages: string[];
}

export interface ProposalRow {
  id: string;
  proposal_id: string;
  workspace_id: string;
  app_slug: string;
  base_version: number | null;
  status: "Pending" | "Approved" | "Rejected" | "Superseded";
  summary: string;
  proposed_by: string;
  decided_by: string | null;
  created_at: string;
  decided_at: string | null;
  checks: ProposalCheck[];
  changes: string[];
}

/** What a direct save came to: live at once, or waiting for a person to decide. */
export type SaveOutcome = { status: "live" } | { status: "pending"; proposalId: string };

export interface AdminJobRow {
  id: string;
  kind: string;
  /** The kind's plain name and one sentence. A kind the server does not know has its own name and no sentence. */
  title?: string;
  description?: string;
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

export interface PositionTypeRow {
  key: string;
  name: string;
  description: string;
  org_types: string[];
  max_holders: number;
  retired: boolean;
  holder_count: number;
}

export interface NewPositionType {
  key: string;
  name: string;
  description: string;
  org_types: string[];
  max_holders: number;
  reason: string;
}

export interface VacancyRow {
  unit: { id: string; name: string };
  org_type: string;
  position: { key: string; name: string };
}

export interface UnitPositionRow {
  key: string;
  name: string;
  description: string;
  max_holders: number;
  holders: { eppn: string; display_name: string; source: string }[];
}

/** One comment on a record. A removed comment has no body. */
export interface CommentRow {
  id: string;
  record_id: string;
  parent_id: string | null;
  author: string;
  author_name: string;
  body: string | null;
  mentions: string[];
  deleted: boolean;
  created_at: string;
  edited_at: string | null;
  deleted_at: string | null;
}

export interface Mentionable {
  eppn: string;
  name: string;
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

/**
 * Sets the token that requests carry. With `persist` false the token lives in memory only, so a
 * reload goes back to the stored one. A view-as token is set this way, because a reload would
 * otherwise keep the view and lose its banner.
 */
export const setAuthToken = (token: string | null, persist = true) => {
  activeToken = token;
  if (persist && typeof window !== "undefined") {
    if (token) {
      localStorage.setItem("scaffoldry_token", token);
    } else {
      localStorage.removeItem("scaffoldry_token");
    }
  }
};

/** A stored record. `version` changes by one on every save. */
export interface AppRecord {
  id: string;
  app_slug?: string;
  data: Record<string, any>;
  version: number;
  created_at?: string;
  created_by?: string | null;
}

/** An app the signed-in person can reach. `scope` says how: through a workspace role, on this app only, or as an administrator. */
export interface MyApp {
  slug: string;
  title: string;
  description: string;
  department: string;
  workspace_id: string;
  scope: "admin" | "workspace" | "app";
}

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

const requestFull = async <T>(path: string, options: RequestInit = {}): Promise<{ status: number; data: T }> => {
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
    return { status: 204, data: null as T };
  }

  return { status: response.status, data: await response.json() };
};

const request = async <T>(path: string, options: RequestInit = {}): Promise<T> => (await requestFull<T>(path, options)).data;

/** Undoes a batch of record changes. The answer names the batch the undo wrote, which undoes the undo. */
export const undoBatch = (appSlug: string, batchId: string): Promise<{ batch_id: string; records: number }> =>
  request(`/apps/${appSlug}/history/${batchId}/undo`, { method: "POST", body: JSON.stringify({}) });

/** Writes the old values of some fields of a record back, as one new version. */
export const restoreRecord = (appSlug: string, recordId: string, body: { to_version: number; fields?: string[]; version: number }) =>
  request<Record<string, unknown>>(`/apps/${appSlug}/records/${recordId}/restore`, { method: "POST", body: JSON.stringify(body) });

export interface TrashEntry {
  record_id: string;
  table_id: string;
  title: string;
  deleted_by: string;
  deleted_by_name: string;
  deleted_at: string;
  delete_batch: string;
  days_left: number;
}

/** What is in the trash of an app, newest first. */
export const listTrash = (appSlug: string, cursor?: string): Promise<{ entries: TrashEntry[]; next_cursor: string | null }> =>
  request(`/apps/${appSlug}/trash${cursor ? `?cursor=${encodeURIComponent(cursor)}` : ""}`);

/** Brings back one record, or the whole batch when given a delete batch id. */
export const restoreTrash = (appSlug: string, what: string): Promise<{ restored: number; batch_id: string }> =>
  request(`/apps/${appSlug}/trash/${encodeURIComponent(what)}/restore`, { method: "POST", body: JSON.stringify({}) });

/** A file a record holds in an attachment field, as the record stores it. */
export interface StoredFile {
  ref_id: string;
  name: string;
  size: number;
  media_type: string;
}

/** A file as the record's file list reports it, with its scan state. */
export interface RecordFile extends StoredFile {
  field: string;
  scan_state: "pending" | "clean" | "infected" | "error" | "skipped" | string;
}

/** The files of a record. `fields` names the attachment fields the person may read, and no others. */
export interface RecordFiles {
  fields: string[];
  files: RecordFile[];
}

export interface UploadedFile extends StoredFile {
  scan_state: string;
}

export const listRecordFiles = (appSlug: string, recordId: string): Promise<RecordFiles> =>
  request(`/apps/${appSlug}/records/${encodeURIComponent(recordId)}/files`);

/** Downloads a file through the record and field checks. The bytes come back as a Blob. */
export const fetchRecordFile = async (appSlug: string, recordId: string, refId: string): Promise<Blob> => {
  const token = getAuthToken();
  let response: Response;
  try {
    response = await fetch(`${getBaseUrl()}/apps/${appSlug}/records/${encodeURIComponent(recordId)}/files/${encodeURIComponent(refId)}`, {
      headers: token ? { Authorization: `Bearer ${token}` } : {},
    });
  } catch (e) {
    throw new ApiError(0, e instanceof Error ? e.message : "The server is not reachable.");
  }
  if (!response.ok) {
    let details: any = null;
    try {
      details = await response.json();
    } catch {
      // Not JSON.
    }
    throw new ApiError(response.status, details?.error ?? `HTTP ${response.status}`, details);
  }
  return response.blob();
};

export interface UploadOptions {
  onProgress?: (sent: number, total: number) => void;
  signal?: AbortSignal;
}

/** Uploads one file. It uses XMLHttpRequest because `fetch` reports no upload progress. */
export const uploadRecordFile = (appSlug: string, file: File, opts: UploadOptions = {}): Promise<UploadedFile> =>
  new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("POST", `${getBaseUrl()}/apps/${appSlug}/files`);
    const token = getAuthToken();
    if (token) xhr.setRequestHeader("Authorization", `Bearer ${token}`);
    // A header holds Latin-1 only, so the name travels percent-encoded.
    xhr.setRequestHeader("X-File-Name", encodeURIComponent(file.name));
    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable) opts.onProgress?.(e.loaded, e.total);
    };
    xhr.onload = () => {
      let body: any = null;
      try {
        body = JSON.parse(xhr.responseText);
      } catch {
        // Not JSON.
      }
      if (xhr.status >= 200 && xhr.status < 300 && body) resolve(body as UploadedFile);
      else reject(new ApiError(xhr.status, body?.error ?? `HTTP ${xhr.status}`, body));
    };
    xhr.onerror = () => reject(new ApiError(0, "The server is not reachable."));
    xhr.onabort = () => reject(new DOMException("The upload was cancelled.", "AbortError"));
    opts.signal?.addEventListener("abort", () => xhr.abort());
    xhr.send(file);
  });

export interface LinkOption {
  id: string;
  label: string;
  secondary: string | null;
}

/** The records a link may point at: ids and labels only, matched on the query. */
export const listLinkOptions = (
  appSlug: string,
  tableId: string,
  q: string,
  cursor?: string,
): Promise<{ options: LinkOption[]; next_cursor: string | null }> => {
  const params = new URLSearchParams();
  if (q) params.set("q", q);
  if (cursor) params.set("cursor", cursor);
  const query = params.toString();
  return request(`/apps/${appSlug}/tables/${encodeURIComponent(tableId)}/link-options${query ? `?${query}` : ""}`);
};

export interface AggregateQueryOptions {
  function: "count" | "sum" | "avg" | "min" | "max";
  field?: string;
  filter?: Record<string, unknown>;
  groupBy?: string;
}

export interface AggregateGroup {
  key: string;
  group: string;
  value: number;
  count?: number;
}

export interface AggregateResult {
  function: "count" | "sum" | "avg" | "min" | "max";
  field?: string;
  group_by?: string;
  value?: number;
  count?: number;
  groups?: AggregateGroup[];
  has_more?: boolean;
}

/** Totals and group counts computed in one statement over the record index. */
export const aggregateRecords = (
  appSlug: string,
  tableId: string,
  options: AggregateQueryOptions,
): Promise<AggregateResult> => {
  const params = new URLSearchParams();
  params.set("function", options.function);
  if (options.field) params.set("field", options.field);
  if (options.groupBy) params.set("group_by", options.groupBy);
  if (options.filter) params.set("filter", JSON.stringify(options.filter));
  const query = params.toString();
  return request(`/apps/${appSlug}/tables/${encodeURIComponent(tableId)}/aggregate${query ? `?${query}` : ""}`);
};

export interface CollaboratorOption {
  id: string;
  label: string;
  email?: string;
  active?: boolean;
}


export interface AppView {
  id: string;
  app_slug: string;
  table_id: string | null;
  name: string;
  kind: string;
  ownership: "personal" | "collaborative" | "locked";
  owner: string | null;
  config: Record<string, any>;
  version: number;
  created_by: string;
  created_at: string;
  updated_by: string;
  updated_at: string;
}

export interface ViewResponse {
  view: AppView;
  index_state: string;
}

export const listViews = (appSlug: string): Promise<{ views: AppView[] }> => {
  return request(`/apps/${appSlug}/views`);
};

export const createView = (
  appSlug: string,
  payload: {
    table_id?: string;
    name: string;
    kind?: string;
    ownership?: "personal" | "collaborative" | "locked";
    config?: Record<string, any>;
  }
): Promise<ViewResponse> => {
  return request(`/apps/${appSlug}/views`, {
    method: "POST",
    body: JSON.stringify(payload),
  });
};

export const updateView = (
  appSlug: string,
  viewId: string,
  payload: {
    name: string;
    config: Record<string, any>;
    version: number;
  }
): Promise<ViewResponse> => {
  return request(`/apps/${appSlug}/views/${viewId}`, {
    method: "PUT",
    body: JSON.stringify(payload),
  });
};

export const deleteView = (
  appSlug: string,
  viewId: string
): Promise<{ deleted: boolean; id: string }> => {
  return request(`/apps/${appSlug}/views/${viewId}`, {
    method: "DELETE",
  });
};

/** One level of a grouped view: the groups, with counts. */
export interface ViewGroups {
  level: number;
  field: string;
  items: { key: string; count: number }[];
  has_more: boolean;
}

export interface ViewFooter {
  function: string;
  value: number | null;
  unsupported?: boolean;
}

/** What running a view returns: the groups of the next level, or the rows of a group (or of the whole view). */
export interface ViewData {
  view_id: string;
  groups?: ViewGroups;
  records?: { id: string; data: Record<string, any> }[];
  next_cursor?: string | null;
  /** On the first page only. A hidden field has none. */
  footers?: Record<string, ViewFooter>;
  /** Present when a window was asked for: where it began (null for the last page) and how it was found. */
  window?: { offset: number | null; strategy: "offset" | "anchor" | "reverse" };
  /** Present when `count: "exact"` was asked for. `at_least` means the count ran out of time. */
  count?: { value: number; exact: true } | { at_least: number };
  /** Present for a summary view: its groups and one value for each measure, 200 groups at most. */
  summary?: { columns: string[]; groups: { key: string[]; values: (number | null)[] }[]; has_more: boolean };
}

/**
 * Runs a view on the server. `groupPath` holds the key chosen at each level so far. The server filters, sorts,
 * groups and totals. The browser only draws.
 */
export const runView = (
  appSlug: string,
  viewId: string,
  opts: { groupPath?: string[]; cursor?: string; limit?: number; offset?: number; last?: boolean; count?: "exact" } = {}
): Promise<ViewData> => {
  const q = new URLSearchParams();
  if (opts.offset !== undefined) q.set("offset", String(opts.offset));
  if (opts.last) q.set("last", "true");
  if (opts.count) q.set("count", opts.count);
  if (opts.groupPath?.length) q.set("group_path", JSON.stringify(opts.groupPath));
  if (opts.cursor) q.set("cursor", opts.cursor);
  if (opts.limit) q.set("limit", String(opts.limit));
  const qs = q.toString();
  return request(`/apps/${appSlug}/views/${viewId}/data${qs ? `?${qs}` : ""}`);
};

export const duplicateView = (
  appSlug: string,
  viewId: string
): Promise<ViewResponse> => {
  return request(`/apps/${appSlug}/views/${viewId}/duplicate`, {
    method: "POST",
    body: JSON.stringify({}),
  });
};

/** Search the directory for collaborators who can read the app. */
export const listCollaborators = (
  appSlug: string,
  q?: string,
): Promise<{ collaborators: CollaboratorOption[] }> => {
  const params = new URLSearchParams();
  if (q) params.set("q", q);
  const query = params.toString();
  return request(`/apps/${appSlug}/collaborators${query ? `?${query}` : ""}`);
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

  adminListPositions: async () => {
    return request<{ positions: PositionTypeRow[] }>("/admin/positions");
  },

  adminCreatePosition: async (body: NewPositionType) => {
    return request<{ position: PositionTypeRow }>("/admin/positions", {
      method: "POST",
      body: JSON.stringify(body),
    });
  },

  adminPatchPosition: async (key: string, body: Record<string, unknown>) => {
    return request<{ position: PositionTypeRow }>(`/admin/positions/${encodeURIComponent(key)}`, {
      method: "PATCH",
      body: JSON.stringify(body),
    });
  },

  adminPositionVacancies: async () => {
    return request<{ vacancies: VacancyRow[] }>("/admin/positions/vacancies");
  },

  listUnitPositions: async (unit: string) => {
    return request<{ positions: UnitPositionRow[] }>(`/orgs/${encodeURIComponent(unit)}/positions`);
  },

  assignPositionHolder: async (unit: string, key: string, body: { eppn: string; replace: boolean; reason: string }) => {
    return request<unknown>(`/orgs/${encodeURIComponent(unit)}/positions/${encodeURIComponent(key)}/holders`, {
      method: "POST",
      body: JSON.stringify(body),
    });
  },

  vacatePositionHolder: async (unit: string, key: string, eppn: string, reason: string) => {
    return request<unknown>(
      `/orgs/${encodeURIComponent(unit)}/positions/${encodeURIComponent(key)}/holders/${encodeURIComponent(eppn)}`,
      { method: "DELETE", body: JSON.stringify({ reason }) }
    );
  },

  listComments: async (appSlug: string, recordId: string) => {
    return request<{ comments: CommentRow[] }>(`/apps/${appSlug}/records/${recordId}/comments`);
  },

  postComment: async (appSlug: string, recordId: string, body: string, parentId?: string) => {
    return request<CommentRow>(`/apps/${appSlug}/records/${recordId}/comments`, {
      method: "POST",
      body: JSON.stringify({ body, ...(parentId ? { parent_id: parentId } : {}) }),
    });
  },

  editComment: async (appSlug: string, commentId: string, body: string) => {
    return request<CommentRow>(`/apps/${appSlug}/comments/${commentId}`, { method: "PATCH", body: JSON.stringify({ body }) });
  },

  removeComment: async (appSlug: string, commentId: string) => {
    return request<CommentRow>(`/apps/${appSlug}/comments/${commentId}`, { method: "DELETE" });
  },

  /** The people a mention can name: those who can read the app. */
  mentionable: async (appSlug: string) => {
    return request<{ people: Mentionable[] }>(`/apps/${appSlug}/mentionable`);
  },

  listProposals: async (workspaceId: string) => {
    return request<ProposalRow[]>(`/workspaces/${workspaceId}/proposals`);
  },

  getProposal: async (id: string) => {
    return request<ProposalRow>(`/proposals/${id}`);
  },

  decideProposal: async (id: string, decision: "approve" | "reject", rationale: string) => {
    return request<ProposalRow>(`/proposals/${id}/decision`, {
      method: "POST",
      body: JSON.stringify({ decision, rationale }),
    });
  },

  /** Saves a rule. A 202 means a person must approve it first, and nothing is live. */
  submitRule: async (appSlug: string, rule: unknown): Promise<SaveOutcome> => {
    const res = await requestFull<{ proposal_id?: string }>(`/apps/${appSlug}/automations`, {
      method: "POST",
      body: JSON.stringify(rule),
    });
    return res.status === 202 ? { status: "pending", proposalId: res.data.proposal_id ?? "" } : { status: "live" };
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

  /** The signed-in person's own jobs, newest first. */
  listMyJobs: async () => (await request<{ jobs?: import("./ui").BackgroundJob[] }>("/jobs?limit=50"))?.jobs ?? [],

  /** Opens a read-only view of an app as another person. The caller swaps tokens and shows the banner. */
  startViewAs: (appSlug: string, eppn: string) =>
    request<{
      token: string;
      expires_at: string;
      app_slug: string;
      target: { eppn: string; name: string };
      read_only: boolean;
    }>(`/apps/${appSlug}/view-as`, { method: "POST", body: JSON.stringify({ eppn }) }),

  /** Ends the view-as session that the current token belongs to. */
  stopViewAs: () => request<{ status: string }>("/auth/stop-view-as", { method: "POST" }),

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

  // Records of an app. A save names the version it read, and the server answers 409 when it is stale.
  listRecords: async (slug: string): Promise<AppRecord[]> => {
    const res = await request<{ records: AppRecord[] }>(`/apps/${encodeURIComponent(slug)}/records`);
    return Array.isArray(res?.records) ? res.records : [];
  },

  getRecord: async (slug: string, id: string): Promise<AppRecord> => {
    return request<AppRecord>(`/apps/${encodeURIComponent(slug)}/records/${encodeURIComponent(id)}`);
  },

  createRecord: async (slug: string, data: Record<string, unknown>): Promise<AppRecord> => {
    return request<AppRecord>(`/apps/${encodeURIComponent(slug)}/records`, { method: "POST", body: JSON.stringify({ data }) });
  },

  updateRecord: async (slug: string, id: string, version: number, data: Record<string, unknown>): Promise<AppRecord> => {
    return request<AppRecord>(`/apps/${encodeURIComponent(slug)}/records/${encodeURIComponent(id)}`, {
      method: "PATCH",
      body: JSON.stringify({ version, data }),
    });
  },

  deleteRecord: async (slug: string, id: string): Promise<void> => {
    await request<null>(`/apps/${encodeURIComponent(slug)}/records/${encodeURIComponent(id)}`, { method: "DELETE" });
  },

  /** The apps the signed-in person can reach, and how. */
  listMyApps: async (): Promise<MyApp[]> => {
    const res = await request<{ apps: MyApp[] }>("/me/apps");
    // An answer that is not a list of apps is not an answer. The desk must not read it as "no apps".
    if (!res || !Array.isArray(res.apps)) throw new ApiError(0, "The server's app list was not in the expected form.");
    return res.apps;
  },

  /** Creates an app in a workspace. 201 means it is live. 202 means it is a proposal waiting for approval. */
  createApp: async (workspaceId: string, manifest: Record<string, unknown>): Promise<{ status: number; data: any }> => {
    return requestFull<any>(`/workspaces/${encodeURIComponent(workspaceId)}/apps`, { method: "POST", body: JSON.stringify(manifest) });
  },

  /** One app's definition: its tables, fields, and views. */
  getApp: async (slug: string): Promise<any> => {
    return request<any>(`/apps/${encodeURIComponent(slug)}`);
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

  getWorkspaceGuards: async (id: string) => {
    return request<{ current: any; history: any[] }>(`/workspaces/${id}/guards`);
  },

  putWorkspaceGuards: async (id: string, payload: { rules?: any[]; source?: string; reason: string }) => {
    return request<any>(`/workspaces/${id}/guards`, {
      method: "PUT",
      body: JSON.stringify(payload),
    });
  },

  impactWorkspaceGuards: async (id: string, payload: { rules?: any[]; source?: string }) => {
    return request<{ changes: any[] }>(`/workspaces/${id}/guards/impact`, {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },

  testWorkspaceGuards: async (id: string, payload: { user: string; action: string; is_ferpa_sensitive?: boolean; rules?: any[]; source?: string }) => {
    return request<{ current: { allowed: boolean; policy?: any }; proposed: { allowed: boolean; policy?: any } }>(`/workspaces/${id}/guards/test`, {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },

  previewFormula: async (payload: { expression: string; table_id: string; app_slug: string; record?: any }) => {
    return request<{
      valid: boolean;
      result: any;
      problems: Array<{ message: string; position: number }>;
      error?: string | null;
      error_pos?: number | null;
    }>("/formula/preview", {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },

  previewAccess: async (
    slug: string,
    payload: {
      table_id: string;
      eppn?: string;
      person?: any;
      rules?: any[];
      column_rules?: any[];
    }
  ) => {
    return request<{
      can_see: number;
      can_add: number;
      can_change: number;
      can_remove: number;
      total_rows: number;
      hidden_columns: string[];
    }>(`/apps/${slug}/access/preview`, {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },

  listViews: async (appSlug: string) => listViews(appSlug),
  createView: async (appSlug: string, payload: Parameters<typeof createView>[1]) => createView(appSlug, payload),
  updateView: async (appSlug: string, viewId: string, payload: Parameters<typeof updateView>[2]) => updateView(appSlug, viewId, payload),
  deleteView: async (appSlug: string, viewId: string) => deleteView(appSlug, viewId),
  duplicateView: async (appSlug: string, viewId: string) => duplicateView(appSlug, viewId),
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
