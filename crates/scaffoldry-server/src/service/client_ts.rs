//! A TypeScript module for an outside app (external apps phase 4).
//!
//! `generate` turns the API description `openapi::describe` built for one caller into one TypeScript file. It has a type
//! per table, `list`, `get`, `create`, `update` and `remove`, cursor paging and the error codes. It uses `fetch` and
//! nothing else. It never stores a token: the app gives it a function that returns one on every call. For a client
//! token the description already holds only what the ceiling reads, and this file leaves out each operation and field
//! the ceiling does not allow.

use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::fmt::Write;

const RUNTIME: &str = r#"export type TokenSource = () => string | Promise<string>;

export interface ClientOptions {
  /** The address of Scaffoldry, such as https://scaffoldry.example.edu. */
  baseUrl: string;
  /** Returns the access token. It is called on every request and never stored here. */
  token: TokenSource;
  fetch?: typeof fetch;
}

export class ScaffoldryError extends Error {
  status: number;
  code: ErrorCode | "unknown";
  remedy: string | undefined;
  constructor(status: number, body: unknown) {
    const b = (typeof body === "object" && body !== null ? body : {}) as { error?: string; code?: string; remedy?: string };
    super(b.error ?? "The request failed.");
    this.status = status;
    this.code = (b.code as ErrorCode | undefined) ?? "unknown";
    this.remedy = b.remedy;
  }
}

export interface Row<T> {
  id: string;
  version: number;
  data: T;
}

export interface Page<T> {
  records: Row<T>[];
  next_cursor?: string | null;
}

export interface ListParams {
  limit?: number;
  cursor?: string;
}

async function call(o: ClientOptions, method: string, path: string, body?: unknown): Promise<unknown> {
  const send = o.fetch ?? fetch;
  const token = await o.token();
  const headers: Record<string, string> = { authorization: "Bearer " + token };
  if (body !== undefined) headers["content-type"] = "application/json";
  const res = await send(o.baseUrl.replace(/\/+$/, "") + "/api/v1" + path, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  let parsed: unknown = null;
  try {
    parsed = text ? JSON.parse(text) : null;
  } catch {
    parsed = null;
  }
  if (!res.ok) throw new ScaffoldryError(res.status, parsed);
  return parsed;
}

function base(slug: string, table: string): string {
  return "/apps/" + encodeURIComponent(slug) + "/tables/" + encodeURIComponent(table) + "/records";
}

function query(p: ListParams): string {
  const pairs: string[] = [];
  if (p.limit !== undefined) pairs.push("limit=" + p.limit);
  if (p.cursor !== undefined) pairs.push("cursor=" + encodeURIComponent(p.cursor));
  return pairs.length ? "?" + pairs.join("&") : "";
}

function listOf<T>(o: ClientOptions, slug: string, table: string) {
  return (params: ListParams = {}) => call(o, "GET", base(slug, table) + query(params)) as Promise<Page<T>>;
}

/** Every record, one page at a time. Keep `limit` small. */
function pagesOf<T>(o: ClientOptions, slug: string, table: string) {
  return async function* (params: ListParams = {}): AsyncGenerator<Row<T>> {
    let cursor = params.cursor;
    for (;;) {
      const page = (await call(o, "GET", base(slug, table) + query({ ...params, cursor }))) as Page<T>;
      yield* page.records;
      if (!page.next_cursor) return;
      cursor = page.next_cursor;
    }
  };
}

function getOf<T>(o: ClientOptions, slug: string, table: string) {
  return (id: string) => call(o, "GET", base(slug, table) + "/" + encodeURIComponent(id)) as Promise<Row<T>>;
}

function createOf<T, I>(o: ClientOptions, slug: string, table: string) {
  return (data: I) => call(o, "POST", base(slug, table), { data }) as Promise<Row<T>>;
}

function updateOf<T, I>(o: ClientOptions, slug: string, table: string) {
  return (id: string, version: number, data: I) =>
    call(o, "PATCH", base(slug, table) + "/" + encodeURIComponent(id), { data, version }) as Promise<Row<T>>;
}

function removeOf(o: ClientOptions, slug: string, table: string) {
  return (id: string, version: number) =>
    call(o, "DELETE", base(slug, table) + "/" + encodeURIComponent(id) + "?version=" + version) as Promise<unknown>;
}
"#;

/// The operations a caller may use on one table.
#[derive(Clone, Copy)]
struct Ops {
    read: bool,
    create: bool,
    update: bool,
    delete: bool,
}

const ALL: Ops = Ops { read: true, create: true, update: true, delete: true };

/// A name that is a valid TypeScript type name: words joined and capitalised, with a letter first.
fn type_name(raw: &str, taken: &mut BTreeSet<String>) -> String {
    let mut name: String = raw
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut cs = w.chars();
            cs.next().map(|f| f.to_ascii_uppercase().to_string() + cs.as_str()).unwrap_or_default()
        })
        .collect();
    if !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
        name.insert(0, 'T');
    }
    let mut unique = name.clone();
    let mut n = 2;
    while !taken.insert(unique.clone()) {
        unique = format!("{name}{n}");
        n += 1;
    }
    unique
}

fn quoted(s: &str) -> String {
    Value::String(s.to_string()).to_string()
}

/// The TypeScript type of a JSON Schema from `openapi::field_schema`.
fn ts_type(schema: &Value) -> String {
    if let Some(options) = schema.get("enum").and_then(Value::as_array) {
        let union: Vec<String> = options.iter().filter_map(Value::as_str).map(quoted).collect();
        if !union.is_empty() {
            return union.join(" | ");
        }
    }
    match schema.get("type").and_then(Value::as_str) {
        Some("string") => "string".to_string(),
        Some("number" | "integer") => "number".to_string(),
        Some("boolean") => "boolean".to_string(),
        Some("array") => {
            let item = schema.get("items").map(ts_type).unwrap_or_else(|| "unknown".to_string());
            format!("Array<{item}>")
        }
        Some("object") => "Record<string, unknown>".to_string(),
        _ => "unknown".to_string(),
    }
}

fn property(name: &str, schema: &Value, optional: bool, readonly: bool) -> String {
    let title = schema.get("title").and_then(Value::as_str).unwrap_or_default().replace("*/", "");
    format!(
        "  /** {title} */\n  {}{}{}: {};\n",
        if readonly { "readonly " } else { "" },
        quoted(name),
        if optional { "?" } else { "" },
        ts_type(schema)
    )
}

/// The table ids and slug in the description's paths, in order.
fn tables_in(document: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for path in document.get("paths").and_then(Value::as_object).into_iter().flat_map(Map::keys) {
        let Some(rest) = path.strip_prefix("/apps/") else { continue };
        let Some((slug, rest)) = rest.split_once("/tables/") else { continue };
        if let Some(table) = rest.strip_suffix("/records") {
            out.push((slug.to_string(), table.to_string()));
        }
    }
    out
}

/// What the caller's ceiling allows on one table. A caller with no ceiling may use every operation and write every field.
fn limits(caller: &AuthUser, slug: &str, table: &str) -> (Ops, Option<Vec<String>>) {
    let Some(client) = &caller.via_client else { return (ALL, None) };
    let allowed = client.ceiling.apps.iter().filter(|a| a.app_slug == slug).flat_map(|a| &a.tables).find(|t| t.table_id == table);
    match allowed {
        Some(t) => (Ops { read: !t.read.is_empty(), create: t.create, update: !t.write.is_empty(), delete: t.delete }, Some(t.write.clone())),
        None => (Ops { read: false, create: false, update: false, delete: false }, Some(Vec::new())),
    }
}

/// The module for `slug`, as the caller sees it.
pub fn generate(caller: &AuthUser, slug: &str, state: &SharedState) -> Result<String, ServiceError> {
    let document = crate::service::openapi::describe(caller, slug, state)?;
    let schemas = document["components"]["schemas"].as_object().cloned().unwrap_or_default();
    let title = document["info"]["title"].as_str().unwrap_or(slug).replace("*/", "");

    let mut out = String::new();
    let _ = writeln!(out, "// Generated by Scaffoldry for the app {} ({}). Do not edit: fetch it again when the app changes.", quoted(&title), quoted(slug));
    let _ = writeln!(out, "// It uses fetch and nothing else. The access token comes from the function you give it on every call.\n");
    let codes: Vec<String> = crate::service::ERROR_CODES.iter().map(|c| quoted(c)).collect();
    let _ = writeln!(out, "export type ErrorCode = {};\n", codes.join(" | "));
    out.push_str(RUNTIME);

    let mut taken: BTreeSet<String> = ["TokenSource", "ClientOptions", "ScaffoldryError", "Row", "Page", "ListParams", "ErrorCode"].iter().map(|s| s.to_string()).collect();
    let mut body = String::new();
    for (app, table) in tables_in(&document).into_iter().filter(|(a, _)| a == slug) {
        let schema_key: String = table.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' { c } else { '_' }).collect();
        let Some(schema) = schemas.get(&schema_key) else { continue };
        let (ops, writable) = limits(caller, &app, &table);
        if !(ops.read || ops.create || ops.update || ops.delete) {
            continue;
        }
        let name = type_name(&table, &mut taken);
        let input = type_name(&format!("{table} input"), &mut taken);
        let required: BTreeSet<&str> = schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        let properties = schema["properties"].as_object().cloned().unwrap_or_default();

        let _ = writeln!(out, "/** A record of {}. */\nexport interface {name} {{", quoted(schema["title"].as_str().unwrap_or(&table)));
        for (field, s) in &properties {
            out.push_str(&property(field, s, true, s.get("readOnly").and_then(Value::as_bool).unwrap_or(false)));
        }
        out.push_str("}\n\n");

        let _ = writeln!(out, "/** What a write may send to {}. */\nexport interface {input} {{", quoted(&table));
        for (field, s) in &properties {
            let computed = s.get("readOnly").and_then(Value::as_bool).unwrap_or(false);
            let allowed = writable.as_ref().is_none_or(|w| w.contains(field));
            if computed || !allowed {
                continue;
            }
            // An update through an outside app sends the fields it changes. The server keeps the rest.
            let optional = !required.contains(field.as_str()) || caller.via_client.is_some() && !ops.create;
            out.push_str(&property(field, s, optional, false));
        }
        out.push_str("}\n\n");

        let mut members = Vec::new();
        if ops.read {
            members.push(format!("      list: listOf<{name}>(o, slug, {t}),\n      all: pagesOf<{name}>(o, slug, {t}),\n      get: getOf<{name}>(o, slug, {t}),", t = quoted(&table)));
        }
        if ops.create {
            members.push(format!("      create: createOf<{name}, {input}>(o, slug, {}),", quoted(&table)));
        }
        if ops.update {
            let partial = if caller.via_client.is_some() { format!("Partial<{input}>") } else { input.clone() };
            members.push(format!("      update: updateOf<{name}, {partial}>(o, slug, {}),", quoted(&table)));
        }
        if ops.delete {
            members.push(format!("      remove: removeOf(o, slug, {}),", quoted(&table)));
        }
        let _ = writeln!(body, "    {}: {{\n{}\n    }},", quoted(&table), members.join("\n"));
    }
    let _ = write!(
        out,
        "/** The app {}. Give it the address of Scaffoldry and a function that returns an access token. */\nexport function createClient(o: ClientOptions) {{\n  const slug = {};\n  return {{\n    tables: {{\n{body}    }},\n  }};\n}}\n",
        quoted(slug),
        quoted(slug)
    );
    Ok(out)
}
