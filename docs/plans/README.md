# Gemini workplan

Thirty-eight briefs and one master. One phase per session. Do not start the next phase until the current phase's tests pass. Paste the prompt at the bottom of the brief you are in.

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

### Wave 1b — proof of the data model, one source of truth, and one access model

The first two rows measure a million-row table. They come before anything else is built on the JSON record and its index, because a failed limit changes the data model. If row-scale phase 6 fails, stop and report. That decision is Johann's, under "Open decisions".

Nothing in Wave 2 or later may add state to the server's memory. The six stateless rows remove what is there, so that two copies of the server can run at once. The first access-model row closes access paths that fail open, and goes first. Access-model phases 2 to 6 give every object coordinates and every grant one shape, and prove it at a million rows, before more features are built on membership-only access.

| Order | Brief | Phase | What is true after it | Status |
| --- | --- | --- | --- | --- |
| 58 | [row-scale.md](row-scale.md) | 4–5 | Totals and group counts in one statement. Full-text search | **COMPLETED** |
| 59 | [row-scale.md](row-scale.md) | 6 | A million-row table meets the stated limits, with measured times on record | **COMPLETED** |
| 60 | [stateless-server.md](stateless-server.md) | 1 | One cache mechanism, a boot that fails when a read fails, and a database for each test | **COMPLETED** |
| 61 | [stateless-server.md](stateless-server.md) | 2 | People, appointments, sessions, and tokens are queries. A revoked token stops on every instance at once | **COMPLETED** |
| 62 | [access-model.md](access-model.md) | 1 | Rules survive a restart, every check goes through one decision, a forbid always wins, and SCIM grants no admin role | **COMPLETED** |
| 63 | [stateless-server.md](stateless-server.md) | 3 | Workspaces, guards, labels, datasets, and the org tree agree across instances | **COMPLETED** |
| 64 | [stateless-server.md](stateless-server.md) | 4 | An approved change is live on every instance on the next request | **COMPLETED** |
| 65 | [stateless-server.md](stateless-server.md) | 5 | Process instances and the ledger are never held in memory. Two instances cannot both decide | **COMPLETED** |
| 66 | [stateless-server.md](stateless-server.md) | 6 | Every write is proven across two instances. A readiness route and a clean stop | **COMPLETED** |
| 67 | [access-model.md](access-model.md) | 2 | Hierarchies are data. One closure table replaces four tree walks. Cedar and policies use the unit tree, not a department string | **COMPLETED** |
| 68 | [access-model.md](access-model.md) | 3 | Every workspace, app, table and field has a unit, a domain and a sensitivity, and a child can only narrow them | **COMPLETED** |
| 69 | [access-model.md](access-model.md) | 4 | One grant shape and one function, role_at. A position grant on the holder's own unit serves every department | **COMPLETED** |
| 70 | [access-model.md](access-model.md) | 5 | A synthetic institution from a seed, and a reference evaluator that the real one is tested against | **COMPLETED** |
| 71 | [access-model.md](access-model.md) | 6 | Row and field access by reach at a million rows, with three hierarchies, against stated limits | **COMPLETED** |

### Wave 1c — make the desk real

Added after the click-through in [clickthrough-findings.md](clickthrough-findings.md). The server has the records, ledger, and rules. Two core screens do not use them. Until these rows are done, the product has no working loop for "open an app, add a record, see it saved." Rows are labelled `1c-n` so the numbers above and below do not move. Row 159 below moves here as 1c-1.

| Order | Brief | Phase | What is true after it | Status |
| --- | --- | --- | --- | --- |
| 1c-1 | [live-data.md](live-data.md) | 3 | The published-app screen reads and writes records through the API. It has no hardcoded record array. A new record is stored and has a ledger entry. Kanban moves are stored. The totals come from the stored rows, and "Approved Funding" counts only the statuses the app defines as approved | **COMPLETED** for the published-app screen and the workspace grid. Not clicked through in a browser. Kanban, calendar, gallery, delete, and link fields are untested against real rows |
| 1c-2 | [live-data.md](live-data.md) | new, after 3 | The Datasets screen reads `/datasets`. It shows the real datasets and row counts | **COMPLETED**. The app studio's dataset picker reads the server too |
| 1c-3 | [ux-standards.md](ux-standards.md) | new, after 3 | No screen shows sample data without a visible "Sample data" label. A test fails on a hardcoded record array in a screen component | **PARTLY DONE**. `no-built-in-sample-data.test.ts` fails on a `SEEDED_` constant or a state started from a literal array, in every screen but `AdminDesk.tsx`. The desk's `INITIAL_*` lists in `AdminDesk.tsx` remain, as the labelled fallback |
| 1c-4 | [position-roles.md](position-roles.md) | 1 | Platform admin comes from a held role. The setup identity ends when the first admin is appointed. A title on a record is not counted as an admin | **COMPLETED** |
| 1c-5 | [position-roles.md](position-roles.md) | 2a | A position, group, unit, or person can be granted one app, not only a whole workspace. An app-only member finds the app in My apps, and can pin it | **COMPLETED**. Pins are per browser |
| 1c-6 | [position-roles.md](position-roles.md) | 3 | An admin lists every direct person grant, and which grants are app-scoped. A reason on a direct grant is optional | **COMPLETED** for the API. A desk screen and a desk reason field are not done |

Proposed order for the next work: [workplan-v2.md](workplan-v2.md). It joins the design notes to the briefs in this file. The design notes are [data-layer.md](data-layer.md), [mcp-guidance.md](mcp-guidance.md), and [position-roles.md](position-roles.md). Rows labelled `1f-n` below come from them. They are **proposed and not approved**. The build agent reads only rows with a number in the first column, so it does not see them. Do not give them numbers until Johann approves the order.

Rows 1c-2 and 1c-3 need a new phase written in their brief. `live-data.md` phase 3 already removes the record arrays from `PublishedAppView.tsx`, so 1c-1 is written. Found by the UI audit (`docs/ui-review.md`), not yet fixed:

| Row | What is wrong | What is true after the fix |
| --- | --- | --- |
| 1c-7 | The desk's app and workspace lists are built into `AdminDesk.tsx`. It asks the server about apps the server does not have, such as `cs-ai-benchmarks`, and gets 404 on every load | The desk lists only what the server returns |
| 1c-8 | "New App", "New App Studio", and "Co-Build New App" each create an app only in the browser's memory, with a made-up slug and domain, and say "Created new app". The toast covers the studio panel | A new app is created on the server, or the button says it is a draft that is not saved. One toast area that does not cover a panel |
| 1c-9 | Unnamed fields in Security and Settings, an unnamed button in the opened navigation, on phone a banner mostly outside the window on `/admin`, and clipped text with no tooltip | Every control has a name. Nothing sits outside the window. Clipped text has a tooltip |

Rows 1c-7 to 1c-9 are done, and checked with `npm run ui:all`: the MCP check passes, all nine UI smoke checks pass, and the audit is down from 29 findings to 8.

- **1c-7.** The desk takes its apps from `GET /me/apps` and loads each definition from `GET /apps/{slug}`. It no longer asks the server about built-in sample apps, and it does not apply its own access check to apps the server listed. If the server's list does not arrive, it falls back to the built-in sample apps and says so in a banner. The Decisions panel waits until the list has settled. The built-in apps still show for a moment while the list loads. Opening a built-in app, and "use this dataset in a new app", still create local copies only.
- **1c-8.** "New App", "New App Studio", and "Co-Build New App" call `POST /workspaces/{id}/apps`. A 201 shows "Created new app". A 202 shows "Submitted for approval. The app is not live until someone approves it." A refusal says the app was not created. The made-up domain is gone. I checked this against a real server: the app was created, appeared in My apps, and took a record.
- **1c-9.** The four workspace settings fields, and the collapsed "New App Studio" button, have names. Ledger hashes have a full-hash tooltip. The phone banner was a false alarm in the audit and the audit rule is fixed.

Still showing in the audit, all minor: the header logo does nothing when clicked, and a few tabs and filters change nothing because they are already selected. Those are false alarms, or close to it.

Done already from the click-through: the false ledger message, made-up dashboard figures, hardcoded header user, and "Live" labels are gone from the published-app screen. The Vite dev proxy exists. The budget field accepts round values. A reload shows the person the stored token belongs to.

### Wave 1d — custom interfaces and apps built anywhere

Faculty want quick, custom web apps with interfaces of their own. These rows let a custom page carry the libraries it needs, let a reviewer open a proposed app before approving it, set up an AI tool in one step, and let an app hosted anywhere use institutional data as the signed-in person, within limits a person approved.

| Order | Brief | Phase | What is true after it | Status |
| --- | --- | --- | --- | --- |
| 72 | [mcp-apps.md](mcp-apps.md) | 6 | A custom page renders in a sandbox, bounded by its grants. A page can be a bundle with its own libraries, and keeps per-person settings | **COMPLETED** |
| 73 | [mcp-apps.md](mcp-apps.md) | 7 | The web desk shows a custom page in a sandboxed frame | **COMPLETED** |
| 74 | [proposal-preview.md](proposal-preview.md) | 1 | An approval names the digest of what was reviewed, and only that content can go live | **COMPLETED** |
| 75 | [proposal-preview.md](proposal-preview.md) | 2 | The proposer and the people who may decide open the proposed app, read-only, under the stricter of the live and proposed rules | **COMPLETED** |
| 76 | [integrations.md](integrations.md) | 2 | Tokens limited to apps, tables, and operations | **COMPLETED** |
| 77 | [agent-coverage.md](agent-coverage.md) | 2 | Protocol conformance, a tool-list budget, error hints, and a guide that runs as tests. The `openapi` resource waits for row 78 | **COMPLETED** |
| 78 | [integrations.md](integrations.md) | 3 | An API description generated for each app | **COMPLETED** |
| 79 | [agent-coverage.md](agent-coverage.md) | 4 | An AI tool is set up in one step: an AGENTS.md section, a skill, and docs that match the server's version | **COMPLETED** |
| 80 | [external-apps.md](external-apps.md) | 1 | An outside app is a registered client with a ceiling a person approved. A ceiling never names a sensitive field | **COMPLETED** |
| 81 | [external-apps.md](external-apps.md) | 2 | Sign-in with OAuth 2.1 and PKCE, consent naming the client and its operator, one-hour tokens, and revocation | **COMPLETED** |
| 82 | [external-apps.md](external-apps.md) | 3 | An outside app calls as the signed-in person, within its ceiling, and every call records the client | **COMPLETED** |
| 83 | [external-apps.md](external-apps.md) | 4 | A TypeScript client generated for each app, and a guide for building outside Scaffoldry | **COMPLETED** |
| 84 | [external-apps.md](external-apps.md) | 8 | AI tools sign in through OAuth as themselves, for a person, with the scope the person picked, and every action names the person, the tool and the task | **COMPLETED** |
| 85 | [foundation.md](foundation.md) | 10 | Tokens from the institution's identity provider are verified. Adds `jsonwebtoken` | **COMPLETED** |
| 86 | [external-apps.md](external-apps.md) | 7 | An outside app may use tokens from the institution's identity provider | **COMPLETED** |
| 87 | [mcp-apps.md](mcp-apps.md) | 8 | Custom pages and review screens render inside the agent, from the published MCP Apps specification | **COMPLETED** |

### Wave 1e — explain, audit, and decide

The rest of the access model, then the rules behind a decision and who decides. These come after the faculty-facing rows so that people can build while the governance around them deepens.

| Order | Brief | Phase | What is true after it | Status |
| --- | --- | --- | --- | --- |
| 88 | [access-model.md](access-model.md) | 7 | Sealed containers, an explanation of any decision, review per person and per object, and impact before a change | **COMPLETED** |
| 89 | [access-model.md](access-model.md) | 12 | A field a person is not cleared for is masked by its category's default, such as a birth year, instead of removed | **COMPLETED** |
| 90 | [access-model.md](access-model.md) | 8 | Receipts for denials, privileged actions and protected reads, sealed into the ledger in batches | **COMPLETED** |
| 91 | [access-model.md](access-model.md) | 9 | Full impersonation of synthetic people only. Writes under it record both people. No persona guessed from an address | **COMPLETED** |
| 92 | [access-model.md](access-model.md) | 10 | Supervision from the directory's manager field, expiring grants, scheduled review, and separation of duties | **COMPLETED** |
| 93 | [access-model.md](access-model.md) | 11 | An Access section in the console: hierarchies, grants, objects, people, reviews and receipts | **COMPLETED** |
| 94 | [policy-library.md](policy-library.md) | 1 | Policies, their sources, and rules that cite a passage word for word | **COMPLETED** |
| 95 | [policy-library.md](policy-library.md) | 2 | A person's agent reads a policy and proposes rules and questions. It cannot publish | **COMPLETED** |
| 96 | [policy-library.md](policy-library.md) | 3 | A person who did not submit a rule reviews it against its passage and publishes it | **COMPLETED** |
| 97 | [authority.md](authority.md) | 1 | A change has a tier. A matrix says who decides. With no rows it behaves as it does today | **COMPLETED** |
| 98 | [authority.md](authority.md) | 2 | The matrix in the console. Its effect on real proposals is shown before a change, and a second person confirms | **COMPLETED** |
| 99 | [policy-library.md](policy-library.md) | 4 | Everyone reads the rules as sentences, linked to policy and source, marked Enforced or Guidance | **COMPLETED** |
| 100 | [authority.md](authority.md) | 3 | The builder is told who decides, why, and what would change it. An agent can ask first | **COMPLETED** |
| 101 | [access-model.md](access-model.md) | 13 | Anyone refused can request access in one step. Routine requests are approved by the institution's rules and expire. The rest go to the right person | **COMPLETED** |
| 102 | [access-model.md](access-model.md) | 14 | An administrator's AI tool drafts access changes from computed suggestions. The server shows sentences and impact. A person confirms | **COMPLETED** |
| 103 | [authority.md](authority.md) | 4 | A simulated institution of 5,000 people, and the same measures from real decisions | **COMPLETED** |
| 104 | [external-apps.md](external-apps.md) | 5 | A confidential client may act as its own service identity, through grants a second person approved | **COMPLETED** |
| 105 | [external-apps.md](external-apps.md) | 6 | An Outside apps panel: clients, ceilings, usage, and each person's connections | **COMPLETED** |
| 106 | [proposal-preview.md](proposal-preview.md) | 3 | A trial copy of a proposed app, filled with synthetic records, to try writes. It expires | **COMPLETED** |
| 107 | [lifecycle.md](lifecycle.md) | 5 | When an owner leaves: orphaned workspaces are listed, a transfer is previewed first, and no workspace loses its last owner | **COMPLETED** |

### Wave 1f — the data layer, the catalog, and MCP guidance (proposed)

Not approved. These rows come from `data-layer.md` and `mcp-guidance.md`. They have no run order yet. Their order is in [workplan-v2.md](workplan-v2.md). The first column has a label, not a number, so `next-phase.py` skips them. They sit on the access model (Waves 1d and 1e) and start after its phase 4.

| Order | Brief | Phase | What is true after it | Status |
| --- | --- | --- | --- | --- |
| 1f-0 | [mcp-guidance.md](mcp-guidance.md) | 1 | An agent that connects is told who it acts for, that a person decides, that stored text is untrusted, and where to look | **COMPLETED** |
| 1f-1 | [data-layer.md](data-layer.md) | 1 | A dataset has an id. Each app table gets one by migration. The app path and the dataset path return the same row | Proposed |
| 1f-2 | [data-layer.md](data-layer.md) | 2 | A data crate is the single door to rows. The architecture audit fails any other reader of the record or label tables | Proposed |
| 1f-3 | [data-layer.md](data-layer.md) | keys | A dataset declares a key, enforced by `record_keys`. A row's unit is a column from the governed list, and a bad value is rejected | Proposed |
| 1f-4 | [data-layer.md](data-layer.md) | catalog | The catalog lists datasets with metadata and synthetic samples. Search works on the desk and in MCP. Discoverability is a policy | Proposed |
| 1f-5 | [data-layer.md](data-layer.md) | catalog | Request access can start from a catalog entry, as an `access-model.md` phase 13 request. Needs the inbox and email | Proposed |
| 1f-6 | [mcp-guidance.md](mcp-guidance.md) | 2 to 4 | Error hints, `explain_decision`, and prompts generated from policy | Proposed |
| 1f-7 | [data-layer.md](data-layer.md) | 3 | Rows and datasets carry labels, with an effective label. Needs the sensitive content rows | Proposed |
| 1f-8 | [data-layer.md](data-layer.md) | 5, 6, 6b | The AI review proposes. A steward reads the evaluation and decides. A change has an effective date, a notice, and exceptions as grants | Proposed |
| 1f-9 | [data-layer.md](data-layer.md) | shares | A row share with a reason and an expiry | Proposed |
| 1f-10 | [data-layer.md](data-layer.md) | 3b, 9 | Relationship access and cross-listing as extra hierarchies. Conditions as clearances with dates. Automations run as service identities | Proposed |
| 1f-11 | [mcp-guidance.md](mcp-guidance.md) | 5 to 7 | Tools are listed by access. Typed text is marked untrusted. Drafts follow `access-model.md` phase 14 | Proposed |

Not scheduled: external collaborators, extra unit tags, run-level tracking of what an automation read, an expedited change path, and standing share policies. See "Version 1" in `data-layer.md`.

### Wave 2 — the data platform

Fields, files, views, import and export, and the million-row proof. A department can bring its spreadsheet and work in it.

| Order | Brief | Phase | What is true after it |
| --- | --- | --- | --- |
| 108 | [calc-graph.md](calc-graph.md) | 3 | Recalculation jobs when a definition changes, and type conversion with a dry run | **COMPLETED** |
| 109 | [calc-graph.md](calc-graph.md) | 4 | Volatile formulas, defaults, and constraints. Adds `platform.timezone` | **COMPLETED** |
| 110 | [jobs.md](jobs.md) | 4 | Quotas, retention of finished jobs, and the Background work indicator | **COMPLETED** |
| 111 | [field-types.md](field-types.md) | 1 | Select options with ids and colors | **COMPLETED** |
| 112 | [record-history.md](record-history.md) | 4 | The disclosure log | **COMPLETED** |
| 113 | [field-types.md](field-types.md) | 2 | Collaborator, created by, and last modified by | **COMPLETED** |
| 114 | [field-types.md](field-types.md) | 3 | Long text with a safe renderer | **COMPLETED** |
| 115 | [field-types.md](field-types.md) | 4 | Duration and date-time | **COMPLETED** |
| 116 | [views.md](views.md) | 1 | Views live apart from the manifest. Personal, collaborative, and locked | **COMPLETED** |
| 117 | [views.md](views.md) | 2 | Filter trees, grouping, footer totals, and color rules | **COMPLETED** |
| 118 | [views.md](views.md) | 3 | A million rows in a window. Uses `@tanstack/react-virtual`, approved by Johann (MIT) | **COMPLETED** |
| 119 | [views.md](views.md) | 4 | Summary views and hand-written accessible charts | **COMPLETED** |
| 120 | [import-export.md](import-export.md) | 1 | An import wizard with a dry run, a rejected-rows report, and resumable jobs | **COMPLETED** |
| 121 | [attachments.md](attachments.md) | 1 | A file store with upload, download, limits, and safe headers | **COMPLETED** |
| 122 | [attachments.md](attachments.md) | 2 | Scanning through an operator's antivirus daemon | **COMPLETED** |
| 123 | [attachments.md](attachments.md) | 3 | Attachment cells and chips in the app | **COMPLETED** |
| 124 | [import-export.md](import-export.md) | 2 | Spreadsheet import and export, with exports logged as disclosures | **COMPLETED** |
| 125 | [import-export.md](import-export.md) | 3 | App bundles. Importing one makes a proposal | **COMPLETED** |
| 126 | [import-export.md](import-export.md) | 4 | Approved templates. An unchanged template skips a review it already had | **COMPLETED** |
| 127 | [import-export.md](import-export.md) | 5 | Snapshots and restore as a new app. A destructive approval takes one first | **COMPLETED** |
| 128 | [import-export.md](import-export.md) | 6 | Moving off a spreadsheet tool, with every decision listed |
| 129 | [links.md](links.md) | 5 | Converting old value joins to links, with a preview |
| 130 | [links.md](links.md) | 6 | Link cells, paste, and a linked records section in the grid |
| 131 | [lifecycle.md](lifecycle.md) | 2 | Retired fields and archived apps |
| 132 | [lifecycle.md](lifecycle.md) | 3 | Retention policies, nightly purge, and a certificate of destruction |
| 133 | [attachments.md](attachments.md) | 4 | Orphan blob purge, retention, and bundles with files |
| 134 | [lifecycle.md](lifecycle.md) | 4 | Legal holds that override every destruction path |
| 135 | [record-history.md](record-history.md) | 5 | History, comments, and disclosures in the record drawer |

### Wave 3 — collaboration and process

People are told, see each other work, and run real committee processes.

| Order | Brief | Phase | What is true after it |
| --- | --- | --- | --- |
| 136 | [realtime.md](realtime.md) | 1 | Live change events over server-sent events, with no data in them |
| 137 | [realtime.md](realtime.md) | 2 | The grid updates in place and keeps what you are typing |
| 138 | [integrations.md](integrations.md) | 1 | An encrypted secrets store and an outbound allowlist |
| 139 | [policy-library.md](policy-library.md) | 5 | Web sources are fetched through the allowlist. A changed source sends its rules back for review | |
| 140 | [policy-library.md](policy-library.md) | 6 | A console panel and one screen for the state of the library | |
| 141 | [notifications.md](notifications.md) | 1 | An inbox and a bell |
| 142 | [notifications.md](notifications.md) | 2 | Email through an allowlisted SMTP host |
| 143 | [realtime.md](realtime.md) | 3 | Presence and a channel for the person's notices and jobs |
| 144 | [approvers.md](approvers.md) | 4 | Delegation by date |
| 145 | [approvers.md](approvers.md) | 5 | The workflow builder picks who decides and previews it |
| 146 | [notifications.md](notifications.md) | 3 | Preferences and digests |
| 147 | [notifications.md](notifications.md) | 4 | Every producer is wired. No notice carries a record value |
| 148 | [process-v2.md](process-v2.md) | 1 | Process events and analytics: where time goes |
| 149 | [workflows.md](workflows.md) | 1 | Flows run with approved grants and reach records by address. They can span apps |
| 150 | [workflows.md](workflows.md) | 2 | A flow touching several workspaces needs approval from each. Moving sensitive data is flagged |
| 151 | [process-v2.md](process-v2.md) | 2 | Branches, cross-table effects, manual and date triggers |
| 152 | [workflows.md](workflows.md) | 3 | Schedules, called flows, chaining, and a circuit breaker |
| 153 | [field-types.md](field-types.md) | 5 | A button field that starts a process |
| 154 | [process-v2.md](process-v2.md) | 3 | Timers and escalation. No timer ever decides |
| 155 | [process-v2.md](process-v2.md) | 4 | Committee votes |
| 156 | [workflows.md](workflows.md) | 4 | Fill-in tasks, with a task grant on one record while the step is open |
| 157 | [process-v2.md](process-v2.md) | 5 | Builder and desk screens for all of it |
| 158 | [workflows.md](workflows.md) | 5 | My Work: one list of decisions, tasks, and reviews across the institution, kept in step by jobs |
| 159 | [workflows.md](workflows.md) | 6 | The My Work page and a review queue |
| 160 | [workflows.md](workflows.md) | 7 | Flow editor: one command model, the outline, and server validation |
| 161 | [workflows.md](workflows.md) | 8 | Flow editor: the block-structured canvas |
| 162 | [workflows.md](workflows.md) | 9 | Test run with a trace on the canvas |
| 163 | [workflows.md](workflows.md) | 10 | Run history, safe retry, and version comparison |

### Wave 4 — ecosystem and reach

Tokens that can be narrower, webhooks, synced datasets, custom pages, people outside the institution, and the compliance layer.

| Order | Brief | Phase | What is true after it |
| --- | --- | --- | --- |
| 164 | [integrations.md](integrations.md) | 4 | Webhooks behind an allowlist and a proposal, with signing, retries, and a call log |
| 165 | [integrations.md](integrations.md) | 5 | Outbound access, secrets, and webhook panels |
| 166 | [connections.md](connections.md) | 1 | Datasets are real and read-only. A file-drop sync |
| 167 | [connections.md](connections.md) | 2 | A changed source stops the sync and makes a proposal |
| 168 | [connections.md](connections.md) | 3 | PostgreSQL connector. Needs a TLS connector |
| 169 | [connections.md](connections.md) | 4 | HTTP connector |
| 170 | [connections.md](connections.md) | 5 | Apps link to datasets |
| 171 | [connections.md](connections.md) | 6 | The connections panel |
| 172 | [views.md](views.md) | 5 | Dashboards and linked widgets |
| 173 | [views.md](views.md) | 6 | Shared links, off until enabled, never including sensitive fields |
| 174 | [views.md](views.md) | 7 | Forms with conditions, boards, calendars, and galleries |
| 175 | [guests.md](guests.md) | 1 | A guest proves an email address with a one-time link |
| 176 | [guests.md](guests.md) | 2 | A form open to guests, with a built-in own-records rule |
| 177 | [guests.md](guests.md) | 3 | A plain status page for the guest |
| 178 | [guests.md](guests.md) | 4 | Operating guests: closing forms, erasing, abuse signals |
| 179 | [admin-console.md](admin-console.md) | 4 | The rest: labels, routes, dataset classification, and screens |
| 180 | [sensitive-content.md](sensitive-content.md) | 2 | Stored records are scanned as a job. A compliance officer reviews findings and designates authorized stores |
| 181 | [sensitive-content.md](sensitive-content.md) | 3 | New writes, imports, and plain-text files are checked. Each detector flags, warns, or blocks, as the organization chooses |
| 182 | [sensitive-content.md](sensitive-content.md) | 4 | History, exports, and snapshots honor the categories. The old flag name is gone |
| 183 | [oscal-catalog.md](oscal-catalog.md) | 1 | The NIST catalogs and baselines, slimmed and validated. Needs files from Johann |
| 184 | [admin-console.md](admin-console.md) | 5 | Policy and OSCAL: versioned Cedar policy with test cases, a real requirements export |
| 185 | [oscal-catalog.md](oscal-catalog.md) | 2 | Control titles, validation, and baseline coverage |
| 186 | [admin-console.md](admin-console.md) | 6 | Business processes: inventory, versions, instance monitor, reassign, cancel |
| 187 | [workflows.md](workflows.md) | 11 | Console panel for flows |
| 188 | [live-data.md](live-data.md) | 3 | Moved to Wave 1c as 1c-1. The desk reads records and the directory from the API |
| 189 | [foundation.md](foundation.md) | 11 | One image serves the API and the web files |
| 190 | [admin-console.md](admin-console.md) | 7 | Audit explorer, token management, agent controls, and a read-only admin inventory tool |
| 191 | [ux-standards.md](ux-standards.md) | 3 | The desk uses the kit. No browser dialogs. No swallowed errors. CI enforces both |
| 192 | [data-grid-parity.md](data-grid-parity.md) | open phases | Grid work on the web desk. Last, because the agent is the primary surface |

Foundation phases 10 and 11 depend on nothing after phase 9. Run them earlier if a pilot date appears.

Migrations. Each phase takes the next free number on `main` and names its file by a slug, as its brief does: `*_delegations.sql` means "the next free number, then `_delegations.sql`". A brief names the shipped file once it exists. `governance/scripts/check-migrations.py` fails when two files share a number or a file is not registered. When two branches take the same number, the second to merge renumbers its file.

Register each file in `crates/scaffoldry-server/src/repository.rs` after the last schema already applied, inside the existing advisory lock. A file that exists is never edited. A fix is a new file. Each file is recorded by name in `schema_migrations`, so the run order above need not match filename order.

Rules that hold across every brief:

- PostgreSQL is the only source of truth. The server holds nothing between requests except the cached collections named in `stateless-server.md`, through its one mechanism. No brief adds a field to `ServerState`, a `static` lock, or a map that lives across requests. A write commits before memory changes.
- Every write route and every writing tool has a case in the two-instance test.
- No write is discarded. Do not put `let _ =` in front of a repository call or a ledger call.
- Access is decided from stored rows, never from a value in the request body.
- There is no production mode. Do not add a variable, a setting, or a flag that turns a check off. Institution configuration is a row in `platform_settings`.
- Scaffoldry signs no token. An agent token is a random string stored as a hash.
- A user's schema is JSON. No statement creates a table, a column, or an index for a user's table or field.
- Records are filtered, sorted, counted, and totalled in PostgreSQL, never in Rust or in the browser.
- Every change an administrator makes carries a reason and is a ledger entry written before the change. No MCP tool applies an admin write. An agent may draft one: the draft is a pending change, with its sentences and its impact, that applies only when a person with the right confirms it.
- Admin routes are listed in `ADMIN_ROUTES` and checked on the server. The client's menu is not a control.
- A rule that grants nothing and is never evaluated does not exist. No field holds policy text that no code evaluates.
- A workspace guard can only forbid. Institutional policy permits.
- Grants permit, guards forbid, rules narrow. A person's role on an object comes only from `role_at`, the highest role among the grants whose reach contains it. A grant's reach depends only on its own row and the trees, never on another person's assignment.
- A hierarchy is data. No hierarchy, unit, domain or position key is written in code or in a Cedar policy.
- Nothing that decides access fails open. A failed load, read or compile is an error, never an empty list or a skipped rule.
- An app outside Scaffoldry reaches data only as a registered client, as the signed-in person, within its approved ceiling. A ceiling never names a sensitive field.
- An approval applies to the digest of what was reviewed. Nothing else goes live.
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
| The default automatic-approval rule for access requests applies only to workspaces whose visibility is departmental or institutional. A restricted workspace always routes to its owner. The rule is a viewer, Internal or lower, in the requester's own unit or below, for at most 30 days | `access-model.md` phase 13 |
| An AI tool drafts access changes and never applies them. A draft expires after 7 days and records the person, the tool, and the task. A widening draft cannot be confirmed by the person whose tool drafted it | `access-model.md` phase 14 |
| The license whitelist is Apache 2.0, MIT, BSD-2-Clause, BSD-3-Clause, ISC, CC0-1.0, and the PostgreSQL License. `python-2.0` and `unlicense` are removed from the audit. The audit still passes, so no package needed a decision | `AGENTS.md`, `governance/scripts/audit-licenses.py` |
| The `million_rows` test keeps every limit and gates on the 19th of twenty runs sorted fastest to slowest (the 95th percentile). It still prints and records the slowest. One slow run on a shared runner does not fail it. Two do | `row-scale.md` phase 6 |
| Institutional rules are kept as an OSCAL catalog of sentences, each citing a stored passage. A person's own agent proposes them. A different person publishes them. Scaffoldry runs no model | `policy-library.md` |
| The platform fetches web sources itself, through the outbound allowlist. It does not extract text from a PDF | `policy-library.md` phase 5 |
| A change has a FIPS 199 tier from the checks it raises. A matrix says who decides. The institution sets it, units only tighten it, and a second person confirms every change to it | `authority.md` |
| A proposal with no resolvable approver waits and is shown as Unassigned. It never falls back to a looser path | `authority.md` |
| Grants permit. Guards forbid. Rules narrow. Every object has a unit in each hierarchy, a domain and a sensitivity, and a child only narrows its container's | `access-model.md` |
| There may be several org hierarchies. Each is a tree with one root and one parent per unit. A hierarchy is added as data, by a proposal a second person confirms, and is retired, never deleted. This replaces "there is one root" in `organization.md` | `access-model.md` phase 2 |
| A workspace, app or table may be sealed, which stops grants from above. Rows cannot be sealed. Unsealing needs a second person | `access-model.md` phase 7 |
| Full impersonation is only of synthetic people. A real person is viewed only through view-as. Johann may change this | `access-model.md` phase 9 |
| SCIM creates no admin appointment: not `platform_admin`, `unit_admin` or `compliance` | `access-model.md` phase 1 |
| A position type does not grant a role by itself. A `holder_unit` grant does that, with a reach, a domain and an expiry | `access-model.md` phase 4, replacing `position-roles.md` phase 2b |
| An app hosted anywhere uses institutional data as a registered OAuth client, as the signed-in person, within a ceiling a person approved. A ceiling never names a sensitive field. Johann may change this | `external-apps.md` |
| An approval names the digest it reviewed. A reviewer can open the proposed app first, read-only | `proposal-preview.md` |
| Complexity is the system's job: people see sentences, safe defaults and one decision at a time, and an AI tool drafts the structure | `ux-standards.md` rules 11 to 16 |
| No MCP tool applies an admin write. An agent may draft one as a pending change that a person confirms | `access-model.md` phase 14 |
| A field a person is not cleared for is masked by its category's default instead of removed | `access-model.md` phase 12 |
| A request that fits the institution's rules is approved without anyone and expires. The default rule approves viewing non-sensitive apps in a person's own unit for up to 30 days. Johann may change this | `access-model.md` phase 13 |
| An AI tool signs in as itself, for a person, with the scope the person chose. Every action names the person, the tool and the task | `external-apps.md` phase 8 |
| A custom page may be a bundle of up to 2 MB that carries its own libraries, each declared and on the license list | `mcp-apps.md` phase 6 |
| Dependencies may be Apache-2.0, MIT, BSD-2-Clause, BSD-3-Clause, ISC or the PostgreSQL License | `AGENTS.md`, approved 2026-10-16 |
| `calamine`, `rust_xlsxwriter` and `flate2` are approved, each added only in the phase that names it and only if the license audit passes. `aes-gcm` waits for the master key decision | `import-export.md`, approved 2026-10-13 |
| CI runs the Rust tests with `cargo-nextest`, in parallel, with a time limit per test | `.github/workflows/ci.yml`, approved 2026-10-13 |
| The million-row test met every limit. The JSON record with a pivot index stays | `row-scale.md` phase 6, PR 114 |

Open decisions. These are Johann's. Do not settle them in a session.

| Decision | Until decided |
| --- | --- |
| Browser redirect to the identity provider, and which provider the first pilot uses | The desk signs in with a pasted token |
| What the Cloud Run demo becomes once one image needs PostgreSQL | `deploy.yml` and `Dockerfile.web` are not touched |
| Git export of app definitions and of the ledger head hash | Not built |
| The crates still waiting in the master brief: `ureq` with `rustls`, `aes-gcm`, `lettre`, and a PostgreSQL TLS connector | Each phase that needs one stops until Johann says yes and the license audit passes |
| Adding `axe-core` (MPL-2.0) as a dev-only test dependency | Not added. Accessibility is tested with role, name, and keyboard tests |
| Running ClamAV as a separate daemon for upload scanning | Files show as not scanned. An operator can require a scanner |
| Who supplies the NIST catalog files, and the 800-171 revision CMMC requires | `oscal-catalog.md` stops until `governance/catalog/source/` and `PROVENANCE.json` exist |
| MCP tools for templates (`list_templates`, `instantiate_template`). The tool list has about 10 bytes of room under its 24,000-byte budget, so a tool needs either shorter descriptions elsewhere or a higher budget with a stated reason | Templates are reachable through REST and the desk. No MCP tool |
| Snapshot expiry. Snapshots carry `expires_at` and nothing purges them, because retention policies and legal holds (`lifecycle.md` phases 3 and 4) are not built. When they are, the purge must skip a held app's snapshots | Snapshots are kept until someone removes them. `import-export.md` phase 5 test 5 is not built |
| Where the master key file for the secrets store is kept, and who holds it | Secrets routes answer `master_key_missing` |
| The retention numbers and the records officer who owns them | Placeholders, and a banner until real ones are set |
| Whether Scaffoldry is also offered as a hosted service, and whether that is one stack for each institution | Self-hosting stays supported. Nothing assumes more than one institution in a database. `stateless-server.md` removes the one thing that blocks a second instance |
| A full export of one institution in documented formats, and whether process definitions are also exported as BPMN XML | Exports are per app and capped. Not built |
| Who holds the positions the first matrix names, such as a privacy officer | The matrix has no rows and behaves as the platform does today |
| Whether the license audit should keep accepting `cc0-1.0`, `python-2.0` and `unlicense`, which the written policy does not list | The audit accepts them |
| Whether the desk keeps its built-in demo people (the sign-in list, the Impersonation Hub, and the mock workspaces with their collaborators), or starts signed out and takes every person from the server | They stay. They grant nothing, because the server checks every request. Real impersonation is Admin, People. Removing them touches about 50 test sites in 13 files and the desk's mock data, so it is its own phase |
| What an imported bundle does with its sample records and its saved views, and whether bundles are compressed with `flate2` | Sample records are exported (at most 5,000, sensitive fields removed) and checked on import, but not loaded. Saved views travel as manifest views inside the proposal. `flate2` is not added: sample data stays inside the document. A webhook arrives off, and its `webhook_host` check waits for `integrations.md` phase 4 |
