# Gemini workplan

Seven briefs. One phase per session. Do not start the next phase until the current phase's tests pass. Paste the prompt at the bottom of the brief you are in.

Scaffoldry is MCP-first. The institution's own agent builds apps through the MCP server. The web desk is a second client of the same service layer. Read "Decisions already made" in [foundation.md](foundation.md) before any session.

The foundation comes first. It adds almost no feature. Several routes do not check the caller today, and several writes never reach PostgreSQL. Feature work on top of that is wasted.

| Order | Brief | Phase | What is true after it |
| --- | --- | --- | --- |
| 1 | [foundation.md](foundation.md) | 1 | README and ARCHITECTURE describe the code that exists |
| 2 | [foundation.md](foundation.md) | 2 | A failed ledger write fails the request. Sessions expire |
| 3 | [foundation.md](foundation.md) | 3 | Migrations run once. The ledger rejects update and delete. Workspace changes are stored |
| 4 | [foundation.md](foundation.md) | 4 | Eight database connections |
| 5 | [foundation.md](foundation.md) | 5 | An app belongs to a workspace. App and record access comes from stored roles |
| 6 | [foundation.md](foundation.md) | 6 | No route is left without an access check |
| 7 | [foundation.md](foundation.md) | 7 | Tokens are hashed rows a user can mint and revoke. First boot prints a setup token. The open token route and the shared signing secret are gone |
| 8 | [foundation.md](foundation.md) | 8 | Institution settings live in PostgreSQL and every change is in the ledger. `SCAFFOLDRY_ENV` is gone. Demo data is a command |
| 9 | [foundation.md](foundation.md) | 9 | A user mints an agent token in the settings pane. A Platform Admin edits settings in `/admin` |
| 10 | [organization.md](organization.md) | 1, 2, 5 | Code is in the working tree, not committed. Run its tests, fix what foundation 2 to 8 changed, commit |
| 11 | [organization.md](organization.md) | 3 | A Platform Admin opens `/admin` → Organization and manages the tree. `organization-admin.test.tsx` exists and fails today |
| 12 | [organization.md](organization.md) | 4 | An Org Unit Admin sees only their units on the workspace rail |
| 13 | [live-data.md](live-data.md) | 1–2 | Records live in PostgreSQL only. A save carries a version and conflicts return 409 |
| 14 | [row-scale.md](row-scale.md) | 1 | A record has a `table_id` column. A list is one indexed page, reached by cursor |
| 15 | [mcp-apps.md](mcp-apps.md) | 1 | One tool registry with one gate. The fake tools are gone |
| 16 | [mcp-apps.md](mcp-apps.md) | 2 | An agent reads and writes records, a page at a time |
| 17 | [row-scale.md](row-scale.md) | 2 | Sort and filter on an indexed field use the pivot index at any table size |
| 18 | [row-scale.md](row-scale.md) | 3 | Formula, count, and rollup values are stored when a record changes. Lookups are one statement per page |
| 19 | [mcp-apps.md](mcp-apps.md) | 3 | The app manifest has a published schema, pages, and size limits |
| 20 | [mcp-apps.md](mcp-apps.md) | 4 | An agent proposes an app change, with page source. A person approves it. The ledger records it |
| 21 | [mcp-apps.md](mcp-apps.md) | 5 | An agent lists and decides waiting process steps |
| 22 | [mcp-apps.md](mcp-apps.md) | 6 | A custom page is rendered into a sandbox document and bounded by its grants |
| 23 | [mcp-apps.md](mcp-apps.md) | 7 | The web desk shows a custom page in a sandboxed frame |
| 24 | [row-scale.md](row-scale.md) | 4–5 | Totals and group counts are one statement. Full-text search |
| 25 | [live-data.md](live-data.md) | 3 | The desk reads records and the directory from the API |
| 26 | [foundation.md](foundation.md) | 10 | Tokens from the institution's identity provider are verified against keys stored as a setting. Adds `jsonwebtoken` |
| 27 | [foundation.md](foundation.md) | 11 | One image serves the API and the web files |
| 28 | [mcp-apps.md](mcp-apps.md) | 8 | Custom pages and the review screens render inside the agent. Needs the MCP Apps spec pasted in |
| 29 | [row-scale.md](row-scale.md) | 6 | A million-row table meets the stated limits, with the measured times on record |
| 30 | [business-process.md](business-process.md) | open phases | Re-run each phase's tests after foundation 8. Fix what the access checks changed |
| 31 | [data-grid-parity.md](data-grid-parity.md) | open phases | Grid work on the web desk. Last, because the agent is the primary surface |

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

Register each file in `crates/scaffoldry-server/src/repository.rs` after the last schema already applied, inside the existing advisory lock. A file that exists is never edited. A fix is a new file. The run order above is not filename order. That is safe: each file is recorded by name in `schema_migrations`, and no file depends on a later-numbered one.

Rules that hold across every brief:

- No write is discarded. Do not put `let _ =` in front of a repository call or a ledger call.
- Access is decided from stored rows, never from a value in the request body.
- There is no production mode. Do not add a variable, a setting, or a flag that turns a check off. Institution configuration is a row in `platform_settings`.
- Scaffoldry signs no token. An agent token is a random string stored as a hash.
- A user's schema is JSON. No statement creates a table, a column, or an index for a user's table or field.
- Records are filtered, sorted, counted, and totalled in PostgreSQL, never in Rust or in the browser.
- A layout the built-in views cannot express is a custom page in a sandbox. Do not extend the component catalog.
- A feature that an agent needs is a service function first. The MCP tool and the REST route both call it.
- The grid does not grow org controls. The process desk does not move to `/admin`. `/admin` stays limited to a Platform Admin (`central_admin`, or a `platform_admin` appointment on the root). The other admin role is Org Unit Admin (`unit_admin` on one unit). Do not label either role Super Admin, System Admin, or Org Admin.

Open decisions. These are Johann's. Do not settle them in a session.

| Decision | Until decided |
| --- | --- |
| Browser redirect to the identity provider, and which provider the first pilot uses | The desk signs in with a pasted token |
| What the Cloud Run demo becomes once one image needs PostgreSQL | `deploy.yml` and `Dockerfile.web` are not touched |
| Remove `PUT /apps/{slug}` so every app change is a proposal | The route stays, with manifest validation |
| May a proposer approve their own unflagged proposal | Yes, as written in mcp apps phase 4 |
| Who approves a custom page | A workspace owner or admin. A page that reads a `ferpa_sensitive` field needs a second person. Not an Org Unit Admin |
| Compiling a page's JSX | In the sandbox with Sucrase, as written in mcp apps phase 6. The server checks no syntax |
| Git export of app definitions and of the ledger head hash | Not built |
