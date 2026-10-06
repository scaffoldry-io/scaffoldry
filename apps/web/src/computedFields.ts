import { AppTable, FieldSpec } from "./types";

/**
 * Evaluates in-memory formulas without external dependencies.
 * Supports token substitution ({field_name}), basic arithmetic (*, /, +, -),
 * and string concatenation.
 */
export function evaluateClientFormula(
  expression: string,
  record: Record<string, any>
): any {
  const expr = expression.trim();
  if (!expr) return "";

  // Direct reference e.g. "{budget}"
  if (expr.startsWith("{") && expr.endsWith("}") && !expr.slice(1, -1).includes("}")) {
    const field = expr.slice(1, -1);
    return record[field] ?? "";
  }

  // Multiplication: {budget} * 0.20
  if (expr.includes("*")) {
    const [left, right] = expr.split("*");
    const v1 = parseOperand(left, record);
    const v2 = parseOperand(right, record);
    if (typeof v1 === "number" && typeof v2 === "number") {
      return v1 * v2;
    }
  }

  // Division: {budget} / 12
  if (expr.includes("/")) {
    const [left, right] = expr.split("/");
    const v1 = parseOperand(left, record);
    const v2 = parseOperand(right, record);
    if (typeof v1 === "number" && typeof v2 === "number" && v2 !== 0) {
      return v1 / v2;
    }
  }

  // Addition or string concatenation
  if (expr.includes("+")) {
    const parts = expr.split("+").map((s) => s.trim());
    let allNumbers = true;
    let sum = 0;
    let concatStr = "";

    for (const part of parts) {
      const v = parseOperand(part, record);
      if (typeof v === "number" && !isNaN(v)) {
        sum += v;
      } else {
        allNumbers = false;
      }
      concatStr += String(v ?? "");
    }

    return allNumbers && parts.length > 1 ? sum : concatStr;
  }

  // Subtraction: {budget} - {spent}
  if (expr.includes("-")) {
    const [left, right] = expr.split("-");
    const v1 = parseOperand(left, record);
    const v2 = parseOperand(right, record);
    if (typeof v1 === "number" && typeof v2 === "number") {
      return v1 - v2;
    }
  }

  return parseOperand(expr, record);
}

function parseOperand(op: string, record: Record<string, any>): any {
  const trimmed = op.trim();
  if (trimmed.startsWith("{") && trimmed.endsWith("}")) {
    const key = trimmed.slice(1, -1);
    const val = record[key];
    const num = Number(val);
    return !isNaN(num) && val !== "" && val !== null && typeof val !== "boolean" ? num : val ?? "";
  }
  if (
    (trimmed.startsWith('"') && trimmed.endsWith('"')) ||
    (trimmed.startsWith("'") && trimmed.endsWith("'"))
  ) {
    return trimmed.slice(1, -1);
  }
  const n = Number(trimmed);
  if (!isNaN(n)) return n;
  return record[trimmed] ?? "";
}

/**
 * Resolves computed field values across tables (Lookup, Count, Rollup, Formula).
 */
export function computeFieldValue(
  field: FieldSpec,
  record: Record<string, any>,
  allTables: AppTable[]
): any {
  switch (field.field_type) {
    case "Formula": {
      if (field.formula_expression) {
        return evaluateClientFormula(field.formula_expression, record);
      }
      return null;
    }

    case "Lookup": {
      if (!field.target_table_id) return null;
      const targetTable = allTables.find((t) => t.id === field.target_table_id);
      if (!targetTable || !targetTable.records) return null;

      // 1. Direct foreign key pointer (e.g. record has lead_investigator_id)
      const directKey = Object.keys(record).find(
        (k) => k.endsWith("_id") && record[k] && targetTable.records.some((r) => r.id === record[k])
      );
      if (directKey) {
        const linkedId = record[directKey];
        const match = targetTable.records.find((r) => r.id === linkedId);
        if (match && field.target_display_field) {
          return match[field.target_display_field] ?? null;
        }
      }

      // 2. Reverse relationship (target table records reference record.id)
      const reverseMatches = targetTable.records.filter((r) =>
        Object.values(r).some((v) => v === record.id)
      );
      if (reverseMatches.length > 0 && field.target_display_field) {
        if (reverseMatches.length === 1) {
          return reverseMatches[0][field.target_display_field] ?? null;
        }
        return reverseMatches.map((r) => r[field.target_display_field]);
      }

      return null;
    }

    case "Count": {
      if (!field.target_table_id) return 0;
      const targetTable = allTables.find((t) => t.id === field.target_table_id);
      if (!targetTable || !targetTable.records) return 0;

      // Count records in target table pointing to this record's id
      const matches = targetTable.records.filter((r) =>
        Object.values(r).some((v) => v === record.id)
      );
      return matches.length;
    }

    case "Rollup": {
      if (!field.target_table_id) return null;
      const targetTable = allTables.find((t) => t.id === field.target_table_id);
      if (!targetTable || !targetTable.records) return null;

      // Find linked records
      const matches = targetTable.records.filter((r) =>
        Object.values(r).some((v) => v === record.id)
      );

      const targetCol = field.target_display_field || "amount";
      const values: number[] = matches
        .map((r) => Number(r[targetCol]))
        .filter((n) => !isNaN(n));

      const fn = (field.rollup_function || "sum").toLowerCase();
      switch (fn) {
        case "sum":
          return values.reduce((acc, curr) => acc + curr, 0);
        case "avg":
          return values.length > 0
            ? values.reduce((acc, curr) => acc + curr, 0) / values.length
            : 0;
        case "min":
          return values.length > 0 ? Math.min(...values) : null;
        case "max":
          return values.length > 0 ? Math.max(...values) : null;
        case "count":
          return values.length;
        default:
          return null;
      }
    }

    default:
      return record[field.name] ?? null;
  }
}

/**
 * Returns field type icon symbol.
 */
export function getFieldTypeIcon(type: string): string {
  switch (type) {
    case "Text":
      return "🔤";
    case "Number":
      return "🔢";
    case "Currency":
      return "💰";
    case "Percent":
      return "%";
    case "Checkbox":
      return "☑";
    case "Select":
      return "▼";
    case "MultiSelect":
      return "🏷";
    case "Date":
      return "📅";
    case "Relation":
      return "🔗";
    case "Lookup":
      return "🔍";
    case "Count":
      return "#";
    case "Rollup":
      return "Σ";
    case "Formula":
      return "ƒx";
    case "Rating":
      return "★";
    case "Email":
      return "✉";
    case "Phone":
      return "📞";
    case "Url":
      return "🌐";
    case "Autonumber":
      return "123";
    case "CreatedTime":
    case "LastModifiedTime":
      return "🕒";
    default:
      return "📄";
  }
}
