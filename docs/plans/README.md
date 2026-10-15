# Gemini workplan

Thirty-four briefs and one master. One phase per session. Do not start the next phase until the current phase's tests pass. Paste the prompt at the bottom of the brief you are in.

[capability-roadmap.md](capability-roadmap.md) explains why the newer briefs exist and what they change in the older ones. Read it first.

Scaffoldry is MCP-first. People build apps directly from the AI tools they already use (a desktop AI app, Claude Code, Codex) through the MCP server, signed in as themselves. The web desk is a second client of the same service layer. Read "Decisions already made" in [foundation.md](foundation.md) before any session.

The foundation comes first. It adds almost no feature. Several routes do not check the caller today, and several writes never reach PostgreSQL. Feature work on top of that is wasted.

### Wave 0 — honest and safe

Nothing here adds a feature. It makes the server refuse what it cannot prove, store what it accepts, and say what it does.

| Order | Brief | Phase | What is true after it | Status |
| --- | --- | --- | --- | --- |
| 1 | [admin-console.md](admin-console.md) | 0 | The console opens. A missing API shows a banner, not silent sample data. Run after Gemini's current organization work is committed | **COMPLETED** |
| 2 | [foundation.md](foundation.md) | 1 | README and ARCHITECTURE describe the code that exists | **COMPLETED** |
| 3 | [foundation.md](foundation.md) | 2 | A failed ledger write fails the request. Sessions expire | **COMPLETED** |
| 4 | [foundation.md](foundation.md) | 3 | Migrations run once. The ledger rejects update and delete. Workspace changes are stored | **COMPLETED** |
| 5 | [foundation.md](foundation.md) | 4 | Eight database connections | **COMPLETED** |
| 6 | [foundation.md](foundation.md) | 5 | An app belongs to a workspace. App and record access comes from stored roles | **COMPLETED** |
| 7 | [foundation.md](foundation.md) | 6 | No route is left without an access check | **COMPLETED** |
| 8 | [guards.md](guards.md) | 1 | One Cedar schema and one typed entity builder. Every policy has a plain description. A denial names the policy | **COMPLETED** |
| 9 | [business-process.md](business-process.md) | open phases | Verify each phase's tests. The completion report in that brief does not match the code. Fix the report | **COMPLETED** |
| 10 | [foundation.md](foundation.md) | 7 | Tokens are hashed rows a user can mint and revoke. First boot prints a setup token | **COMPLETED** |
| 11 | [foundation.md](foundation.md) | 8 | Institution settings live in PostgreSQL and every change is in the ledger. `SCAFFOLDRY_ENV` is gone | **COMPLETED** |
| 12 | [foundation.md](foundation.md) | 9 | A user mints an agent token in the settings pane. A Platform Admin edits settings in `/admin` | **COMPLETED** |
| 13 | [ux-standards.md](ux-standards.md) | 1 | One component kit and a review checklist | **COMPLETED** |
| 14 | [jobs.md](jobs.md) | 1 | A durable job queue on PostgreSQL | **COMPLETED** |
| 15 | [jobs.md](jobs.md) | 2 | A scheduler, a Jobs panel, and job tools for agents | **COMPLETED** |

### Wave 1 — the governed core

People, units, positions, rules, proposals, and a data model with real links, history, and access rules. A department can run a governed process on it.

| Order | Brief | Phase | What is true after it | Status |
| --- | --- | --- | --- | --- |
| 16 | [organization.md](organization.md) | 1, 2, 5 | Code is in the working tree, not committed. Run its tests, fix what earlier phases changed, commit | **COMPLETED** |
| 17 | [organization.md](organization.md) | 3 | A Platform Admin opens `/admin` → Organization and manages the tree | **COMPLETED** |
| 18 | [organization.md](organization.md) | 4 | An Org Unit Admin sees only their units on the workspace rail | **COMPLETED** |
| 19 | [admin-console.md](admin-console.md) | 1 | A real console shell, one guard, an overview, and one ledger definition per decision type | **COMPLETED** |
| 20 | [admin-console.md](admin-console.md) | 2 | People: search, hold, revoke tokens, appointments, manual users | **COMPLETED** |
| 21 | [approvers.md](approvers.md) | 1 | Positions in the org tree, record authors, and a pure approver resolver with separation of duties | **COMPLETED** |
| 22 | [admin-console.md](admin-console.md) | 3 | Workspaces and apps inventory | **COMPLETED** |
| 23 | [approvers.md](approvers.md) | 2 | Position types, holders, and vacancies | **COMPLETED** |
| 24 | [approvers.md](approvers.md) | 3 | A step waits for a named position. The submitter cannot decide | **COMPLETED** |
| 25 | [guards.md](guards.md) | 2 | Workspace guards are evaluated on every decision. The old unevaluated field is removed | **COMPLETED** |
| 26 | [guards.md](guards.md) | 3 | A workspace owner adds a rule from a template and sees who it affects | **COMPLETED** |
| 27 | [live-data.md](live-data.md) | 1–2 | Records live in PostgreSQL only. A save carries a version and conflicts return 409 | **COMPLETED** |
| 28 | [row-scale.md](row-scale.md) | 1 | A record has a `table_id`. A list is one indexed page | **COMPLETED** |
| 29 | [mcp-apps.md](mcp-apps.md) | 1 | One tool registry with one gate. The fake tools are gone | **COMPLETED** |
| 30 | [agent-coverage.md](agent-coverage.md) | 1 | A scenario runner and twenty scripted faculty requests run against the real server | **COMPLETED** |
| 31 | [ux-standards.md](ux-standards.md) | 2 | Every error has a code and a next step | **COMPLETED** |
| 32 | [admin-console.md](admin-console.md) | 4 | First part only: `effective_ferpa_sensitive` with an empty label set, and every read of sensitivity goes through it | **COMPLETED** |
| 33 | [sensitive-content.md](sensitive-content.md) | 1 | Sensitivity categories are settings, with presets for FERPA, PHI, PCI, and PII. Detectors find SSNs and card numbers. A field can carry several categories | **COMPLETED** |
| 34 | [mcp-apps.md](mcp-apps.md) | 2 | An agent reads and writes records, a page at a time | **COMPLETED** |
| 35 | [record-history.md](record-history.md) | 1 | Every record change is recorded in the same transaction, with the actor | **COMPLETED** |
| 36 | [row-scale.md](row-scale.md) | 2 | Sort and filter on an indexed field use the pivot index. Multi-valued fields index one row per value | **COMPLETED** |
| 37 | [jobs.md](jobs.md) | 3 | Index building and flag recomputation run as jobs, with a visible `building` state | **COMPLETED** |
| 38 | [mcp-apps.md](mcp-apps.md) | 3 | The app manifest has a published schema, pages, and size limits | **COMPLETED** |
| 39 | [mcp-apps.md](mcp-apps.md) | 4 | An agent proposes an app change, with page source. A person approves it | **COMPLETED** |
| 40 | [links.md](links.md) | 1 | Records link by id. A link field and its index | **COMPLETED** |
| 41 | [links.md](links.md) | 2 | Two-way links, and what delete does to them | **COMPLETED** |
| 42 | [links.md](links.md) | 3 | Lookup, rollup, and count through links | **COMPLETED** |
| 43 | [row-scale.md](row-scale.md) | 3 | Computed values are written, not recomputed, using links | **COMPLETED** |
| 44 | [calc-graph.md](calc-graph.md) | 1 | One formula evaluator, in Rust, with a preview route | **COMPLETED** |
| 45 | [calc-graph.md](calc-graph.md) | 2 | A dependency graph with cycle detection and ordered recalculation | **COMPLETED** |
| 46 | [mcp-apps.md](mcp-apps.md) | 4a | Process definitions and every direct app change are proposals. A Proposals screen exists | **COMPLETED** |
| 47 | [mcp-apps.md](mcp-apps.md) | 5 | An agent lists and decides waiting process steps | **COMPLETED** |
| 48 | [record-history.md](record-history.md) | 2 | Undo and restore | **COMPLETED** |
| 49 | [record-history.md](record-history.md) | 3 | Comments, mentions, and the commenter role | **COMPLETED** |
| 50 | [lifecycle.md](lifecycle.md) | 1 | Delete moves a record to trash. Restore brings it back with its links | **COMPLETED** |
| 51 | [links.md](links.md) | 4 | A link picker that respects access | **COMPLETED** |
| 52 | [access-rules.md](access-rules.md) | 1 | Share with groups, units, and positions. One `member_role` function | **COMPLETED** |
| 53 | [access-rules.md](access-rules.md) | 2 | Rules as data, one row scope, and a SQL and Rust equivalence test | **COMPLETED** |
| 54 | [access-rules.md](access-rules.md) | 3 | Every path applies the scope. No filtering or sorting on a hidden column | **COMPLETED** |
| 55 | [access-rules.md](access-rules.md) | 4 | Column rules and denials with a reason and a remedy | **COMPLETED** |
| 56 | [access-rules.md](access-rules.md) | 5 | Rules authored by proposal, with a preview of who is affected | **COMPLETED** |
| 57 | [access-rules.md](access-rules.md) | 6 | View as another user, read-only | **COMPLETED** |

### Wave 1b — proof of the data model, one source of truth, and who decides

The first two rows measure a million-row table. They come before anything else is built on the JSON record and its index, because a failed limit changes the data model. If row-scale phase 6 fails, stop and report. That decision is Johann's, under "Open decisions".

Nothing in Wave 2 or later may add state to the server's memory. The six stateless rows remove what is there, so that two copies of the server can run at once. The rest make the rules behind a decision visible, and make who decides a setting an institution controls.

| Order | Brief | Phase | What is true after it | Status |
| --- | --- | --- | --- | --- |
| 58 | [row-scale.md](row-scale.md) | 4–5 | Totals and group counts in one statement. Full-text search | **COMPLETED** |
| 59 | [row-scale.md](row-scale.md) | 6 | A million-row table meets the stated limits, with measured times on record | **COMPLETED** |
| 60 | [stateless-server.md](stateless-server.md) | 1 | One cache mechanism, a boot that fails when a read fails, and a database for each test | **COMPLETED** |
| 61 | [stateless-server.md](stateless-server.md) | 2 | People, appointments, sessions, and tokens are queries. A revoked token stops on every instance at once | **COMPLETED** |
| 62 | [stateless-server.md](stateless-server.md) | 3 | Workspaces, guards, labels, datasets, and the org tree agree across instances | |
| 63 | [stateless-server.md](stateless-server.md) | 4 | An approved change is live on every instance on the next request | |
| 64 | [stateless-server.md](stateless-server.md) | 5 | Process instances and the ledger are never held in memory. Two instances cannot both decide | |
| 65 | [stateless-server.md](stateless-server.md) | 6 | Every write is proven across two instances. A readiness route and a clean stop | |
| 66 | [policy-library.md](policy-library.md) | 1 | Policies, their sources, and rules that cite a passage word for word | |
| 67 | [policy-library.md](policy-library.md) | 2 | A person's agent reads a policy and proposes rules and questions. It cannot publish | |
| 68 | [policy-library.md](policy-library.md) | 3 | A person who did not submit a rule reviews it against its passage and publishes it | |
| 69 | [authority.md](authority.md) | 1 | A change has a tier. A matrix says who decides. With no rows it behaves as it does today | |
| 70 | [authority.md](authority.md) | 2 | The matrix in the console. Its effect on real proposals is shown before a change, and a second person confirms | |
| 71 | [policy-library.md](policy-library.md) | 4 | Everyone reads the rules as sentences, linked to policy and source, marked Enforced or Guidance | |
| 72 | [authority.md](authority.md) | 3 | The builder is told who decides, why, and what would change it. An agent can ask first | |
| 73 | [authority.md](authority.md) | 4 | A simulated institution of 5,000 people, and the same measures from real decisions | |

### Wave 1c — make the desk real

Added after the click-through in [clickthrough-findings.md](clickthrough-findings.md). The server has the records, ledger, and rules. Two core screens do not use them. Until these rows are done, the product has no working loop for "open an app, add a record, see it saved." Rows are labelled `1c-n` so the numbers above and below do not move. Row 159 below moves here as 1c-1.

| Order | Brief | Phase | What is true after it | Status |
| --- | --- | --- | --- | --- |
| 1c-1 | [live-data.md](live-data.md) | 3 | The published-app screen reads and writes records through the API. It has no hardcoded record array. A new record is stored and has a ledger entry. Kanban moves are stored. The totals come from the stored rows, and "Approved Funding" counts only the statuses the app defines as approved | |
| 1c-2 | [live-data.md](live-data.md) | new, after 3 | The Datasets screen reads `/datasets`. It shows the real datasets and row counts | |
| 1c-3 | [ux-standards.md](ux-standards.md) | new, after 3 | No screen shows sample data without a visible "Sample data" label. A test fails on a hardcoded record array in a screen component | |
| 1c-4 | [position-roles.md](position-roles.md) | 1 | Platform admin comes from a held role. The setup identity ends when the first admin is appointed. A title on a record is not counted as an admin | **COMPLETED** |
| 1c-5 | [position-roles.md](position-roles.md) | 2a | A position, group, unit, or person can be granted one app, not only a whole workspace. Open: an app-only member cannot find the app in the desk | **COMPLETED** for the server. The desk question is open |
| 1c-6 | [position-roles.md](position-roles.md) | 3 | An admin lists every direct person grant, and which grants are app-scoped | **COMPLETED** for the API (3a). A required reason (3b) and a desk screen are not done |

Rows 1c-2 and 1c-3 need a new phase written in their brief. `live-data.md` phase 3 already removes the record arrays from `PublishedAppView.tsx`, so 1c-1 is written. Done already from the click-through: the false ledger message, made-up dashboard figures, hardcoded header user, and "Live" labels are gone from the published-app screen. The Vite dev proxy exists. The budget field accepts round values. A reload shows the person the stored token belongs to.

### Wave 2 — the data platform

Fields, files, views, import and export, and the million-row proof. A department can bring its spreadsheet and work in it.

| Order | Brief | Phase | What is true after it |
| --- | --- | --- | --- |
| 74 | [calc-graph.md](calc-graph.md) | 3 | Recalculation jobs when a definition changes, and type conversion with a dry run | **COMPLETED** |
| 75 | [calc-graph.md](calc-graph.md) | 4 | Volatile formulas, defaults, and constraints. Adds `platform.timezone` |
| 76 | [jobs.md](jobs.md) | 4 | Quotas, retention of finished jobs, and the Background work indicator | **COMPLETED** |
| 77 | [field-types.md](field-types.md) | 1 | Select options with ids and colors | **COMPLETED** |
| 78 | [record-history.md](record-history.md) | 4 | The disclosure log | **COMPLETED** |
| 79 | [field-types.md](field-types.md) | 2 | Collaborator, created by, and last modified by | **COMPLETED** |
| 80 | [field-types.md](field-types.md) | 3 | Long text with a safe renderer | **COMPLETED** |
| 81 | [field-types.md](field-types.md) | 4 | Duration and date-time | **COMPLETED** |
| 82 | [views.md](views.md) | 1 | Views live apart from the manifest. Personal, collaborative, and locked | **COMPLETED** |
| 83 | [views.md](views.md) | 2 | Filter trees, grouping, footer totals, and color rules | **COMPLETED** |
| 84 | [views.md](views.md) | 3 | A million rows in a window. Uses `@tanstack/react-virtual`, approved by Johann (MIT) | **COMPLETED** |
| 85 | [views.md](views.md) | 4 | Summary views and hand-written accessible charts | **COMPLETED** |
| 86 | [import-export.md](import-export.md) | 1 | An import wizard with a dry run, a rejected-rows report, and resumable jobs |
| 87 | [attachments.md](attachments.md) | 1 | A file store with upload, download, limits, and safe headers | **COMPLETED** |
| 88 | [attachments.md](attachments.md) | 2 | Scanning through an operator's antivirus daemon | **COMPLETED** |
| 89 | [attachments.md](attachments.md) | 3 | Attachment cells and chips in the app | **COMPLETED** |
| 90 | [import-export.md](import-export.md) | 2 | Spreadsheet import and export, with exports logged as disclosures |
| 91 | [import-export.md](import-export.md) | 3 | App bundles. Importing one makes a proposal |
| 92 | [import-export.md](import-export.md) | 4 | Approved templates. An unchanged template skips a review it already had |
| 93 | [import-export.md](import-export.md) | 5 | Snapshots and restore as a new app. A destructive approval takes one first |
| 94 | [import-export.md](import-export.md) | 6 | Moving off a spreadsheet tool, with every decision listed |
| 95 | [links.md](links.md) | 5 | Converting old value joins to links, with a preview |
| 96 | [links.md](links.md) | 6 | Link cells, paste, and a linked records section in the grid |
| 97 | [lifecycle.md](lifecycle.md) | 2 | Retired fields and archived apps |
| 98 | [lifecycle.md](lifecycle.md) | 3 | Retention policies, nightly purge, and a certificate of destruction |
| 99 | [attachments.md](attachments.md) | 4 | Orphan blob purge, retention, and bundles with files |
| 100 | [lifecycle.md](lifecycle.md) | 4 | Legal holds that override every destruction path |
| 101 | [record-history.md](record-history.md) | 5 | History, comments, and disclosures in the record drawer |

### Wave 3 — collaboration and process

People are told, see each other work, and run real committee processes.

| Order | Brief | Phase | What is true after it |
| --- | --- | --- | --- |
| 102 | [realtime.md](realtime.md) | 1 | Live change events over server-sent events, with no data in them |
| 103 | [realtime.md](realtime.md) | 2 | The grid updates in place and keeps what you are typing |
| 104 | [integrations.md](integrations.md) | 1 | An encrypted secrets store and an outbound allowlist |
| 105 | [policy-library.md](policy-library.md) | 5 | Web sources are fetched through the allowlist. A changed source sends its rules back for review | |
| 106 | [policy-library.md](policy-library.md) | 6 | A console panel and one screen for the state of the library | |
| 107 | [notifications.md](notifications.md) | 1 | An inbox and a bell |
| 108 | [notifications.md](notifications.md) | 2 | Email through an allowlisted SMTP host |
| 109 | [realtime.md](realtime.md) | 3 | Presence and a channel for the person's notices and jobs |
| 110 | [approvers.md](approvers.md) | 4 | Delegation by date |
| 111 | [approvers.md](approvers.md) | 5 | The workflow builder picks who decides and previews it |
| 112 | [notifications.md](notifications.md) | 3 | Preferences and digests |
| 113 | [notifications.md](notifications.md) | 4 | Every producer is wired. No notice carries a record value |
| 114 | [process-v2.md](process-v2.md) | 1 | Process events and analytics: where time goes |
| 115 | [workflows.md](workflows.md) | 1 | Flows run with approved grants and reach records by address. They can span apps |
| 116 | [workflows.md](workflows.md) | 2 | A flow touching several workspaces needs approval from each. Moving sensitive data is flagged |
| 117 | [process-v2.md](process-v2.md) | 2 | Branches, cross-table effects, manual and date triggers |
| 118 | [workflows.md](workflows.md) | 3 | Schedules, called flows, chaining, and a circuit breaker |
| 119 | [field-types.md](field-types.md) | 5 | A button field that starts a process |
| 120 | [process-v2.md](process-v2.md) | 3 | Timers and escalation. No timer ever decides |
| 121 | [process-v2.md](process-v2.md) | 4 | Committee votes |
| 122 | [workflows.md](workflows.md) | 4 | Fill-in tasks, with a task grant on one record while the step is open |
| 123 | [process-v2.md](process-v2.md) | 5 | Builder and desk screens for all of it |
| 124 | [workflows.md](workflows.md) | 5 | My Work: one list of decisions, tasks, and reviews across the institution, kept in step by jobs |
| 125 | [workflows.md](workflows.md) | 6 | The My Work page and a review queue |
| 126 | [workflows.md](workflows.md) | 7 | Flow editor: one command model, the outline, and server validation |
| 127 | [workflows.md](workflows.md) | 8 | Flow editor: the block-structured canvas |
| 128 | [workflows.md](workflows.md) | 9 | Test run with a trace on the canvas |
| 129 | [workflows.md](workflows.md) | 10 | Run history, safe retry, and version comparison |

### Wave 4 — ecosystem and reach

Tokens that can be narrower, webhooks, synced datasets, custom pages, people outside the institution, and the compliance layer.

| Order | Brief | Phase | What is true after it |
| --- | --- | --- | --- |
| 130 | [integrations.md](integrations.md) | 2 | Tokens limited to apps, tables, and operations |
| 131 | [agent-coverage.md](agent-coverage.md) | 2 | Protocol conformance, a tool-list budget, error hints, and a guide that runs as tests |
| 132 | [integrations.md](integrations.md) | 3 | An API description generated for each app |
| 133 | [integrations.md](integrations.md) | 4 | Webhooks behind an allowlist and a proposal, with signing, retries, and a call log |
| 134 | [integrations.md](integrations.md) | 5 | Outbound access, secrets, and webhook panels |
| 135 | [connections.md](connections.md) | 1 | Datasets are real and read-only. A file-drop sync |
| 136 | [connections.md](connections.md) | 2 | A changed source stops the sync and makes a proposal |
| 137 | [connections.md](connections.md) | 3 | PostgreSQL connector. Needs a TLS connector |
| 138 | [connections.md](connections.md) | 4 | HTTP connector |
| 139 | [connections.md](connections.md) | 5 | Apps link to datasets |
| 140 | [connections.md](connections.md) | 6 | The connections panel |
| 141 | [mcp-apps.md](mcp-apps.md) | 6 | A custom page is rendered into a sandbox document and bounded by its grants |
| 142 | [mcp-apps.md](mcp-apps.md) | 7 | The web desk shows a custom page in a sandboxed frame |
| 143 | [views.md](views.md) | 5 | Dashboards and linked widgets |
| 144 | [views.md](views.md) | 6 | Shared links, off until enabled, never including sensitive fields |
| 145 | [views.md](views.md) | 7 | Forms with conditions, boards, calendars, and galleries |
| 146 | [guests.md](guests.md) | 1 | A guest proves an email address with a one-time link |
| 147 | [guests.md](guests.md) | 2 | A form open to guests, with a built-in own-records rule |
| 148 | [guests.md](guests.md) | 3 | A plain status page for the guest |
| 149 | [guests.md](guests.md) | 4 | Operating guests: closing forms, erasing, abuse signals |
| 150 | [admin-console.md](admin-console.md) | 4 | The rest: labels, routes, dataset classification, and screens |
| 151 | [sensitive-content.md](sensitive-content.md) | 2 | Stored records are scanned as a job. A compliance officer reviews findings and designates authorized stores |
| 152 | [sensitive-content.md](sensitive-content.md) | 3 | New writes, imports, and plain-text files are checked. Each detector flags, warns, or blocks, as the organization chooses |
| 153 | [sensitive-content.md](sensitive-content.md) | 4 | History, exports, and snapshots honor the categories. The old flag name is gone |
| 154 | [oscal-catalog.md](oscal-catalog.md) | 1 | The NIST catalogs and baselines, slimmed and validated. Needs files from Johann |
| 155 | [admin-console.md](admin-console.md) | 5 | Policy and OSCAL: versioned Cedar policy with test cases, a real requirements export |
| 156 | [oscal-catalog.md](oscal-catalog.md) | 2 | Control titles, validation, and baseline coverage |
| 157 | [admin-console.md](admin-console.md) | 6 | Business processes: inventory, versions, instance monitor, reassign, cancel |
| 158 | [workflows.md](workflows.md) | 11 | Console panel for flows |
| 159 | [live-data.md](live-data.md) | 3 | Moved to Wave 1c as 1c-1. The desk reads records and the directory from the API |
| 160 | [foundation.md](foundation.md) | 10 | Tokens from the institution's identity provider are verified. Adds `jsonwebtoken` |
| 161 | [foundation.md](foundation.md) | 11 | One image serves the API and the web files |
| 162 | [admin-console.md](admin-console.md) | 7 | Audit explorer, token management, agent controls, and a read-only admin inventory tool |
| 163 | [ux-standards.md](ux-standards.md) | 3 | The desk uses the kit. No browser dialogs. No swallowed errors. CI enforces both |
| 164 | [mcp-apps.md](mcp-apps.md) | 8 | Custom pages and review screens render inside the agent. Needs the MCP Apps spec pasted in |
| 165 | [data-grid-parity.md](data-grid-parity.md) | open phases | Grid work on the web desk. Last, because the agent is the primary surface |

Foundation phases 10 and 11 depend on nothing after phase 9. Run them earlier if a pilot date appears.

Migrations, in filename order. Since foundation phase 3, each file runs once and is recorded in `schema_migrations`.

| File | Brief |
| --- | --- |
| `0004_organization_scope.sql` | organization phase 1 |
| `0005_process_instances.sql` | business process phase 4 |
| `0006_role_source.sql` | organization phase 5 |
| `0007_record_version.sql` | live data phase 2 |
| `0008_schema_migrations.sql` | foundation phase 3 |
| `0009_ledger_append_only.sql` | foundation phase 3 |
| `0010_app_workspace.sql` | foundation phase 5 |
| `0011_app_proposals.sql` | mcp apps phase 4. Also creates `app_page_sources` |
| `0012_api_tokens.sql` | foundation phase 7 |
| `0013_platform_settings.sql` | foundation phase 8 |
| `0014_record_table_id.sql` | row scale phase 1 |
| `0015_record_index.sql` | row scale phase 2 |
| `0016_record_search.sql` | row scale phase 5 |
| `0017_data_labels.sql` | admin console phase 4 |
| `0018_policy_versions.sql` | admin console phase 5. Also creates `policy_cases` and `governance_documents` |
| `0019_rule_versions.sql` | mcp apps phase 4a |
| `0020_ledger_filters.sql` | admin console phase 7 |
| `0021_positions.sql` | approvers phase 1 |
| `0022_record_created_by.sql` | approvers phase 1 |
| `0023_workspace_guards.sql` | guards phase 2. Archives and drops `workspaces.cedar_policy_guard` |
| `0024_proposal_rules.sql` | mcp apps phase 4a |
| `0025_delegations.sql` | approvers phase 4 |
| `0026_jobs.sql` | jobs phase 1 |
| `0027_record_history.sql` | record history phase 1 |
| `0028_record_comments.sql` | record history phase 3 |
| `0029_disclosure_log.sql` | record history phase 4 |
| `0030_member_principals.sql` | access rules phase 1 |
| `0031_record_links.sql` | links phase 1 |
| `0032_trash_links.sql` | lifecycle phase 1 |
| `0033_app_views.sql` | views phase 1 |
| `0034_view_shares.sql` | views phase 6. Also creates `rate_limits` |
| `0035_import_export.sql` | import export phase 4 (`app_templates`) |
| `0036_connections.sql` | connections phase 1 |
| `0037_secrets.sql` | integrations phase 1 |
| `0038_webhooks.sql` | integrations phase 4 |
| `0039_token_scope.sql` | integrations phase 2 |
| `0040_process_v2.sql` | process v2 phase 1 |
| `0041_notifications.sql` | notifications phase 1 |
| `0042_lifecycle.sql` | lifecycle phase 1 |
| `0043_guests.sql` | guests phase 1 |
| `0044_job_schedules.sql` | jobs phase 2 |
| `0045_index_status.sql` | jobs phase 3 |
| `0046_calc_status.sql` | calc graph phase 3 |
| `0047_view_anchors.sql` | views phase 3, only if the measured `OFFSET` fails its limit |
| `0048_email_outbox.sql` | notifications phase 2 |
| `0049_notification_prefs.sql` | notifications phase 3 |
| `0050_retention.sql` | lifecycle phase 3 |
| `0051_legal_holds.sql` | lifecycle phase 4 |
| `0052_snapshots.sql` | import export phase 5 |
| `0053_process_timers.sql` | process v2 phase 3 |
| `0054_process_votes.sql` | process v2 phase 4 |
| `0055_flows.sql` | workflows phase 1. Also adds `proposal_approvals` and `flow_effects` |
| `0056_work_items.sql` | workflows phase 5 |
| `0057_task_grants.sql` | workflows phase 4 |
| `0058_flow_state.sql` | workflows phase 3 |
| `0059_content_findings.sql` | sensitive content phase 2. Also creates `authorized_stores` and adds `data_labels.categories` |
| `0060_attachment_content_scan.sql` | sensitive content phase 3 |
| `0061_attachments.sql` | attachments phase 1. Shipped as `0051_files.sql`, with `0052_file_scans.sql` for phase 2. Migrations up to `0052` are taken on disk, so a number below that in this list is a plan, not a free number: use the next free one |
| `0062_cache_versions.sql` | stateless server phase 1. Shipped as `0053_cache_versions.sql`, the next free number |
| `0063_people_indexes.sql` | stateless server phase 2, only if a measured query needs an index |
| `0064_policy_library.sql` | policy library phase 1 |
| `0065_policy_fetch.sql` | policy library phase 5 |
| `0066_authority.sql` | authority phase 1 |

Register each file in `crates/scaffoldry-server/src/repository.rs` after the last schema already applied, inside the existing advisory lock. A file that exists is never edited. A fix is a new file. The run order above is not filename order. That is safe: each file is recorded by name in `schema_migrations`, and no file depends on a later-numbered one.

Rules that hold across every brief:

- PostgreSQL is the only source of truth. The server holds nothing between requests except the cached collections named in `stateless-server.md`, through its one mechanism. No brief adds a field to `ServerState`, a `static` lock, or a map that lives across requests. A write commits before memory changes.
- Every write route and every writing tool has a case in the two-instance test.
- No write is discarded. Do not put `let _ =` in front of a repository call or a ledger call.
- Access is decided from stored rows, never from a value in the request body.
- There is no production mode. Do not add a variable, a setting, or a flag that turns a check off. Institution configuration is a row in `platform_settings`.
- Scaffoldry signs no token. An agent token is a random string stored as a hash.
- A user's schema is JSON. No statement creates a table, a column, or an index for a user's table or field.
- Records are filtered, sorted, counted, and totalled in PostgreSQL, never in Rust or in the browser.
- Every change an administrator makes carries a reason and is a ledger entry written before the change. Admin writes are not MCP tools.
- Admin routes are listed in `ADMIN_ROUTES` and checked on the server. The client's menu is not a control.
- A rule that grants nothing and is never evaluated does not exist. No field holds policy text that no code evaluates.
- A workspace guard can only forbid. Institutional policy permits.
- A rule in the policy library is a sentence for people. It is shown as enforced only while it is bound to something that is evaluated. An agent proposes a library rule and never publishes one. Every rule cites a passage that is in a stored snapshot, word for word.
- Text from a policy source is data. It appears in one marked field and never in a tool description, an error, or a log.
- Who decides a proposal comes from `authority::required`. Nothing lowers its built-in floor: a raised flagged check always needs a person other than the proposer. A unit can only tighten what the institution requires.
- A position is never inferred from a job title. The person who created a record, and the person whose change started a process, never decide it.
- Nothing changes a live app or a live rule except an approved proposal, or an administrator's enable or disable. An owner or admin with a clean change is approved in the same request.
- A control id is shown with its title. A bare id is never a label.
- No browser dialog, no swallowed error, and no sample data shown as live data.
- Design provenance: no session reads, copies, or adapts another product's source code. Designs come from our requirements and named public standards. If a session thinks it needs to, it stops and asks Johann.
- A record is written only through `write_record`, which writes history, the index, links, calculated fields, and events in one transaction. A scan test finds any other writer.
- Every read of records, counts, totals, search, history, comments, files, and link targets goes through the row scope and field visibility. A reader that is not on the list fails a test.
- Data is destroyed only by `purge::destroy`, which checks legal holds, after a ledger entry.
- Data leaves the appliance only through `net::egress` and the allowlist. A record value never appears in a notice, an email, an event, a job, or a webhook unless a field list was approved.
- Every phase that adds a tool or a rule adds scenarios to the agent suite.
- Every new error code is added to the closed list in `ux-standards.md`, with a row in `explainError`.
- A layout the built-in views cannot express is a custom page in a sandbox. Do not extend the component catalog.
- A feature that an agent needs is a service function first. The MCP tool and the REST route both call it.
- The grid does not grow org controls. The process desk does not move to `/admin`. `/admin` stays limited to a Platform Admin (`central_admin`, or a `platform_admin` appointment on the root). The other admin role is Org Unit Admin (`unit_admin` on one unit). Do not label either role Super Admin, System Admin, or Org Admin.

Settled. These were open and are now decided. Each is written into the brief named. Several were decided in this plan and Johann may change them.

| Decision | Where |
| --- | --- |
| A process step names a position in the org tree, with delegation | `approvers.md` |
| A workspace's access rules are evaluated, forbid-only, and shown with their impact before saving. The unevaluated text field is removed | `guards.md` |
| OSCAL controls come from the NIST catalogs, with titles, validation, and baseline coverage | `oscal-catalog.md` |
| Every process definition and every direct app change is a proposal | `mcp-apps.md` phase 4a |
| Policy activation always needs a second person. The setup identity counts | `admin-console.md` phase 5 |
| Screens are built from one kit, to WCAG 2.2 AA, with no browser dialogs | `ux-standards.md` |
| Records link by id, two-way. The by-value join is retired | `links.md` |
| Views are presentation and live apart from the governed manifest | `views.md` |
| One formula evaluator, in Rust | `calc-graph.md` |
| Jobs, notifications, timers, and webhooks are built, on a PostgreSQL queue, with an allowlist for anything that leaves | `jobs.md`, `notifications.md`, `process-v2.md`, `integrations.md` |
| Real-time updates are server-sent events with ids only | `realtime.md` |
| Delete is trash. Destruction is retention, with proof. Legal holds override all of it | `lifecycle.md` |
| Flows run as a service identity with approved grants, reach records by address, never delete, and may span workspaces when each touched workspace's owner approves | `workflows.md` |
| A person has one list of decisions, tasks, and reviews across the institution, and an assignee can act only on one record and the listed fields while a step is open | `workflows.md` |
| People build from the AI tools they already use. There is no separate agent to deploy | `README.md`, `foundation.md` |
| Row and column rules narrow access and are evaluated by one scope, tested equal in Rust and SQL | `access-rules.md` |
| Shared links are off until enabled, have no passwords, and never carry sensitive fields | `views.md` phase 6 |
| People outside the institution use a guest form with a built-in own-records rule, opened only by a two-person proposal | `guests.md` |
| A page's JSX is compiled in the sandbox. A custom page is approved by a workspace owner or admin, with a second person when it reads a sensitive field | `mcp-apps.md` |
| A proposer may approve their own change when no check is flagged and they own or administer the workspace | `mcp-apps.md` phase 4 |
| PostgreSQL is the only source of truth. Any number of server instances may serve one database. Caches are refreshed by a counter read on every request, not by `LISTEN` and `NOTIFY` | `stateless-server.md` |
| A test has a database of its own. No test changes the process environment | `stateless-server.md` phase 1 |
| CI runs its checks as parallel jobs behind one required check, `validate`. Three steps wait for planned work and switch on by themselves: parallel tests when `TestDb` exists, OSCAL validation when code reads `OSCAL_OUT` or `OSCAL_CATALOG_OUT`, and the `million_rows` test when it exists. Agents do not edit `ci.yml` | `.github/workflows/ci.yml` |
| Institutional rules are kept as an OSCAL catalog of sentences, each citing a stored passage. A person's own agent proposes them. A different person publishes them. Scaffoldry runs no model | `policy-library.md` |
| The platform fetches web sources itself, through the outbound allowlist. It does not extract text from a PDF | `policy-library.md` phase 5 |
| A change has a FIPS 199 tier from the checks it raises. A matrix says who decides. The institution sets it, units only tighten it, and a second person confirms every change to it | `authority.md` |
| A proposal with no resolvable approver waits and is shown as Unassigned. It never falls back to a looser path | `authority.md` |

Open decisions. These are Johann's. Do not settle them in a session.

| Decision | Until decided |
| --- | --- |
| Browser redirect to the identity provider, and which provider the first pilot uses | The desk signs in with a pasted token |
| What the Cloud Run demo becomes once one image needs PostgreSQL | `deploy.yml` and `Dockerfile.web` are not touched |
| Git export of app definitions and of the ledger head hash | Not built |
| The crates and packages in the master brief: `ureq` with `rustls`, `aes-gcm`, `lettre`, `calamine`, `rust_xlsxwriter`, `flate2`, `@tanstack/react-virtual`, and a PostgreSQL TLS connector | Each phase that needs one stops until Johann says yes and the license audit passes |
| Adding `axe-core` (MPL-2.0) as a dev-only test dependency | Not added. Accessibility is tested with role, name, and keyboard tests |
| Running ClamAV as a separate daemon for upload scanning | Files show as not scanned. An operator can require a scanner |
| Who supplies the NIST catalog files, and the 800-171 revision CMMC requires | `oscal-catalog.md` stops until `governance/catalog/source/` and `PROVENANCE.json` exist |
| Where the master key file for the secrets store is kept, and who holds it | Secrets routes answer `master_key_missing` |
| If `row-scale.md` phase 6 fails its limits, whether to move to typed per-table storage. That changes architecture rule 4 | The JSON record with a pivot index stays |
| The retention numbers and the records officer who owns them | Placeholders, and a banner until real ones are set |
| Whether Scaffoldry is also offered as a hosted service, and whether that is one stack for each institution | Self-hosting stays supported. Nothing assumes more than one institution in a database. `stateless-server.md` removes the one thing that blocks a second instance |
| A full export of one institution in documented formats, and whether process definitions are also exported as BPMN XML | Exports are per app and capped. Not built |
| Who holds the positions the first matrix names, such as a privacy officer | The matrix has no rows and behaves as the platform does today |
