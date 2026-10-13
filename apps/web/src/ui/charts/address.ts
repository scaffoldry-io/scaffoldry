/** A group of a summary, as a field and the value its rows share. An empty value is the group with none. */
export interface GroupKey {
  field: string;
  value: string;
}

export interface UnsavedFilter {
  conjunction: "And";
  clauses: { id: string; field_name: string; operator: "Equals" | "IsEmpty"; value: string }[];
}

export interface UnsavedView {
  app: string;
  table: string;
  filter: UnsavedFilter;
}

const bytes = (s: string) => new TextEncoder().encode(s);
const text = (b: Uint8Array) => new TextDecoder().decode(b);

function encode(s: string): string {
  let bin = "";
  for (const byte of bytes(s)) bin += String.fromCharCode(byte);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function decode(s: string): string {
  const bin = atob(s.replace(/-/g, "+").replace(/_/g, "/"));
  return text(Uint8Array.from(bin, (c) => c.charCodeAt(0)));
}

/**
 * The address that opens the grid on one group of a summary, in a view that is not saved. The view is in the address
 * itself: a table and a filter that names the group, with no write to the server.
 */
export function unsavedViewAddress(app: string, table: string, groups: GroupKey[]): string {
  const view: UnsavedView = {
    app,
    table,
    filter: {
      conjunction: "And",
      clauses: groups.map((g, i) => ({ id: `group-${i}`, field_name: g.field, operator: g.value === "" ? "IsEmpty" : "Equals", value: g.value })),
    },
  };
  return `?unsaved=${encode(JSON.stringify(view))}`;
}

/** The unsaved view an address carries, or `null` when it carries none or carries something that is not one. */
export function readUnsavedView(search: string): UnsavedView | null {
  const raw = new URLSearchParams(search).get("unsaved");
  if (!raw) return null;
  try {
    const v = JSON.parse(decode(raw)) as UnsavedView;
    const ok =
      typeof v?.app === "string" &&
      typeof v.table === "string" &&
      v.filter?.conjunction === "And" &&
      Array.isArray(v.filter.clauses) &&
      v.filter.clauses.every((c) => typeof c.field_name === "string" && (c.operator === "Equals" || c.operator === "IsEmpty") && typeof c.value === "string");
    return ok ? v : null;
  } catch {
    return null;
  }
}
