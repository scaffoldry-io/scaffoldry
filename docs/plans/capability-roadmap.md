# Capability roadmap — master brief

This file is not a work item for Gemini. It explains why seventeen new briefs exist, what they reverse, and what they need. Read it before any of them.

Scaffoldry's plans so far are strong on governance: the ledger, proposals, positions, guards, and the compliance export. They are thin on the daily mechanics of a database tool that thousands of faculty will use through an agent: links between records, history and undo, background work, views, import, attachments, and notices. A governed platform that is awkward to use will be bypassed, and a bypassed platform governs nothing.

This brief lists those gaps, the choices made about them, and the earlier decisions they change.

## Design provenance

Scaffoldry's designs come from its own requirements, from named public standards, and from general engineering knowledge. No session reads, copies, or adapts source code from another product in this repository. If a session believes it needs to, it stops and asks Johann. This is a rule about our process. It is not a statement about any product.

Third-party libraries are different. They are dependencies, they follow the license whitelist in `AGENTS.md`, and `governance/scripts/audit-licenses.py` checks them.

## The gaps

| Capability | State in the plan before this brief | Brief |
| --- | --- | --- |
| A durable queue for long work | None. Index rebuilds and recalculation run inside a request | `jobs.md` |
| History of every record change, undo, restore | The ledger records decisions. Records have no history | `record-history.md` |
| Comments and mentions on a record | None | `record-history.md` |
| A record of disclosures of sensitive data | None | `record-history.md` |
| Live updates when someone else edits | A version conflict at save time only | `realtime.md` |
| Row and column rules, sharing with groups, a commenter role, view as another user | One sensitivity flag and workspace guards | `access-rules.md` |
| Formula ordering, cycle detection, scheduled recalculation, constraints, type conversion | Formulas are computed per write with no declared order | `calc-graph.md` |
| Links between records by identity | Tables are joined by matching field values | `links.md` |
| More field types: stable select options, collaborator, long text, duration, button | Select options are strings. No collaborator or long text | `field-types.md` |
| File attachments | None | `attachments.md` |
| Personal views, charts, summaries, dashboards, shared links | All views are governed and few | `views.md` |
| Import wizard, exports, app bundles, templates, snapshots | CSV helpers only | `import-export.md` |
| Governed sync from student and HR systems | Datasets hold sample rows | `connections.md` |
| Webhooks, a secrets store, outbound control, scoped tokens, an API description per app | Webhooks are disabled. Agent tokens carry the user's full rights | `integrations.md` |
| An inbox and email | A log line on a process instance | `notifications.md` |
| Timers, escalation, branches, committee votes, process analytics | Linear steps with one waiting step | `process-v2.md` |
| Trash, retention, legal hold | `delete_record` removes the row | `lifecycle.md` |
| Applicants and other people with no account | Every user is in the directory | `guests.md` |
| Proof that the MCP surface can do what faculty ask | Tests of individual tools | `agent-coverage.md` |

## Choices

| Choice | Why |
| --- | --- |
| A user's table stays one JSONB record plus a derived index. It is not a real database table per user table | An approved proposal must change an app without running DDL against a production database. The manifest stays the contract. `row-scale.md` phase 6 measures it. If it fails its limits, typed per-table storage is the fallback. That would change architecture rule 4 and needs Johann's decision |
| Formulas are a small expression language evaluated only in Rust | A reviewer can read it. It has no I/O and cannot loop. One evaluator cannot disagree with itself |
| No model runs inside the product | The institution's agent drives MCP. `agent-coverage.md` makes that agent effective |
| Sync from other systems is scheduled, read-only, and governed. It is not a live proxy | A sync has a history, a drift check, and a sensitivity label. A live proxy passes every schema change and slow query through |
| Shared view links are off until a Platform Admin turns on a scope, carry no passwords, and never include a sensitive field | A password is an authentication surface to defend. The sensitive-field rule is a FERPA safeguard |
| Webhooks leave only through an allowlist, and creating one is a proposal. A payload carries ids unless values are approved | Data leaving the appliance is a disclosure |
| The queue is a PostgreSQL table with `FOR UPDATE SKIP LOCKED` | One database to back up and patch. No second service |
| Custom pages declare field-level grants a reviewer reads as a sentence | Designed in `mcp-apps.md` |

## Earlier decisions these briefs change

Each change is written into the affected brief. None changes code, because none of it is built.

| Earlier decision | Now | Reason |
| --- | --- | --- |
| Tables are joined by matching field values. No `record_links` table | Records link by record id, two-way, with an index | A renamed or duplicated key breaks a value join without any error. A link by id cannot |
| Views are part of the governed manifest | Views are presentation, stored apart. A personal view needs no approval. A view never widens access | Today, adding a sort to your own board would be a proposal |
| Formulas run in Rust and TypeScript, kept equal by test vectors | Rust only. The browser asks the server for a preview | Two evaluators drift |
| `WebhookDispatch` cannot succeed. No HTTP client | Webhooks work, behind the allowlist and a proposal | Institutions need them. The risk is where data goes, and the allowlist answers it |
| No timers, notifications, or email | All three, built on the queue | An approval nobody is told about is not a process |
| A multi-valued field cannot be indexed | One index row per value | A rule on a multi-person field needs it |
| No virtualization before 2,000 rows. No `OFFSET` | Windowed scrolling, with measured `OFFSET` for scrollbar jumps | A scrollbar cannot jump with cursor paging alone |
| Select options are strings | Options have ids, labels, and colors | Renaming an option today orphans stored values |
| Approval rebuilds indexes inside the request | A job, with a visible `building` state | Seconds at a million rows become minutes at ten |
| `delete_record` removes the row | It moves the row to trash. Purge is a retention job | Retention and holds need something to hold |

## Dependencies that need Johann's yes

Each crate has its own permissive license. Gemini runs `governance/scripts/audit-licenses.py` on every added crate and stops if a transitive dependency falls outside the whitelist. `rustls` brings certificate and crypto crates whose licenses must be checked. Prefer the system certificate store to a bundled root list.

| Crate or package | Used by | Why |
| --- | --- | --- |
| `ureq` (MIT or Apache-2.0), with `rustls` | `integrations.md`, `connections.md` | A small blocking HTTP client for worker threads |
| `aes-gcm` (MIT or Apache-2.0) | `integrations.md` | Encrypting stored secrets |
| `lettre` (MIT) | `notifications.md` | Sending mail over SMTP |
| `calamine` (MIT) | `import-export.md` | Reading spreadsheets |
| `rust_xlsxwriter` (MIT or Apache-2.0) | `import-export.md` | Writing spreadsheets |
| `flate2` (MIT or Apache-2.0) | `import-export.md` | Compressed bundles and snapshots |
| `@tanstack/react-virtual` (MIT) | `views.md` | Windowed scrolling past 2,000 rows |
| A PostgreSQL TLS connector | `connections.md` | Talking to an external database over TLS |
| ClamAV, a separate daemon that is not linked | `attachments.md` | Virus scanning. It is GPL, so it runs as its own process and Scaffoldry only speaks its socket protocol |

## Risks

- **Scope.** This is a large body of work. The run order in the README groups it into waves, and each wave ends in something usable.
- **Order.** Several briefs reshape earlier ones. Each states what it needs and what it replaces. If one is moved, check its "Needs" line.
- **Test weight.** Every brief carries tests. `agent-coverage.md` runs scripted requests against the whole server. Every brief adds scenarios to it.
- **No code is written.** All of this is plan.

## Enterprise objections

| Objection | Answer |
| --- | --- |
| This is too much to build. | Yes. The waves in the README let the first ones ship alone. Governance and the data model come before views and integrations. |
| The new briefs will drift from the old ones. | Each states what it replaces. The README holds one order. The scenario suite runs them together. |
| The JSON record model will not hold up. | It has a measured gate in `row-scale.md` phase 6 and a named fallback. |
| Adding file storage, email, and outbound calls widens the attack surface. | Each has its own brief with limits, an allowlist or a scanner, a setting that only restricts, and tests for abuse. None is on by default. |
| A fifth place to configure things. | Every setting is a `platform_settings` key, every panel is in the console, and every change is a ledger entry. |
