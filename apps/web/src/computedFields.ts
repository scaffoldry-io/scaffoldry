import { AppTable, CompoundFilter, FieldSpec, FilterClause, SortRule } from "./types";

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
      const records = targetTable.records;

      // 1. Direct foreign key pointer (e.g. record has lead_investigator_id)
      const directKey = Object.keys(record).find(
        (k) => k.endsWith("_id") && record[k] && records.some((r) => r.id === record[k])
      );
      if (directKey) {
        const linkedId = record[directKey];
        const match = records.find((r) => r.id === linkedId);
        if (match && field.target_display_field) {
          return match[field.target_display_field] ?? null;
        }
      }

      // 2. Reverse relationship (target table records reference record.id)
      const reverseMatches = records.filter((r) =>
        Object.values(r).some((v) => v === record.id)
      );
      const displayField = field.target_display_field;
      if (reverseMatches.length > 0 && displayField) {
        if (reverseMatches.length === 1) {
          return reverseMatches[0][displayField] ?? null;
        }
        return reverseMatches.map((r) => r[displayField]);
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

/**
 * Evaluates whether a record satisfies a single filter clause.
 */
export function matchesFilterClause(
  record: Record<string, any>,
  clause: FilterClause
): boolean {
  const cellVal = record[clause.field_name];

  if (clause.operator === "is_empty") {
    return cellVal === undefined || cellVal === null || cellVal === "";
  }
  if (clause.operator === "is_not_empty") {
    return cellVal !== undefined && cellVal !== null && cellVal !== "";
  }

  const query = clause.value.trim().toLowerCase();
  if (!query) {
    return true;
  }

  switch (clause.operator) {
    case "equals":
      return String(cellVal ?? "").trim().toLowerCase() === query;
    case "not_equals":
      return String(cellVal ?? "").trim().toLowerCase() !== query;
    case "contains":
      return String(cellVal ?? "").toLowerCase().includes(query);
    case "not_contains":
      return !String(cellVal ?? "").toLowerCase().includes(query);
    case "greater_than": {
      const n1 = Number(cellVal);
      const n2 = Number(query);
      return !isNaN(n1) && !isNaN(n2) && n1 > n2;
    }
    case "less_than": {
      const n1 = Number(cellVal);
      const n2 = Number(query);
      return !isNaN(n1) && !isNaN(n2) && n1 < n2;
    }
    default:
      return true;
  }
}

/**
 * Filters records by compound AND/OR filter criteria.
 */
export function applyCompoundFilter(
  records: Record<string, any>[],
  filter?: CompoundFilter
): Record<string, any>[] {
  if (!filter || !filter.clauses || filter.clauses.length === 0) {
    return records;
  }

  return records.filter((rec) => {
    if (filter.conjunction === "AND") {
      return filter.clauses.every((c) => matchesFilterClause(rec, c));
    } else {
      return filter.clauses.some((c) => matchesFilterClause(rec, c));
    }
  });
}

/**
 * Sorts records by multiple sort rules.
 */
export function applyMultiSort(
  records: Record<string, any>[],
  sortRules?: SortRule[]
): Record<string, any>[] {
  if (!sortRules || sortRules.length === 0) {
    return [...records];
  }

  const copy = [...records];
  copy.sort((a, b) => {
    for (const rule of sortRules) {
      const valA = a[rule.field_name];
      const valB = b[rule.field_name];

      let ord = 0;
      const numA = Number(valA);
      const numB = Number(valB);

      if (!isNaN(numA) && !isNaN(numB) && typeof valA !== "boolean") {
        ord = numA - numB;
      } else {
        const strA = String(valA ?? "").toLowerCase();
        const strB = String(valB ?? "").toLowerCase();
        ord = strA.localeCompare(strB);
      }

      if (ord !== 0) {
        return rule.direction === "asc" ? ord : -ord;
      }
    }
    return 0;
  });

  return copy;
}

export interface RecordGroup {
  groupValue: string;
  records: Record<string, any>[];
  totalBudget: number;
}

/**
 * Groups records by a specified field with aggregated sub-totals.
 */
export function groupRecordsByField(
  records: Record<string, any>[],
  groupByField?: string
): RecordGroup[] {
  if (!groupByField) {
    const total = records.reduce((acc, r) => acc + (Number(r.budget || r.allocated_amount) || 0), 0);
    return [{ groupValue: "All Records", records, totalBudget: total }];
  }

  const groupsMap = new Map<string, Record<string, any>[]>();

  for (const rec of records) {
    const key = String(rec[groupByField] || "Unassigned");
    if (!groupsMap.has(key)) {
      groupsMap.set(key, []);
    }
    groupsMap.get(key)!.push(rec);
  }

  const result: RecordGroup[] = [];
  for (const [groupValue, groupRecords] of groupsMap.entries()) {
    const total = groupRecords.reduce(
      (acc, r) => acc + (Number(r.budget || r.allocated_amount) || 0),
      0
    );
    result.push({ groupValue, records: groupRecords, totalBudget: total });
  }

  return result;
}

/**
 * Escapes a cell value for RFC 4180 CSV export.
 */
export function escapeCsvCell(cell: string): string {
  if (cell.includes(",") || cell.includes('"') || cell.includes("\n") || cell.includes("\r")) {
    return `"${cell.replace(/"/g, '""')}"`;
  }
  return cell;
}

/**
 * Exports fields and records to an RFC 4180 CSV string.
 */
export function exportToCsv(
  fields: { name: string; label?: string }[],
  records: Record<string, any>[]
): string {
  const headers = fields.map((f) => escapeCsvCell(f.name));
  const lines = [headers.join(",")];

  for (const record of records) {
    const row = fields.map((f) => {
      const val = record[f.name];
      if (val === undefined || val === null) return "";
      if (Array.isArray(val)) return escapeCsvCell(val.join(", "));
      return escapeCsvCell(String(val));
    });
    lines.push(row.join(","));
  }

  return lines.join("\n");
}

/**
 * Parses an RFC 4180 CSV string into headers and row objects.
 */
export function parseCsv(csvText: string): { headers: string[]; rows: Record<string, any>[] } {
  const trimmed = csvText.trim();
  if (!trimmed) return { headers: [], rows: [] };

  const parsedRows: string[][] = [];
  let currentRow: string[] = [];
  let currentField = "";
  let inQuotes = false;

  for (let i = 0; i < trimmed.length; i++) {
    const ch = trimmed[i];

    if (inQuotes) {
      if (ch === '"') {
        if (i + 1 < trimmed.length && trimmed[i + 1] === '"') {
          currentField += '"';
          i++;
        } else {
          inQuotes = false;
        }
      } else {
        currentField += ch;
      }
    } else {
      if (ch === '"') {
        inQuotes = true;
      } else if (ch === ",") {
        currentRow.push(currentField.trim());
        currentField = "";
      } else if (ch === "\r") {
        if (i + 1 < trimmed.length && trimmed[i + 1] === "\n") {
          i++;
        }
        currentRow.push(currentField.trim());
        currentField = "";
        parsedRows.push(currentRow);
        currentRow = [];
      } else if (ch === "\n") {
        currentRow.push(currentField.trim());
        currentField = "";
        parsedRows.push(currentRow);
        currentRow = [];
      } else {
        currentField += ch;
      }
    }
  }

  if (currentField || currentRow.length > 0) {
    currentRow.push(currentField.trim());
    parsedRows.push(currentRow);
  }

  if (parsedRows.length === 0) return { headers: [], rows: [] };

  const headers = parsedRows[0];
  const rows: Record<string, any>[] = [];

  for (let r = 1; r < parsedRows.length; r++) {
    const rowCells = parsedRows[r];
    if (rowCells.length === 0 || (rowCells.length === 1 && !rowCells[0])) continue;

    const rowObj: Record<string, any> = {};
    for (let c = 0; c < headers.length; c++) {
      const header = headers[c];
      if (!header) continue;
      const raw = rowCells[c] ?? "";
      const num = Number(raw);
      if (raw !== "" && !isNaN(num)) {
        rowObj[header] = num;
      } else if (raw.toLowerCase() === "true") {
        rowObj[header] = true;
      } else if (raw.toLowerCase() === "false") {
        rowObj[header] = false;
      } else {
        rowObj[header] = raw;
      }
    }
    rows.push(rowObj);
  }

  return { headers, rows };
}

