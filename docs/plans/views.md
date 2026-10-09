# Views and dashboards — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `mcp-apps.md` phase 4 and `jobs.md` phase 3. Phases 2 to 4 need `row-scale.md` phase 4. Phase 5 needs `mcp-apps.md` phase 7. Phase 6 needs `access-rules.md` phase 3 and `record-history.md` phase 4.

People look at the same data in different ways. A chair wants a board of applications by stage. A coordinator wants the same table grouped by program with a total at the foot. A dean wants a dashboard: a chart, a count, and a list that follows the chart. Today every view is part of the governed manifest, so changing a sort on your own board is a proposal. The views are few, they have no groups or colors, and nothing summarizes.

This brief moves views out of governance, because a view cannot widen access, and builds the views and dashboards people expect, at any table size.

## Decisions already made

| Question | Answer |
| --- | --- |
| Is a view governed? | No. A view is presentation. It cannot grant access, because every read goes through `row_scope` and `field_visibility`. An approved page still is governed, because it can carry code |
| Who changes a view? | A personal view: its owner, who needs only read access. A collaborative view: editors and above. A locked view: admins and above. A locked view may still be used by everyone |
| Where do views live? | In their own table, apart from the manifest. The manifest holds tables, fields, access rules, and pages |
| Are charts drawn with a library? | No. They are hand-written SVG with a table alternative, so they are accessible and add no dependency |
| Are shared links on? | Off. A Platform Admin turns on a scope. No passwords. A shared view never includes a sensitive field |

## What exists today

| Fact | Where |
| --- | --- |
| `AppManifest.views` holds `AppView` entries. Changing one is a manifest proposal | `scaffoldry-engine/src/lib.rs` |
| `ViewType` and a single-level `CompoundFilter` | `scaffoldry-engine/src/lib.rs` |
| Sort, filter, and index derivation read views from the manifest | `row-scale.md` phase 2 |
| The grid brief plans column chrome (order, width, hidden, frozen, summary) stored on the view | `data-grid-parity.md` phase 4 |
| An aggregate route exists | `row-scale.md` phase 4 |

## Out of scope

- A map view and a timeline with dependencies.
- Per-cell conditional formatting by formula. Color rules are by filter.
- Embedding a dashboard in another site.
- Cross-app dashboards.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency, except `@tanstack/react-virtual` in phase 3 once Johann has said yes.
3. A view never widens access. A test proves a view cannot return a hidden row or field.
4. A list, a group, a total, or a chart is computed by the server over the indexed set. The browser never sorts, filters, groups, or totals records.
5. Color is never the only signal.

## Phase 1 — views live on their own

Migration `crates/scaffoldry-core/migrations/0033_app_views.sql`:

```sql
CREATE TABLE IF NOT EXISTS app_views (
    id UUID PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64),
    name VARCHAR(120) NOT NULL,
    kind VARCHAR(12) NOT NULL,
    ownership VARCHAR(13) NOT NULL DEFAULT 'collaborative',
    owner VARCHAR(255),
    config JSONB NOT NULL DEFAULT '{}',
    version INTEGER NOT NULL DEFAULT 1,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_by VARCHAR(255) NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_app_views_app ON app_views (app_slug) WHERE deleted_at IS NULL;
```

`kind` is `grid`, `kanban`, `calendar`, `gallery`, `form`, `summary`, or `dashboard`. `ownership` is `collaborative`, `personal`, or `locked`.

Migration of existing apps. When a manifest is first loaded after this phase, copy each `AppManifest.views` entry into `app_views` as a collaborative view, keep its id where it is a UUID and generate one otherwise, empty `views` in the stored manifest, and record the migration in the ledger once with a count. `AppPage::View { view_id }` points at `app_views.id`. The manifest validator stops validating view contents. It keeps checking that a `View` page's id exists when the proposal is approved.

Routes.

| Method | Path | Behavior |
| --- | --- | --- |
| GET | `/apps/{slug}/views` | The caller's personal views and all collaborative and locked ones |
| POST | `/apps/{slug}/views` | Create. A personal view needs read access. Collaborative needs `editor`. Locked needs `admin` |
| PUT | `/apps/{slug}/views/{id}` | Body `config`, `name`, and `version`. A stale version is `409`. Rights as above, by the view's ownership |
| DELETE | `/apps/{slug}/views/{id}` | Soft delete, rights as above |
| POST | `/apps/{slug}/views/{id}/duplicate` | Into a personal view |

MCP tools `list_views`, `save_view`, and `delete_view`, scope `App(Read)`, with the ownership rule enforced inside. A view's `config` holds the sort, filters, hidden fields, order, widths, grouping, and aggregations from `data-grid-parity.md` phase 4, and the new settings of this brief.

Indexes follow views. Saving a view that sorts or filters on a field outside the table's indexed set triggers the job from `jobs.md` phase 3, and the response says `index_state: building`. A table may index at most 20 fields. Past that, the save is `400` with error code `too_many_indexes` and the list of indexed fields no view uses. The Data governance panel in `admin-console.md` shows unused indexed fields for cleanup.

Tests.

1. Manifest views migrate to `app_views` with the same ids and the manifest holds none afterwards.
2. A viewer creates a personal view. A viewer cannot create a collaborative one. An editor can. An editor cannot change a locked view. An admin can.
3. A personal view is invisible to others.
4. A stale `version` is `409`.
5. A view with a sort on an unindexed field of a 200,000-row table returns `index_state: building` and enqueues one job.
6. A twenty-first indexed field is `too_many_indexes`.
7. A view cannot return a row or field the person may not read. Build a view that tries, and assert the response.
8. The migration writes one ledger entry with the count.

## Phase 2 — richer views

1. Filter trees. Replace `CompoundFilter` with a tree: `{ conjunction, children }` where a child is a clause or another group, nested at most two levels. Add the operators `In`, `Between`, `Before`, `After`, and relative dates `today`, `this_week`, `last_7_days`, `last_30_days`, `next_30_days`, evaluated on the server in `platform.timezone`. Compile to SQL over the index as in `row-scale.md` phase 2. A field outside the indexed set is allowed only on tables of 10,000 rows or fewer.
2. Grouping. A view may group by up to three fields. The server returns the groups with counts (at most 200 per level, with a flag when more exist), and pages the rows of an opened group. The cursor carries the group path.
3. Footer aggregations. Each field's footer choice: `count`, `filled`, `empty`, `unique`, `sum`, `avg`, `min`, `max`. Computed by the aggregate route over the whole filtered set. A hidden field offers none.
4. Color rules. Up to 10 per view: `{ id, label, when: <filter tree>, tone, target: row | field }`, with `tone` from the fixed set. The server evaluates each rule on the rows it returns and adds `_tone` to each row with the matching rule labels. A tone shows as a color and as an icon and the rule label in the accessible name. A rule using a hidden field is dropped for that person.

Tests.

1. A nested filter `(a = 1 OR a = 2) AND b > 5` returns the right rows and its plan has no sequential scan.
2. `last_7_days` at 00:30 in the institution's zone and at 23:30 include the right boundary days.
3. Grouping by two fields returns groups with counts that sum to the filtered total. Opening a group pages only its rows.
4. A footer `sum` over 200,000 rows is correct and runs as one statement.
5. A color rule marks the right rows. A row matching two rules shows both labels. A hidden field's rule is dropped.
6. A tone is exposed in the accessible name, not only in a style.

## Phase 3 — a million rows in a window

Needs Johann's yes on `@tanstack/react-virtual`.

1. Offset window. `GET .../rows` accepts `offset` and `limit` in addition to the cursor, for an ordering over an indexed key. Measure first. Run `SELECT ... ORDER BY ... OFFSET 500000 LIMIT 200` over 1,000,000 rows and paste the plan and the time. If it is under 300 ms and the plan uses the index, offsets are allowed up to 1,000,000. If not, build anchors instead: a job `build_anchors` samples the sort key of every 1,000th row into `view_anchors (view_id, position, cursor)` (migration `0047_view_anchors.sql`) and a jump fetches from the nearest anchor with a cursor.
2. Counts. A request asks `count=exact` and runs under a two-second statement timeout. If it times out, the response is `{ "at_least": n }`, and the screen says `Over 250,000 records`. A view with no filter uses the stored estimate for the table plus the delta of writes since, shown as `about`.
3. The last page. Jump to end runs the same query with the order reversed and flips the result.
4. The grid renders only visible rows with a buffer, fetches windows of 200, shows skeleton rows while a window loads, and keeps `aria-rowcount` and `aria-rowindex` correct. Page Down, Page Up, Ctrl+Home, and Ctrl+End work. Dragging the scrollbar to the middle loads that window.

Tests.

1. The measured `OFFSET` result is pasted. The chosen strategy passes its latency limit at 1,000,000 rows.
2. A jump to 50 percent returns the right rows for the chosen strategy.
3. `count=exact` on a hard filter returns `at_least` within the timeout.
4. Last-page returns the final 200 rows in the right order.
5. Web: scrolling renders at most the visible rows plus the buffer, and `aria-rowindex` is correct. Ctrl+End loads the end.
6. A scoped person (`access-rules.md`) gets counts and windows over their scope only.

## Phase 4 — summaries and charts

Summary view. `kind: summary { group_by: [field], aggregations: [{ field, fn }] }`. A read-only table of groups and aggregates from the aggregate route, at most 200 groups with a flag for more. A row opens the grid with that group as a filter, in an unsaved view carried in the address.

Aggregate route additions: `bucket` for a date field (`day`, `week`, `month`, `quarter`, `year` in `platform.timezone`), a second `group_by`, and `fn` of `count_distinct`.

Charts, in `apps/web/src/ui/charts/`, hand-written SVG, no library.

| Chart | Data |
| --- | --- |
| Bar, grouped and stacked | One or two group fields and an aggregate |
| Line and area | A date bucket and an aggregate, one or more series |
| Pie and donut | One group field, at most 12 slices, the rest as `Other` |
| Stat | One aggregate, with an optional comparison to a second filter |

Every chart has axis labels and tick values, direct value labels where room allows, a legend as text, pattern fills in addition to color so series are distinguishable without color, and a `View as table` button that shows the exact data. Points are reachable by keyboard: arrows move, and a live region announces `March: 42 applications`. Empty data shows `EmptyState`. A chart over a hidden field is not offered.

Tests.

1. A summary view over 200,000 rows matches a direct aggregate and has 200 groups at most.
2. A date bucket by month in the institution's time zone puts a record on the right month across a boundary.
3. A bar chart renders one rect per group with a text label and a pattern fill. The table alternative holds the same numbers.
4. Arrow keys move through points and the live region announces each.
5. A pie over 20 groups renders 11 slices and `Other`.
6. A stat with a comparison renders both numbers and the difference in words.

## Phase 5 — dashboards and linked widgets

`kind: dashboard { columns: 12, widgets: [Widget] }`.

```rust
pub struct Widget {
    pub id: String, pub title: String,
    pub x: u8, pub y: u8, pub w: u8, pub h: u8,
    pub body: WidgetBody,
    pub select_by: Option<SelectBy>,
}
pub enum WidgetBody { View { view_id: Uuid }, Chart { view_id: Uuid }, Stat { ... }, Text { markdown: String }, Page { page_id: String } }
pub struct SelectBy { pub source_widget: String, pub source_field: String, pub target_field: String }
```

Linked widgets. When `select_by` is set, selecting a row in the source widget makes this widget's request add the filter `target_field = <selected row's source_field value>` to its own filters. The server runs it. Nothing is filtered in the browser. Clearing the selection removes it.

Rules. Widgets may not overlap and must fit the 12 columns. A `Page` widget must name an approved custom page of this app. A dashboard names no unapproved page. A `Text` widget uses the safe Markdown renderer from `field-types.md`. A dashboard is a presentation object and needs no proposal, but each `Page` widget is governed by that page's own approval.

Pages in a widget receive a read-only `scaffoldry.context.selection`, the selected record id from the linked source, through the bridge. It gives the page nothing it cannot already read.

Layout editing is operable by keyboard: select a widget, arrows move it, Shift with arrows resizes, and numeric inputs set the values exactly. Below 900 px the widgets stack in reading order. The dashboard is one landmark per widget with its title as the name.

Tests.

1. Selecting a row in a list widget filters a linked chart through the server, and the request holds the filter.
2. Overlapping or out-of-bounds widgets are `400`.
3. A page widget naming an unapproved page is `400`.
4. A restricted person's dashboard shows only their scope in every widget.
5. Web: arrow keys move a widget, Shift-arrows resize it, and the numeric inputs agree. At 800 px widgets stack in order.
6. The context selection reaches a page through the bridge and holds only an id.

## Phase 6 — shared links

Migration `0034_view_shares.sql`:

```sql
CREATE TABLE IF NOT EXISTS view_shares (
    id UUID PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    view_id UUID NOT NULL,
    token_hash CHAR(64) NOT NULL UNIQUE,
    scope VARCHAR(12) NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ,
    last_used_at TIMESTAMPTZ,
    use_count BIGINT NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS rate_limits (
    key VARCHAR(200) NOT NULL,
    window_start TIMESTAMPTZ NOT NULL,
    count INTEGER NOT NULL,
    PRIMARY KEY (key, window_start)
);
```

`scope` is `institution` (any signed-in person at the institution, with their own row rules applied) or `public` (anyone with the link). The setting `sharing.allowed_scopes` is an array of those two, default empty, and a Platform Admin changes it through the settings route. A `public` share is refused unless the workspace's classification is `Public`, with error code `share_not_allowed`. Add that code to the closed list.

Rules.

- A token is 128 random bits, shown once, stored as a hash. The link is `/s/{token}`.
- Expiry is required. It defaults to 30 days and is at most 90.
- At most 20 active shares per view.
- A shared view is read-only. It cannot export unless the share was made with `allow_download`, and a download of a table with any sensitive field is refused for `public`.
- A shared view never includes an effective-sensitive field or a hidden field, always.
- For `public`, the viewer has no person attributes, so a row rule that depends on a person folds to `DenyAll`, and the share shows no rows. This is correct, and the creator is told at creation time.
- For `institution`, the viewer must be signed in, `row_scope` applies as that person, and a workspace guard can still forbid.
- Each request counts against `rate_limits`: 60 per minute per token and 20 per minute per address for `public`. Past the limit, `429` with `rate_limited`.

Routes. `POST /apps/{slug}/views/{id}/shares` (app owner or admin), `GET` of the list, `DELETE` of one. Public data routes under `/api/v1/shared/{token}/...` mirror the read routes for the view only. The share's creation, use count, and revocation are in the admin console under `Shares`, with `Revoke all for this app`, in `ADMIN_ROUTES`.

Screen. A `Share` button on the view for owners and admins: scope choice (only those enabled), expiry, download choice, and a plain sentence of who can open it. A `public` share shows a warning `Banner`. The link is shown once with `Copy`. A list shows active shares with `Revoke`.

Tests.

1. With no scope enabled, creating a share is `share_not_allowed`. With `institution` enabled it works.
2. `public` on a workspace that is not `Public` is `share_not_allowed`.
3. A shared view's response has no sensitive or hidden field, for any share and any person.
4. A `public` share of a table with a person-dependent row rule returns no rows, and the creator was warned.
5. A revoked or expired token is `404`. The token is stored only as a hash.
6. The 61st request in a minute is `429`.
7. A guard that forbids a unit blocks an `institution` share for that unit.
8. Web: the link is shown once. A public share shows the warning.

## Phase 7 — forms, boards, calendars, and galleries

- Form view. A subset and order of fields, labels, help text, a success message, and `show_when` conditions per field over `Eq`, `Ne`, `In`, `IsEmpty`, and `IsNotEmpty` of other form fields. Conditions are evaluated in the browser for immediate feedback and again on the server at submit, which is authoritative. This is the one place a second evaluator exists. It is tiny, and its test vectors are generated from the Rust one into a JSON file the TypeScript test reads. A form is submitted through the ordinary create route, so every rule applies. A form for people outside the institution is `guests.md`.
- Board. Columns by single-select id, with counts from the group aggregate and cards paged per column, 50 at a time with `Load more`. A move calls the update route with the version. A keyboard move uses Alt with arrows and announces `Moved to Offer made.` An optional limit per column shows a warning, not a block.
- Calendar. Month, week, and day. It fetches by date range through the index and shows at most 500 events per range with `+12 more`. Events open the record drawer.
- Gallery. A cover from the first image of an attachment field and chosen fields on the card.

Tests.

1. Every vector in the generated file gives the same visibility in Rust and TypeScript. A hidden required field is not required on submit. The server rejects a submit that violates a condition the client skipped.
2. A board of 10,000 cards loads only the first page per column and counts match the aggregate.
3. A keyboard move updates the record and announces the new column.
4. A calendar month with 2,000 events shows 500 and a `+` count, and the query used the index.
5. A gallery shows the first image and falls back to initials for a record with none.

## How to prompt Gemini

```
Read docs/plans/views.md and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency, except the one Johann has approved for this phase.
A view never widens access. Never return a hidden row or field.
Do not sort, filter, group, or total records in the browser.
Do not read, copy, or adapt another product's source code.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| Letting editors change views will leak data. | A view only chooses what to show from what the person can already read. Phase 1 test 7 tries to make a view return a hidden row and field. |
| A shared link will put student data on the internet. | Links are off by default, `public` needs a `Public` workspace, sensitive and hidden fields are always removed, tokens expire, and every use is counted and rate-limited. |
| A chart will mislead a screen reader user. | Each has a table alternative, text labels, keyboard navigation, and announced values. |
| Sorting a million rows will freeze the screen. | The server computes every page over an index, and the screen draws only visible rows. |
| A dashboard will become a way around approvals. | A dashboard shows data the person may read. A page widget needs an approved page. |
| Two form evaluators will drift. | The form one is tiny, the server decides at submit, and the vectors are generated from the Rust evaluator. |
