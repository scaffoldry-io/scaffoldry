// The second evaluator of `show_when`, for immediate feedback in a form. The server's (`scaffoldry-engine/src/show_when.rs`)
// is authoritative, and `test/show-when.test.ts` runs the vectors it generates through this one. Keep it as small as that.

export type ShowOp = "eq" | "ne" | "in" | "is_empty" | "is_not_empty";

export interface Condition {
  field: string;
  op: ShowOp;
  value?: unknown;
}

export interface FormFieldConditions {
  name: string;
  show_when?: Condition[];
}

/** A scalar as the text it compares as. `null` is empty. */
function text(v: unknown): string | null {
  if (v === null || v === undefined) return null;
  if (typeof v === "string") {
    const t = v.trim();
    return t === "" ? null : t;
  }
  if (typeof v === "boolean" || typeof v === "number") return String(v);
  return JSON.stringify(v);
}

/** The texts of an answer: one for a scalar, one for each item of a list, none when empty. */
function texts(v: unknown): string[] {
  const items = Array.isArray(v) ? v : [v];
  return items.map(text).filter((t): t is string => t !== null);
}

/** The values `in` compares with: a list, or a comma list in a string. */
function wanted(v: unknown): string[] {
  if (typeof v === "string") return v.split(",").map((p) => p.trim()).filter((p) => p !== "");
  return texts(v);
}

function holds(c: Condition, answer: unknown): boolean {
  const have = texts(answer);
  switch (c.op) {
    case "is_empty":
      return have.length === 0;
    case "is_not_empty":
      return have.length > 0;
    case "eq": {
      const want = text(c.value);
      return want !== null && have.includes(want);
    }
    case "ne": {
      const want = text(c.value);
      return !(want !== null && have.includes(want));
    }
    case "in": {
      const list = wanted(c.value);
      return have.some((h) => list.includes(h));
    }
  }
}

/** Whether each field of the form shows, in the form's order. A hidden field answers as empty to the fields after it. */
export function visibleFields(fields: FormFieldConditions[], answers: Record<string, unknown>): boolean[] {
  const seen: Record<string, unknown> = {};
  return fields.map((f) => {
    const shown = (f.show_when ?? []).every((c) => holds(c, c.field in seen ? seen[c.field] : null));
    seen[f.name] = shown ? (answers[f.name] ?? null) : null;
    return shown;
  });
}
