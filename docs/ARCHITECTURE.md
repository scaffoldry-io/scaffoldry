# Scaffoldry Platform Architecture

**Version:** 2.0 (Clean Foundation)  
**Date:** October 2026  
**License:** Apache License 2.0  
**Website:** https://scaffoldry.io  
**Repository:** https://github.com/scaffoldry-io/scaffoldry

---

## 1. Executive Summary & Purpose

Scaffoldry is an open-source, AI-native decision and application platform designed for **distributed authority environments**, such as higher education, academic medical centers, and research institutes.

### The Institutional Problem
In distributed institutions, central IT and individual departments exist in structural tension:
- **Central IT** requires security, regulatory compliance (FERPA, HIPAA, NIST SP 800-171), and institutional data coordination. This often leads to rigid ERP implementations that take years to adapt to departmental needs.
- **Deans, Department Chairs, and Lab Directors** hold autonomous decision authority and need immediate business solutions. Facing central IT delays, they adopt commercial SaaS tools (Airtable, Smartsheet, Google Sheets).
- **The Failure Mode:** Data silos, unmonitored compliance exposure, vendor lock-in, and unpredictable per-seat pricing.

### The Scaffoldry Solution
Scaffoldry provides an institution-owned appliance that gives end users the power to build custom business applications with AI, while maintaining hard boundaries on data integrity, decision authority, and regulatory compliance.

---

## 2. Core Architectural Principles

All development in this repository must adhere to four governing principles:

### 2.1 Sovereign and Non-Rugpullable
Scaffoldry must remain free from commercial license traps. 
- Build only on technologies governed by neutral non-profit foundations (Apache Software Foundation, Linux Foundation, PostgreSQL Global Development Group).
- Use strictly permissive licenses: **Apache 2.0, MIT, BSD-2-Clause, BSD-3-Clause, ISC, CC0-1.0, and the PostgreSQL License**.
- Never introduce dependencies governed by venture-backed open-core licenses (BSL, SSPL, or proprietary enterprise tiers).

### 2.2 The Modular Monolith Appliance
Scaffoldry ships as a self-contained appliance that runs on **one standard Linux machine** that existing enterprise sysadmins already know how to patch.
- Deployable via standard Docker Compose.
- Zero nested hypervisors, zero QEMU virtualization, and zero complex hyper-orchestration.
- No dependency on proprietary cloud serverless runtimes. An institution must be able to run Scaffoldry entirely inside their own on-prem firewall or on a sovereign cloud VM.

#### 2.3 Separation of Concerns
The platform cleanly separates four critical duties:
1. **Calculation:** Performed in-memory by `scaffoldry-engine` for responsive spreadsheet interactivity.
2. **Durability:** Handled by an ACID-compliant PostgreSQL 17 relational database.
3. **Authorization:** Decided by an embedded Cedar policy engine at the API boundary.
4. **Audit & Governance:** Permanently recorded in an immutable SHA-256 cryptographic decision ledger.

### 2.4 PostgreSQL Is the Only Source of Truth
PostgreSQL holds every fact the platform keeps, and a server holds nothing between requests that it cannot read again from the database. A server keeps copies of eleven collections of configuration, and only these: `settings`, `labels`, `position_types`, `organizations`, `workspaces` (with their collaborators), `guards`, `datasets`, `relationships`, `manifests` (with the saved views of each app), `rules`, and `policy`. Each has a counter in the `cache_versions` table. A writer raises the counter in the transaction that writes, and every request reads the counters first and reloads any collection whose counter moved. People, appointments, sessions, tokens, records, process instances, and the ledger are never copied: they are read from the database each time, and a decision on a waiting step or an append to the ledger locks the row it changes. Because of this, any number of server instances may serve one database. A write through one is read through the next request to any other, a request that reaches an instance that cannot read the database fails with `storage_unavailable`, and `GET /readyz` tells a load balancer whether an instance can read it. On `SIGTERM` an instance stops taking connections, finishes the requests it holds for up to 30 seconds, lets the job it is running finish, and exits.

### 2.5 Write for People
All documentation, user interfaces, error messages, and commit histories must follow the `write-for-people` standard:
- One idea per sentence.
- Use plain, precise language instead of academic or artificial jargon.
- State instructions before explanations.
- Name the actor and the action explicitly.

---

## 3. The Six-Layer Architecture

```
+-----------------------------------------------------------------------------------------+
|                                    LAYER 1: USER INTERFACE                              |
|                          React 19, Tailwind CSS & TanStack Query/Table                  |
|                                                                                         |
|       * Reactive Spreadsheet Grid           * Dynamic Application Forms                 |
|       * Business Approval Desk              * Natural-Language AI Builder               |
+-----------------------------------------------------------------------------------------+
                                             |
                                   (HTTP REST / JSON-RPC)
                                             v
+-----------------------------------------------------------------------------------------+
|                                LAYER 2: ACCESS & POLICY                                 |
|                                   Cedar Policy Engine                                   |
|                                                                                         |
|       * Real-time authorization checks for every cell, view, and action                 |
|       * Clear separation of departmental authority vs. central IT controls              |
|       * Declarative rules written in plain policy syntax                                |
+-----------------------------------------------------------------------------------------+
                                             |
                                             v
+-----------------------------------------------------------------------------------------+
|                               LAYER 3: CALCULATION ENGINE                               |
|                         Sovereign Rust Engine (scaffoldry-engine)                       |
|                                                                                         |
|       * In-memory typed representation of manifests, views, and datasets                |
|       * Real-time formula dependency graphs, relational rollups, and lookups            |
|       * High-speed analytical evaluation without database lockups                       |
+-----------------------------------------------------------------------------------------+
          |                                                               |
          | (Sync Records)                                                | (Record Approved Decisions)
          v                                                               v
+------------------------------------+           +----------------------------------------+
|      LAYER 4: DURABLE STORE        |           |       LAYER 5: DECISION LEDGER         |
|            PostgreSQL 17           |           |      Cryptographic SHA-256 Chain       |
|                                    |           |                                        |
|  * Persistent disk storage         |           |  * Immutable cryptographic block chain  |
|  * Normalized core tables          |           |  * Sequential SHA-256 hash validation  |
|  * JSONB for dynamic user fields   |           |  * Tamper detection on server boot     |
|  * Standard daily pg_dump backups  |           |  * OSCAL AU-02 audit record continuity |
+------------------------------------+           +----------------------------------------+
                                             |
                                             v
+-----------------------------------------------------------------------------------------+
|                               LAYER 6: COMPLIANCE STANDARD                              |
|                                     NIST OSCAL 1.1.2                                    |
|                                                                                         |
|       * Machine-readable catalogs, profiles, and System Security Plans (SSPs)           |
|       * Direct export for university CISOs, auditors, and enterprise GRC tools          |
|       * Pre-mapped baselines: FERPA, HIPAA, NIST SP 800-171, and CMMC                   |
+-----------------------------------------------------------------------------------------+
```

---

## 4. Layer Responsibilities & Implementation Details

### Layer 1: User Interface
- **Technology:** React 19, `@tanstack/react-table`, `@tanstack/react-query`, and Tailwind CSS.
- **Responsibilities:**
  - Render high-density, virtualized spreadsheet grids that stay smooth at 10,000+ rows.
  - Expose the **Business Approval Desk**: an intuitive, non-technical dashboard where Department Chairs and compliance officers approve or reject proposed changes.

### Layer 2: Access & Policy (Cedar)
- **Technology:** Cedar Policy Engine (Apache 2.0, Rust).
- **Responsibilities:**
  - Execute real-time Attribute-Based Access Control (ABAC).
  - Enforce data sensitivity gates (e.g., student financial data is hidden from external faculty reviewers).
  - Evaluate decision approval authority across workspaces, records, apps, and datasets.

### Layer 3: Calculation Engine (Sovereign Rust Engine)
- **Technology:** `scaffoldry-engine` (native Rust).
- **Responsibilities:**
  - Hold active sheet schemas, views, and field specifications in memory.
  - Recompute calculated formulas, relational rollups, and lookups across columns in RAM.
  - Eliminate the need to issue complex, blocking SQL queries to PostgreSQL during interactive user editing.

### Layer 4: Durable Operational Store (PostgreSQL)
- **Technology:** PostgreSQL 17 (official image on Alpine Linux).
- **Responsibilities:**
  - Provide durable ACID transactions for live data.
  - Store core institutional entities (Workspaces, Collaborators, App Manifests, Published Datasets, Relationships, Automations, Sessions, SCIM) in normalized tables with advisory-lock migrations.
  - Store dynamic, user-defined fields in indexed `JSONB` columns with generated expression indexes.

### Layer 5: Decision & Audit Ledger (Cryptographic SHA-256 Blockchain)
- **Technology:** Sovereign Cryptographic Ledger (`governance_ledger` in PostgreSQL 17).
- **Responsibilities:**
  - Maintain an immutable, tamper-evident SHA-256 hash chain from genesis block 0 through block N.
  - Every publication, DNS binding, policy revision, and workflow approval appends a block linking `previous_hash` and computing `entry_hash`.
  - On every server boot, verify the cryptographic integrity of the entire chain. Refuse to start if any block hash mismatch or discontinuity is detected.
  - Provides a permanent cryptographic paper trail that aligns with NIST OSCAL AU-02.

### Layer 6: Compliance Standard (NIST OSCAL)
- **Technology:** NIST OSCAL 1.1.2 JSON Schema.
- **Responsibilities:**
  - Serve as the export lattice for institutional compliance reporting.
  - Allow university compliance officers to generate automated System Security Plans (SSPs) directly from the running appliance.
  - Validated via automated CI scripts (`governance/scripts/validate-oscal.py`).

### Organization Tree & Scope Hierarchy
- **Standards Alignment:** NCES CEDS (Common Education Data Standards) organization hierarchy; REFEDS eduPerson scoped affiliations (`faculty`, `staff`, `student`, `employee`, `member`, `affiliate`, `alum`).
- **Hierarchy Structure:**
  - Strict single-parent tree: `organizations.parent_id` points to exactly one parent. Closed `org_type` values (`Institution`, `College`, `Department`, `Center`, `Program`). There is exactly one root row (`org_type = "Institution"`, `parent_id = NULL`).
  - Scoping Pure Function: `unit_in_scope` walks ancestors with an explicit 32-step cycle guard. If an organization node is unreachable or cycles, traversal terminates safely.
- **Administrative Roles & UI Labels:**
  - **Platform Admin:** Stored affiliation `central_admin` or root `platform_admin` role. Has supervisory access to all units, all workspaces, and the full `/admin` console (policy, ledger, infra, SCIM, and org tree management).
  - **Compliance officer:** Stored role `roles.scoped_affiliation = "compliance"` on the root unit, appointed by a Platform Admin through `POST /orgs/{id}/appointments` and recorded as an `AccessRoleGranted` ledger entry. The holder's affiliation resolves to `compliance`, which opens the disclosure log, the governance ledger and policy reads, and the other compliance routes. It is a role and not a claim from the identity provider: a SCIM record that says `compliance@...` grants nothing, so the registry cannot hand out this access. Like `central_admin`, the role replaces the person's registry affiliation while it is held.
  - **Org Unit Admin:** Stored role `roles.scoped_affiliation = "unit_admin"`. Scoped strictly to that unit and its descendants. Sees "Your units" on the workspace rail filter and can create workspaces within assigned units. Never gets `/admin` policy or ledger access.
  - *Strict Rule:* Never use the labels "Super Admin", "System Admin", or "Org Admin". Admin status is never inferred from job titles (e.g., "Chair" or "Dean").
- **Identity & SCIM Synchronization:**
  - SCIM 2.0 provisioning route synchronizes enterprise academic/position attributes (`sync_org_roles`).
  - Maps `department`, `division`, and `organization` to tree units; deepest match receives `is_primary = true`.
  - Roles carry a `source` tag (`scim` or `api`). SCIM resync drops only `source = "scim"` records, preserving manual API appointments (`source = "api"`).
- **Workspace Scoping:**
  - Workspaces reside in organization units (`workspaces.organization_id`).
  - `list_workspaces` evaluates `unit_in_scope` before checking individual workspace collaborator roles, preventing cross-department workspace leakage.

### Spreadsheet Data Grid Parity
- **Technology:** React 19, `@tanstack/react-table`, custom virtualized canvas grid (`DataGrid.tsx`).
- **Interaction & Editing Parity:**
  - Type-aware inline cell editors across all rich types: Text, Number, Date, Select, Checkbox, MultiSelect, Currency, Percent, Rating, Email, Phone, Url, Formula, Linked records.
  - Excel/Airtable interaction model: Bulk rectangular cell selection, clipboard TSV copy/paste, bulk clear, drag-to-fill handle, and undo/redo history stack.
  - Column chrome controls: Column hiding, freeze/pin column left, multi-column sort, filter popovers, and type badges.
  - Column schema manipulation in `AppBuilder`: Dynamic field renaming, type conversion, formula expression editing, insert field left/right, and field deletion without requiring SQL DDL migrations.
  - Linked fields with relational characteristics: Target table binding, display label overrides, relational filtering, and cardinality enforcement (has-one vs has-many).
- **Calculation Engine:**
  - Client-side parser and evaluator (`evaluateClientFormula`) mirrors the sovereign Rust engine (`scaffoldry-engine`).
  - Evaluates arithmetic, string manipulation, logical conditions (`IF`), aggregation (`SUM`, `AVG`), and relational rollups in-memory.

### Sovereign Business Process Engine
- **Standards Alignment:** BPMN 2.0 vocabulary and semantics (Start events, Sequence flows, Conditional gates, Service tasks, User tasks, End events).
- **Core Architecture:**
  - **Action Effects as Data:** Workflow execution yields structured `ActionEffect` values (`UpdateRecordStatus`, `NotifyCollaborator`, `WebhookDispatch`, `CreateLedgerAuditEntry`, `CedarPolicyDenied`, `UserTaskCreated`, `RetriggerSuppressed`) rather than unverified log strings.
  - **Cedar ABAC Enforcement:** Every automated record modification and action execution is gated by real-time Cedar policy checks (`scaffoldry_policy::authorize_record_action`).
  - **Retrigger & Cycle Guard:** Embedded retrigger loop guard suppresses cascaded automation loops on same-record triggers, backed by a strict depth backstop (depth cap = 3).
  - **Human Decision Steps (UserTask):**
    - Workflow steps can declare `Step::UserTask` targeting specific roles or groups.
    - When a user task step is encountered, execution suspends: a `ProcessInstance` is persisted in PostgreSQL with status `Waiting` (`0005_process_instances.sql`).
    - The pending task surfaces in the **ProcessDesk** queue for authorized chairs and reviewers.
    - Human decisions (`Approve` or `Reject`) advance the process instance to `Completed` or `Rejected` and record a permanent `WorkflowRuleApproved` block into the SHA-256 `governance_ledger`.


### Standardized Interface: Model Context Protocol (MCP) as Authoritative API
- **Standards Search & Alignment:**
  - Core Protocol: Model Context Protocol (MCP) specification version `2024-11-05` over JSON-RPC 2.0.
  - Interactive UI Extension: MCP Apps extension (`modelcontextprotocol/ext-apps`, SEP-1865).
  - UI Resource Scheme: `ui://` URI scheme serving sandboxed HTML (`text/html;profile=mcp-app`) with bidirectional AppBridge messaging.
- **Responsibilities:**
  - Expose every platform capability (workspaces, apps, tabular records, datasets, audit ledger, and policy simulation) as an MCP tool or resource.
  - The MCP server is the single authoritative API for the platform.
  - REST endpoints exist as a thin compatibility adapter delegating to the same shared service layer.
  - Every MCP tool execution and resource read requires an authenticated session and evaluates Cedar ABAC before execution.

---

## 5. Deployment and Operations

Scaffoldry deploys as a single compose stack:

1. **Host Environment:** Any standard Linux distribution (Ubuntu 24.04 LTS recommended) with 2 vCPUs and 4 GB RAM.
2. **Public Ingress (Zero Open Inbound Ports):**
   - Deployed behind a **Cloudflare Tunnel** (`cloudflared`).
   - All inbound firewall ports (80, 443, 22) remain closed on the public interface.
   - Cloudflare provides edge TLS termination, DDoS defense, and optional Zero Trust access control.
3. **Automated Off-Machine Backup:**
   - A daily timer executes a snapshot script.
   - Dumps PostgreSQL (`pg_dump`). The ledger is contained directly within the PostgreSQL database dump.
   - Encrypts the snapshot archive and uploads it to off-machine object storage (Backblaze B2, Google Cloud Storage, or Proton Drive).

---

## 6. Application Lifecycle: Review, Deployment, and Security Maintenance

Applications created on Scaffoldry (e.g., Departmental Admissions Review, Grant Tracking) are backed by declarative definitions in Git. This architecture directly enables professional code review, automated security scanning, and fleet maintenance over time without subjecting end users to developer friction.

### 6.1 The Governed Review Workflow

Changes to an application never push directly to live data. Every creation, formula update, or schema alteration follows a governed path:

1. **The Proposal:** When an agent or user proposes changes to an app, the platform stores a proposal row holding the draft manifest.
2. **Automated Pre-Review Checks:** Before notifying a human reviewer, the platform runs automated checks:
   - **Data Sensitivity Scan:** Scans the schema and fields to detect FERPA, HIPAA, or restricted research data markers.
   - **Policy Evaluation:** Asserts that the requested changes comply with institutional rules via Cedar.
   - **Compliance Mapping:** Verifies that new data structures map to the relevant OSCAL catalog controls.
3. **The Human Review on The Desk:** Reviewers (such as Department Chairs or Compliance Officers) review the proposed changes through a clear business UI on The Desk:
   - Visual before-and-after view of the schema and formula changes.
   - Data sensitivity classification and required approvals.
   - Automated check results (Pass/Fail).
4. **Governed Deployment:** When approved, the system writes the manifest and appends an audit block to the `governance_ledger` in one transaction. The production database and calculation engine reload the new manifest instantly.

#### 6.1.1 Views Live Apart From the Manifest

A view is presentation, so it is not governed. Views are stored in `app_views`, not in the manifest, and changing one is not a proposal. A view cannot grant access, because every read still goes through the row scope and the field visibility rules, and a view over a field the person cannot see is refused when it is run.

- **Ownership decides who changes a view.** A `personal` view is its owner's alone. A `collaborative` view needs an editor. A `locked` view needs an admin, and everyone who can read the app may use it. Routes: `GET`/`POST /apps/{slug}/views`, `PUT`/`DELETE /apps/{slug}/views/{id}`, `POST .../duplicate`. The MCP tools are `list_views`, `save_view` and `delete_view`. A save names the version it read, and a stale one is a `409`.
- **Indexes follow views.** A save that sorts or filters on a field outside the table's indexed set starts that index in the same transaction (`index_state: building`), and a table may index 20 fields (`too_many_indexes` lists the indexed fields no view uses).
- **What runs is the manifest with the stored views merged in.** A manifest stored with views in it has them moved once, when the server starts, and the move is one `ViewsMigrated` ledger entry with the count. The engine's copy keeps the merged list, so validation, index derivation and the schema routes read `views` as before.

#### 6.1.2 What a View Can Do (Phase 2)

The server does the work. The browser draws what it is sent and never sorts, filters, groups or totals records. `GET /apps/{slug}/views/{id}/data` runs a view.

- **Filter trees.** A filter is a group: a conjunction over clauses and nested groups, two levels deep below the root and 50 clauses at most. The new operators are `In` (a JSON array or a comma list), `Between` (`low..high`, both inside), `Before` and `After`. A date range may be a relative word: `today`, `this_week` (opens Monday), `last_7_days`, `last_30_days` (today and the days before) or `next_30_days`. The server turns the word into whole local days in `platform.timezone`, then compiles the filter. Tests prove the window is the same at 00:30 and at 23:30 local time. Over the index, each clause past the walked field is an `EXISTS` probe of the same record, so a tree on indexed fields never scans the table. A tree that names a field outside the index is allowed only on a table of 10,000 rows or fewer, and then it reads the record JSON.
- **Grouping.** A view groups by up to three fields (`group_by`). With no `group_path`, the route returns the groups of the first level with counts, at most 200 per level, and `has_more` says when there are others. A `group_path` of the keys chosen so far (a JSON array) returns the next level, and a path as long as the list returns that group's rows, paged by the ordinary cursor. A key of `""` is the group with no value. The groups come from the same index statement as the aggregate route.
- **Footers.** `column_summary` holds one choice per field: `count`, `filled`, `empty`, `unique`, `sum`, `avg`, `min` or `max`. Each is one aggregate statement over the whole filtered set and rides on the first page only. A field the person cannot see has no footer, and a function the field's type cannot answer says `unsupported`.
- **Color rules.** A view holds ten rules at most, each `{ id, label, when, tone, target }` with `tone` one of `neutral`, `info`, `success`, `warning`, `danger`, and `target` of `row` or `field` (a field rule names the field). The server evaluates the rules on the rows it returns and adds `_tone`, a list of the rules that matched, in the view's order. A rule needs a label, because color is never the only signal: the web shows an icon and the label as text and names the group for the same words. A rule that reads a field the person cannot see is dropped for them, and the view still runs.

#### 6.1.3 A Million Rows in a Window (Phase 3)

A view with a sort on an indexed field opens at any row, with the same row scope and column rules as every other read: `GET /apps/{slug}/views/{id}/data?offset=&limit=`, `last=true`, and `count=exact`.

- **Jumps use anchors.** Skipping 500,000 rows with `OFFSET` over 1,000,000 took 3.0 s, and the best form without anchors took 0.4 s and grew with depth (`docs/plans/views.md` has the plans). A `build_anchors` job samples the sort key of every 1,000th row into `view_anchors`. A jump starts at the nearest anchor and skips fewer than 1,000 rows, in about 30 ms. Until the anchors exist a deep jump still answers, by skipping on the index, and it asks for them. A window may start at row 1,000,000 at most.
- **The order is a total order.** An ascending sort reads the key up with blanks last, then the record id. A descending sort is exactly that reversed. So the last page is the same query reversed, and the rows are flipped back.
- **Counts say when they gave up.** `count=exact` runs under the count timeout (two seconds). If it runs out, a count capped at 250,000 runs, and the answer is `{ "at_least": 250000 }`, which the screen shows as `Over 250,000 records`. If that also runs out, the answer is `{ "at_least": 0 }`.
- **A person with a row scope** gets windows and counts over their own scope only. They never use anchors, because their positions are not the table's.
- **The grid** (`VirtualGrid`, using `@tanstack/react-virtual`) draws the rows in view and a buffer, asks for windows of 200, shows skeleton rows until one arrives, and numbers its rows with `aria-rowcount` and `aria-rowindex`. Browsers stop honoring the height of a scroll area somewhere between 17 and 33 million pixels, and a million rows of 32 pixels is 32 million, so the scroll area is kept to 10 million pixels and the grid keeps its own offset in the list's pixels.

#### 6.1.4 The Measured Size (Row Scale Phase 6)

One table of **1,000,000 rows of thirty fields, five of them indexed,** was measured through the routes, twenty runs of each operation, release build, one PostgreSQL 16 server. CI runs the same test (`million_rows`, `cargo test --release -- --ignored million_rows`) and fails if the 19th of twenty, sorted from fastest to slowest (the 95th percentile), passes its limit or a read scans the records.

| Operation | Median | Slowest | Limit |
| --- | --- | --- | --- |
| First page, default order | 13.2 ms | 19.3 ms | 100 ms |
| A page by cursor near the end | 11.1 ms | 12.5 ms | 100 ms |
| Sort by an indexed number, a page near the end | 13.4 ms | 21.4 ms | 200 ms |
| Indexed range filter, 200 of 1,000 matches | 18.6 ms | 20.6 ms | 200 ms |
| `sum` over the whole table | 388.7 ms | 452.4 ms | 1,500 ms |
| `count` grouped by a five-value field | 1,007.7 ms | 1,115.7 ms | 1,500 ms |
| Search for a word in ten rows | 9.3 ms | 12.5 ms | 200 ms |
| Create one record, five indexed fields | 14.4 ms | 22.2 ms | 50 ms |
| Update one indexed field | 14.7 ms | 20.2 ms | 50 ms |

The next limit is one PostgreSQL server. The totals are the slowest rows because they read every value of a field, so they grow with the table: a total reads the index, never the records. Nothing was measured above a million rows.

#### 6.1.5 Summaries and Charts (Phase 4)

A summary is a view of `kind: summary` with `group_by` (one or two fields) and `aggregations` (`{ field, fn }`, `fn` one of `count`, `sum`, `avg`, `min`, `max`, `count_distinct`). Running it (`GET /apps/{slug}/views/{id}/data`) returns `summary: { columns, groups, has_more }`: a read-only table of at most 200 groups, ordered by key, with a flag when there are more. It releases totals and never records.

- **Each measure is one call to the aggregate route**, ordered by key so every column has the same groups, and the columns are put together by key. The totals and the groups are computed in PostgreSQL over the index. Nothing is counted in Rust.
- **The aggregate route** gains `bucket` (`day`, `week`, `month`, `quarter`, `year`) for a date `group_by`, `group_by2` for a second group, and `count_distinct` for groups. A bucket is cut in `platform.timezone`, so a record at 23:30 on 31 March in New York is in March, not April. A week opens on Monday and a bucket's key is the date it opens on.
- **Charts** are SVG written by hand in `apps/web/src/ui/charts/` with no library: bar (grouped or stacked), line and area, pie and donut (12 slices at most, the rest as `Other`), and a stat with a comparison. Each has axis labels and ticks, direct value labels where there is room, a legend as text, a pattern as well as a color for each series, and a `View as table` button that shows the exact data. Points are reachable by keyboard, and a live region says where the arrow keys landed (`March: 42 applications`). A stat says the difference in words (`Up 12 from 30 last term (40%)`).
- **A row of a summary** opens the grid on its group in an unsaved view whose filter is in the address (`?unsaved=`). Nothing is written to the server.
- **A chart over a field the person cannot see is not offered.** The schema the server sends leaves a hidden field out, and the aggregate route refuses a hidden field in a group, a measure or a filter.

#### 6.1.6 Attachments (Phase 1)

A file is kept outside the record. The record holds a list of `{ref_id, name}` in an `Attachment` field. The bytes sit on disk at `$SCAFFOLDRY_FILES_DIR/aa/bb/{sha256}`, so the same bytes uploaded twice are one blob and two refs (`files`, `file_refs`, migration `0051`).

- **Upload** (`POST /apps/{slug}/files`) needs write access. The body streams to a temporary name while it is hashed, is stopped at `files.max_mb` (default 50, at most 500), and moves to its hash name once the hash is known. The app's total is held to `files.app_quota_mb` (default 5000).
- **The type comes from the bytes.** What the client says is never read. Programs, scripts and pages are refused as `file_type_blocked`, by their first bytes or their name (`.exe`, `.html`, `.svg` and the like). Names are cleaned of paths and odd characters.
- **Download** (`GET /apps/{slug}/records/{id}/files/{ref_id}`) goes through the same record and field checks as a read. Any refusal is one 404. Only PNG, JPEG, GIF and WebP are shown inline. Everything else is `Content-Disposition: attachment`. Every response carries `X-Content-Type-Options: nosniff`, `Content-Security-Policy: default-src 'none'; sandbox` and `Cache-Control: private, no-store`.
- **A download from a sensitive field** writes a `file_download` row to the disclosure log.
- **An upload no record takes** is removed after 24 hours by the `purge_unattached_files` job. A server with `SCAFFOLDRY_FILES_DIR` set to a missing or read-only directory does not start. With it unset, uploads are off.

#### 6.1.7 Scanning (Attachments Phase 2)

With `files.scanner` set (`{ "kind": "clamd", "socket": "/run/clamav/clamd.ctl" }`), every upload starts `pending` and one `scan_file` job sends each waiting file to the daemon over its socket. The client is written by hand over `std` (`scan.rs`). A daemon that does not answer in 30 seconds gives `error`.

- **A pending file cannot be attached or downloaded.** `clean`, `error` and `skipped` files can. With no scanner a file is `skipped`, and the Overview says so in a Banner.
- **An infected file** is removed from disk, every upload of the same bytes is marked deleted, and a `FileInfected` ledger entry names the hash, the signature and the uploader.
- **`files.require_scan`** makes an upload fail with `scanner_unavailable` (503) when no scanner answers. It never turns a check off.
- **The Overview** shows whether a scanner is set, whether it answers, the last result, and how many files are in `error` or `skipped`.

#### 6.1.8 Attachments in the app (Phase 3)

A record stores `{ref_id, name, size, media_type}` for each file, so a grid chip shows a name and a size with no second request. `GET /apps/{slug}/records/{id}/files` adds the scan state and names the attachment fields the person may read. The record drawer's editor (`ui/Attachments.tsx`) has a drop target and a `Choose files` button that both work from the keyboard, a list with name, size, type and scan state as text, `Remove`, and a download. Uploads show text and a progress bar with `Cancel`, and an error shows the server's specific message through `explainError`. A field the person may not read leaves no label and no control.

#### 6.1.9 One source of truth (Stateless Server, Phase 1)

PostgreSQL is the source of truth, and a second copy of the server must be able to run beside the first. Today a copy still holds most of its collections in memory. The first phase adds the one way an instance may hold a collection, and starts with the settings.

- **A counter for each cached collection** (`cache_versions`, `cache.rs`). A writer raises it in the transaction that writes the collection. Every request begins with one statement that reads all the counters, and an instance reloads any collection whose counter it does not hold. `LISTEN` and `NOTIFY` are not used, because a missed notice would leave a cache stale and say nothing. The list of collections is closed. A collection not on it is read every time.
- **A request fails when the database cannot be read** (`503`, `storage_unavailable`). The next request tries again.
- **The server does not start with a read that fails.** There is no memory-only mode, and `repository` is always present. A test uses a real database of its own (`TestDb`).
- **Phases 2 and 3** turn people, sessions and tokens into queries, and put workspaces, guards, labels, datasets and the organization tree on the counters.

### 6.2 Long-Term Security Maintenance & Fleet Patching

In legacy low-code tools, departmental apps become unmaintained legacy liabilities. In Scaffoldry, apps are maintained as version-controlled repositories:

1. **Upstream Security Patches:** When Scaffoldry Core publishes security patches, dependency upgrades, or formula engine fixes, the system automatically opens patch proposals across the application fleet.
2. **Automated Regression Testing:** The system tests the departmental application against the patch in an isolated sandbox to verify that existing formulas, rollups, and views continue to calculate correctly.
3. **One-Click Approval:** Departmental administrators receive a prompt on The Desk: *"Security patch available for Admissions App. Regression tests: 100% Passed. [Apply Update]"*.

### 6.3 Institutional Deployment Modes

Scaffoldry supports two operating modes to accommodate both non-technical departments and enterprise IT teams:

- **Mode A: The Invisible Git Workflow (Default):**
  Faculty, staff, and departmental administrators interact entirely through The Desk. Embedded Git manages branches, approvals, and merges behind the scenes. Users experience the collaborative ease of low-code without developer tooling.
- **Mode B: Enterprise Git Sync (Central IT Integration) (Not Built):**
  Planned export to sync departmental application definitions with the university's existing GitHub Enterprise or GitLab infrastructure.
  - Central IT security tools (Dependabot, Snyk, CodeQL, Trivy) continuously scan application code and dependencies.
  - Central enterprise engineers can review and audit departmental applications through standard GitHub Pull Requests.

---

## 7. Guidelines for AI Agents Working in this Repository

Future AI assistants working on Scaffoldry must strictly follow these instructions:

1. **Never introduce hypervisor or VM nesting.** Do not suggest QEMU, nested libvirt, or host-level Podman quadlet workarounds. Everything runs in standard, portable containers.
2. **Never store long-lived credentials in scripts.** Never create credential-minting bash scripts or SSH scrapers. Use standard environment variables and scoped service tokens.
3. **Never bypass human approval for production changes.** Production deployments and schema modifications must always originate from an approved merge into `main`.
4. **Never write raw SQL migrations for user-defined fields.** User-created columns live in record JSONB; do not run dynamic `ALTER TABLE` DDL queries against PostgreSQL.
5. **Adhere to Write for People.** Write simple, clear prose. Avoid dense sentence structures, pseudo-philosophical aphorisms, and nested subordinate clauses.
6. **Preserve Organization Tree Invariants.** Units form a single-parent tree. Never give a unit two parents. Never use labels other than Platform Admin and Org Unit Admin. Never infer admin privileges from job titles.
7. **Maintain Business Process Human Gates.** Human decision steps (`UserTask`) must wait for real human decisions via ProcessDesk and append to `governance_ledger`. Never fire-and-forget or bypass human authority.
8. **Preserve Retrigger and Loop Guards.** Automation rules must respect retrigger suppression and the depth-3 backstop to prevent infinite execution cycles.
9. **Sovereign Engine & Cryptographic Ledger Integrity.** The calculation engine is native Rust (`scaffoldry-engine`), never Arrow/DataFusion. The audit ledger is an immutable SHA-256 blockchain stored in PostgreSQL, never embedded Git.

---

## 8. Planned Direction (Not Built)

This section describes a direction that is designed and not approved or built. The rest of this document describes code that exists. The notes linked here are the source. The section is in place so the document does not drift from the plan.

### 8.1 The Data Layer

Data becomes a layer of its own, behind one door. Apps, pages, automations, and AI agents are clients. A dataset is a governed table with an owning function and unit. Every read and write of a row goes through the data layer, which checks the labels on the data, the grants of the person, and the grants of the client.

The data layer is a crate boundary inside the modular monolith. It is not a second service and it adds no machine to the appliance. It sits across the access, store, and ledger layers. An audit fails any code outside it that reads the record or label tables. See `docs/plans/data-layer.md`.

### 8.2 Which Engine Answers Which Question

- **Cedar** answers questions about workspaces, apps, and actions: may this person open this app, approve this change, or export from this workspace.
- **The row scope** (`service/scope.rs`, in Rust, compiled to SQL) answers questions about rows and fields: which rows may this person read, and which columns. Audience fields and label requirements extend it. It is checked against Cedar's answers by an equivalence test.
- **The ledger** records every decision. Neither engine writes to it directly.

### 8.3 The Catalog

People find what data exists without seeing a real value. A catalog entry holds metadata and synthetic samples. Discoverability is a policy. A person can ask the steward for access from the entry, and the outcome is a dated grant.

### 8.4 MCP Guidance

The MCP server teaches the agent. At `initialize` it gives instructions built for the caller. Tool descriptions, result guidance, and error hints say what to do next. Resources and prompts, generated from policy data, carry the rules and the tasks. Text stored in the system is marked untrusted, and a change to access is proposed to a person, never applied by the agent. See `docs/plans/mcp-guidance.md`.
