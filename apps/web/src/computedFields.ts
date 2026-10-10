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
  if (!expression || typeof expression !== "string") return null;
  const expr = expression.trim();
  if (!expr || expr.length > 500) return null;

  const tokens = tokenize(expr);
  if (!tokens) return null;

  const parser = new FormulaParser(tokens, record);
  return parser.parse();
}

type TokenType =
  | "NUMBER"
  | "STRING"
  | "FIELD"
  | "IDENT"
  | "PLUS"
  | "MINUS"
  | "STAR"
  | "SLASH"
  | "EQ"
  | "NEQ"
  | "LT"
  | "LTE"
  | "GT"
  | "GTE"
  | "LPAREN"
  | "RPAREN"
  | "COMMA"
  | "EOF";

interface Token {
  type: TokenType;
  value?: any;
}

function tokenize(input: string): Token[] | null {
  const tokens: Token[] = [];
  let i = 0;
  const n = input.length;

  while (i < n) {
    const ch = input[i];

    if (/\s/.test(ch)) {
      i++;
      continue;
    }

    if (ch === "{") {
      const close = input.indexOf("}", i);
      if (close === -1) return null;
      tokens.push({ type: "FIELD", value: input.slice(i + 1, close) });
      i = close + 1;
      continue;
    }

    if (ch === '"' || ch === "'") {
      let str = "";
      i++;
      let closed = false;
      while (i < n) {
        if (input[i] === ch) {
          closed = true;
          i++;
          break;
        }
        if (input[i] === "\\") {
          i++;
          if (i < n) {
            str += input[i];
            i++;
          }
        } else {
          str += input[i];
          i++;
        }
      }
      if (!closed) return null;
      tokens.push({ type: "STRING", value: str });
      continue;
    }

    if (/[0-9]/.test(ch) || (ch === "." && i + 1 < n && /[0-9]/.test(input[i + 1]))) {
      let numStr = "";
      while (i < n && /[0-9.]/.test(input[i])) {
        numStr += input[i];
        i++;
      }
      const num = Number(numStr);
      if (isNaN(num)) return null;
      tokens.push({ type: "NUMBER", value: num });
      continue;
    }

    if (/[a-zA-Z_]/.test(ch)) {
      let idStr = "";
      while (i < n && /[a-zA-Z0-9_]/.test(input[i])) {
        idStr += input[i];
        i++;
      }
      tokens.push({ type: "IDENT", value: idStr });
      continue;
    }

    if (ch === "!" && i + 1 < n && input[i + 1] === "=") {
      tokens.push({ type: "NEQ" });
      i += 2;
      continue;
    }
    if (ch === "<" && i + 1 < n && input[i + 1] === "=") {
      tokens.push({ type: "LTE" });
      i += 2;
      continue;
    }
    if (ch === ">" && i + 1 < n && input[i + 1] === "=") {
      tokens.push({ type: "GTE" });
      i += 2;
      continue;
    }
    if (ch === "<" && i + 1 < n && input[i + 1] === ">") {
      tokens.push({ type: "NEQ" });
      i += 2;
      continue;
    }

    switch (ch) {
      case "+": tokens.push({ type: "PLUS" }); break;
      case "-": tokens.push({ type: "MINUS" }); break;
      case "*": tokens.push({ type: "STAR" }); break;
      case "/": tokens.push({ type: "SLASH" }); break;
      case "=": tokens.push({ type: "EQ" }); break;
      case "<": tokens.push({ type: "LT" }); break;
      case ">": tokens.push({ type: "GT" }); break;
      case "(": tokens.push({ type: "LPAREN" }); break;
      case ")": tokens.push({ type: "RPAREN" }); break;
      case ",": tokens.push({ type: "COMMA" }); break;
      default: return null;
    }
    i++;
  }

  tokens.push({ type: "EOF" });
  return tokens;
}

class FormulaParser {
  private pos = 0;
  private depth = 0;

  constructor(
    private tokens: Token[],
    private record: Record<string, any>
  ) {}

  private peek(): Token {
    return this.tokens[this.pos] || { type: "EOF" };
  }

  private advance(): Token {
    const t = this.tokens[this.pos];
    this.pos++;
    return t;
  }

  private isTruthy(v: any): boolean {
    if (v === null || v === undefined || v === "" || v === 0 || v === false) {
      return false;
    }
    return true;
  }

  public parse(): any {
    try {
      const res = this.expr();
      return res;
    } catch {
      return null;
    }
  }

  private withDepth<T>(fn: () => T): T {
    this.depth++;
    if (this.depth > 32) throw new Error("Max depth exceeded");
    try {
      return fn();
    } finally {
      this.depth--;
    }
  }

  private expr(): any {
    return this.or();
  }

  private or(): any {
    return this.withDepth(() => {
      let left = this.and();
      while (
        this.peek().type === "IDENT" &&
        this.peek().value?.toUpperCase() === "OR"
      ) {
        this.advance();
        const right = this.and();
        left = Boolean(this.isTruthy(left) || this.isTruthy(right));
      }
      return left;
    });
  }

  private and(): any {
    return this.withDepth(() => {
      let left = this.not();
      while (
        this.peek().type === "IDENT" &&
        this.peek().value?.toUpperCase() === "AND"
      ) {
        this.advance();
        const right = this.not();
        left = Boolean(this.isTruthy(left) && this.isTruthy(right));
      }
      return left;
    });
  }

  private not(): any {
    return this.withDepth(() => {
      if (
        this.peek().type === "IDENT" &&
        this.peek().value?.toUpperCase() === "NOT"
      ) {
        this.advance();
        const inner = this.not();
        return !this.isTruthy(inner);
      }
      return this.cmp();
    });
  }

  private cmp(): any {
    return this.withDepth(() => {
      const left = this.add();
      const p = this.peek();
      if (
        p.type === "EQ" ||
        p.type === "NEQ" ||
        p.type === "LT" ||
        p.type === "LTE" ||
        p.type === "GT" ||
        p.type === "GTE"
      ) {
        const op = this.advance().type;
        const right = this.add();
        return this.compare(left, right, op);
      }
      return left;
    });
  }

  private compare(left: any, right: any, op: TokenType): boolean {
    if (typeof left === "number" && typeof right === "number") {
      switch (op) {
        case "EQ": return left === right;
        case "NEQ": return left !== right;
        case "LT": return left < right;
        case "LTE": return left <= right;
        case "GT": return left > right;
        case "GTE": return left >= right;
      }
    }
    const sLeft = left === null || left === undefined ? "" : String(left);
    const sRight = right === null || right === undefined ? "" : String(right);
    switch (op) {
      case "EQ":
        return left === right || sLeft === sRight;
      case "NEQ":
        return left !== right && sLeft !== sRight;
      case "LT": return sLeft < sRight;
      case "LTE": return sLeft <= sRight;
      case "GT": return sLeft > sRight;
      case "GTE": return sLeft >= sRight;
    }
    return false;
  }

  private add(): any {
    return this.withDepth(() => {
      let left = this.mul();
      while (this.peek().type === "PLUS" || this.peek().type === "MINUS") {
        const op = this.advance().type;
        const right = this.mul();
        if (op === "PLUS") {
          if (typeof left === "string" || typeof right === "string") {
            const sl = left === null || left === undefined ? "" : String(left);
            const sr = right === null || right === undefined ? "" : String(right);
            left = sl + sr;
          } else if (typeof left === "number" && typeof right === "number") {
            left = left + right;
          } else {
            left = null;
          }
        } else {
          if (typeof left === "number" && typeof right === "number") {
            left = left - right;
          } else {
            left = null;
          }
        }
      }
      return left;
    });
  }

  private mul(): any {
    return this.withDepth(() => {
      let left = this.unary();
      while (this.peek().type === "STAR" || this.peek().type === "SLASH") {
        const op = this.advance().type;
        const right = this.unary();
        if (typeof left === "number" && typeof right === "number") {
          if (op === "STAR") {
            left = left * right;
          } else {
            if (right === 0) {
              left = null;
            } else {
              left = left / right;
            }
          }
        } else {
          left = null;
        }
      }
      return left;
    });
  }

  private unary(): any {
    return this.withDepth(() => {
      if (this.peek().type === "MINUS") {
        this.advance();
        const inner = this.unary();
        if (typeof inner === "number") {
          return -inner;
        }
        return null;
      }
      return this.primary();
    });
  }

  private primary(): any {
    return this.withDepth(() => {
      const p = this.peek();

      if (p.type === "NUMBER") {
        this.advance();
        return p.value;
      }

      if (p.type === "STRING") {
        this.advance();
        return p.value;
      }

      if (p.type === "FIELD") {
        this.advance();
        const val = this.record[p.value];
        if (val === undefined || val === null) return null;
        return val;
      }

      if (p.type === "LPAREN") {
        this.advance();
        const val = this.expr();
        if (this.peek().type === "RPAREN") {
          this.advance();
        } else {
          throw new Error("Expected closing parenthesis");
        }
        return val;
      }

      if (p.type === "IDENT") {
        const idToken = this.advance();
        if (this.peek().type === "LPAREN") {
          this.advance();
          const args: any[] = [];
          if (this.peek().type !== "RPAREN") {
            args.push(this.expr());
            while (this.peek().type === "COMMA") {
              this.advance();
              args.push(this.expr());
            }
          }
          if (this.peek().type === "RPAREN") {
            this.advance();
          } else {
            throw new Error("Expected closing parenthesis");
          }
          return this.callFunc(idToken.value.toUpperCase(), args);
        }
        return null;
      }

      return null;
    });
  }

  private callFunc(name: string, args: any[]): any {
    switch (name) {
      case "IF": {
        const cond = args[0];
        if (this.isTruthy(cond)) {
          return args.length > 1 ? args[1] : null;
        } else {
          return args.length > 2 ? args[2] : null;
        }
      }
      case "AND": {
        if (args.length === 0) return null;
        return args.every((a) => this.isTruthy(a));
      }
      case "OR": {
        if (args.length === 0) return null;
        return args.some((a) => this.isTruthy(a));
      }
      case "NOT": {
        if (args.length === 0) return null;
        return !this.isTruthy(args[0]);
      }
      case "ROUND": {
        if (typeof args[0] !== "number") return null;
        const digits = typeof args[1] === "number" ? args[1] : 0;
        const factor = Math.pow(10, digits);
        return Math.round(args[0] * factor) / factor;
      }
      case "ABS": {
        if (typeof args[0] !== "number") return null;
        return Math.abs(args[0]);
      }
      case "CONCAT": {
        let str = "";
        for (const a of args) {
          if (a !== null && a !== undefined) {
            str += String(a);
          }
        }
        return str;
      }
      case "LEN": {
        const val = args[0];
        if (val === null || val === undefined) return 0;
        return String(val).length;
      }
      case "BLANK": {
        return null;
      }
      case "ISBLANK": {
        const val = args[0];
        return val === null || val === undefined || val === "";
      }
      default:
        return null;
    }
  }
}

export function parseOperand(op: string, record: Record<string, any>): any {
  return evaluateClientFormula(op, record);
}

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

