/** What a limited token may do. The server checks it; this file only describes it. */
export type TokenOp = "read" | "create" | "update" | "delete";

export interface TokenScope {
  apps: string[];
  /** A table id and the operations allowed on it. */
  tables: Record<string, TokenOp[]>;
  tools?: string[];
  max_rows_per_call?: number;
}

export const TOKEN_OPS: { op: TokenOp; label: string; word: string }[] = [
  { op: "read", label: "Read", word: "read" },
  { op: "create", label: "Add", word: "add" },
  { op: "update", label: "Change", word: "change" },
  { op: "delete", label: "Delete", word: "delete" },
];

function joinAnd(items: string[]): string {
  if (items.length <= 1) return items.join("");
  if (items.length === 2) return `${items[0]} and ${items[1]}`;
  return `${items.slice(0, -1).join(", ")}, and ${items[items.length - 1]}`;
}

/** Titles to show instead of ids. An id with no title is shown as it is. */
export interface ScopeNames {
  apps?: Record<string, string>;
  tables?: Record<string, string>;
}

/** `This token can read and add Applicants in Admissions. It cannot do anything else.` */
export function scopeSentence(scope: TokenScope | null | undefined, names: ScopeNames = {}): string {
  if (!scope) return "This token has all the rights of its owner.";
  const ops = TOKEN_OPS.filter(({ op }) => Object.values(scope.tables).some((list) => list.includes(op))).map((o) => o.word);
  const tables = Object.keys(scope.tables).map((id) => names.tables?.[id] ?? id);
  const apps = scope.apps.map((slug) => names.apps?.[slug] ?? slug);
  if (ops.length === 0 || tables.length === 0 || apps.length === 0) return "This token cannot do anything.";
  return `This token can ${joinAnd(ops)} ${joinAnd(tables)} in ${joinAnd(apps)}. It cannot do anything else.`;
}
