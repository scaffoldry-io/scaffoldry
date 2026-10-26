# Declarative pages with OpenUI Lang — assessment and conditional brief

> **Status:** Assessment. Not in the run order. Phases 1 to 5 wait on the three decisions below. Do not start a phase until Johann has decided.

```
Title:   Assess OpenUI (thesysdev/openui) for Scaffoldry, and the plan to use it if a decision asks for it
Risk:    notable (a new runtime in the page sandbox, and a reversal of "no layout language")
Ask:     Decide whether custom pages gain a third format, a declarative OpenUI Lang program
IfYes:   Phases 1 to 5 run. A reviewer reads what a page shows and changes. The server checks a page when it is proposed, not when it opens
IfNo:    Nothing changes. Agents may still bundle OpenUI into a bundle page today, under the existing checks
Checks:  Measurements below, made 2026-10-26 on lang-core 1.0.1 and react-lang 1.0.1. No platform code changed. No test suite was run
```

## Verdict

Do not add OpenUI to the platform now.

1. Most of OpenUI does not apply. Its main loop is a host that prompts a model and streams the answer into a chat. Scaffoldry runs no model. The chat packages, the server package, the gateway, and the LangChain integration have no place here.
2. It conflicts with two standing decisions. `mcp-apps.md` says "no layout language", and the rules in `README.md` say "do not extend the component catalog". OpenUI Lang is a layout language over a component catalog.
3. It fails ARCHITECTURE section 2.1 as written. It is a company project (Thesys Inc.), seven months old, with one npm maintainer. The language reached 1.0 two weeks ago.

The part that does apply is real. A page written as an OpenUI Lang program can be read by the server. Every query, every write, and every component is visible before the page runs. Today a custom page is JavaScript, and the server has no JavaScript parser (the open ponytail in `mcp-apps.md` phase 6). If Johann wants pages that a reviewer can read, that are accessible by construction, and that are checked when proposed, OpenUI Lang 1.0 is the best format found for that. The conditional brief below is how to adopt it.

Revisit when one of these is true:

- `views.md` phase 5 dashboards have shipped, and agents still write custom pages for layouts the views cannot express.
- Reviewers ask what a page does, and the grant sentence is not enough.
- OpenUI Lang moves to a neutral body, or a second independent implementation of the spec exists.

## Watching for a freeze

`governance/scripts/audit-dependency-health.py` checks every direct dependency against its registry. It names a dependency stale after 365 days without a release, and frozen after 730 days, or when its newest release is deprecated or yanked. It is not a CI gate, because its answer changes with the date. A person runs it on a schedule (the pull request that adds it carries a weekly workflow to copy).

OpenUI is young and has one owner, so it is watched more closely once it is a dependency. Each condition below opens a review. Phase 1 sets the first one in the script.

| Condition | How it is seen |
| --- | --- |
| No release of `@openuidev/lang-core` or `@openuidev/react-lang` in 120 days. From March to October 2026, `lang-core` released about once a week | `STALE_DAYS_FOR` in the script, set in phase 1 |
| An OpenUI package is deprecated on npm | The script reports it frozen |
| A new version of OpenUI Lang breaks stored programs, against spec section 3 | Phase 2 test 3 fails on the upgrade |
| A new version changes license | `audit-licenses.py` fails on the upgrade |
| The repository is archived, or Thesys closes, is sold, or stops publishing the spec | A person. No registry reports it |
| A React major version that `react-lang` does not support within 90 days of its release | The upgrade's own tests fail |

When one trips, the answer is to freeze, not to remove. The vendored file keeps working, because it loads nothing at run time. After phase 4 the parser and the proposal checks are Scaffoldry's own. Only the browser renderer would need a successor. That is one loader branch, and the MIT license allows a fork. Phases 1 to 3 without phase 4 leave more to replace, which is one more reason decision 3 matters.

## What OpenUI is

OpenUI is a set of npm packages under the MIT license. Its core is OpenUI Lang, a line-oriented language a model writes to describe a screen. One statement per line binds a name to a component call, a value, a query, or a mutation. A client parses the program and renders it with a component library the host defines. Queries and mutations call tools the host provides. The spec is three documents in `spec/` with 144 conformance fixture files.

```
root = Stack([title, tbl])
title = Header("Open awards")
awards = Query("records", { table_id: "awards", limit: 50 }, { records: [] })
tbl = Table(["Award", "Amount"], @Each(awards.records, "a", [a.data.award, a.data.amount]))
```

Only two packages matter for Scaffoldry: `@openuidev/lang-core` (parser and runtime, no framework) and `@openuidev/react-lang` (the React renderer). Everything else is out of scope.

## What was measured

Packages were installed from npm with `--ignore-scripts` into a scratch directory. They were bundled with esbuild 0.25.10 in production mode.

| Fact | Value | Source |
| --- | --- | --- |
| License | MIT for every OpenUI package. The renderer pulls in `zod` (MIT), `ci-info` (MIT), and, through devtools, `lucide-react` (ISC). All are on the list | Each installed `package.json` |
| Age | First npm release 2026-03-09. `lang-core` 1.0.0 on 2026-10-12. 29 releases of `lang-core` in seven months | `npm view <pkg> time` |
| Governance | Thesys Inc. One npm maintainer on `lang-core`. The same repository ships the vendor's paid gateway and its cloud telemetry sink | `npm view`, `LICENSE`, `packages/` |
| Renderer size | 549,958 bytes minified, 124,706 gzip, with React left out. `zod` is 454,008 bytes of it. `lang-core` imports all of `zod`, so `zod/mini` does not help | esbuild metafile |
| A whole page as a bundle | 775,236 bytes, 194,266 gzip, with React, ReactDOM, the renderer, and three components | esbuild |
| The built-in component library | About 3.2 MB raw, by its own README. Over the 2 MB bundle limit. Not measured here | `packages/browser-bundle/README.md` |
| Today's page runtime | 435,053 bytes: React, ReactDOM, Sucrase, the bridge, and the loader | `crates/scaffoldry-server/src/ui/` |
| Network in the production renderer | None. No `fetch(`, `XMLHttpRequest`, `WebSocket`, `sendBeacon`, `localStorage`, `eval(`, `Function(`, `import(`, or `window.open`. Five `https://` strings are JSON Schema ids and prompt examples | `grep` of the built file |
| Install telemetry | On by default. `lang-core`'s `postinstall` posts an install event to PostHog through a CloudFront host. Off with `DO_NOT_TRACK=1`, `OPENUI_TELEMETRY_DISABLED=1`, or `--ignore-scripts` | `src/telemetry/shared.ts`, `install.ts` |
| Runtime telemetry | Opt-in, server-side only. A browser never sends it | `src/telemetry/runtime.ts` |
| Development builds | With `NODE_ENV=development`, `react-lang` mounts a devtools widget loaded from `cdn.jsdelivr.net` at the floating tag `@0`. A production build removes it | `react-lang/src/devtoolsBootstrap.ts`, `devtools/src/cdn.ts` |
| MCP | The renderer accepts an MCP client as its tool provider and unwraps `structuredContent` | `lang-core/src/runtime/mcp.ts` |

Not measured: the proof page running inside the real page shell, under its Content Security Policy, in Chromium. Running downloaded code needed a permission this session did not have. Phase 1 test 2 makes that proof from the vendored file.

Four installs here run package scripts today: the web job in `ci.yml` (line 115), the build agent's `npm ci`, `deploy/Dockerfile.web`, and a developer's own `npm install`. Each would send an install event unless the opt-out is set. Only the Rust job installs with `--ignore-scripts`.

## Fit against the rules

| Scaffoldry rule | OpenUI | Fit |
| --- | --- | --- |
| Scaffoldry runs no model | Built around a host that calls a model | Only the language and the renderer apply |
| Foundation-governed dependencies (ARCHITECTURE 2.1) | One company, one maintainer | Fails as written. MIT means a fork is always possible, and the spec is small enough to keep |
| License list | MIT and ISC | Passes |
| No layout language. Do not extend the component catalog | It is a layout language over a catalog | Conflicts. Decision 1 |
| Standards first | MCP Apps (SEP-1865) is the standard for screens inside an AI tool, and Scaffoldry already uses it. OpenUI calls itself an open standard, but no standards body publishes it | OpenUI would run inside an MCP Apps page. It replaces nothing |
| Design provenance | A server-side parser would follow the published spec | Decision 3 |
| Page sandbox | The production renderer opens no connection. The frame's policy would block one anyway | Passes |
| WCAG 2.2 AA (`ux-standards.md`) | Agent-written JSX has no accessibility guarantee. A program can only use components Scaffoldry wrote | A gain, if the library is built to the kit's standard |
| A reviewer approves grants, not code | A program shows its queries, writes, and fields | A gain, but only with the server-side parser in phase 4 |
| Least software | 550 KB of new runtime in the frame | Worth it only with phase 4. Without phase 4 it adds weight and no assurance |

## What already works with no change

Two paths need nothing from Scaffoldry. Neither needs a decision.

1. **A bundle page.** An agent builds one JavaScript file holding `react-lang`, its own components, and its program, and proposes it as a `bundle` page (`mcp-apps.md` phase 6). The measured size, 775 KB, is under the 2 MB limit. The `page_dependencies` check lists the libraries, and their licenses pass. The sandbox, the policy, and the `page_call` grants apply as they do to any bundle. The server learns nothing from the program, because it sees only JavaScript.
2. **An app hosted elsewhere.** An app built with OpenUI signs in as a registered client (`external-apps.md`) and passes its MCP client to the renderer. A query in the program names a Scaffoldry tool, such as `list_records`. It runs as the signed-in person, within the client's approved ceiling.

## Decisions for Johann

| Decision | If yes | If no |
| --- | --- | --- |
| 1. Custom pages may have a declarative format, a program over a component library Scaffoldry writes. This reverses "no layout language" for that format only | Phases 1 to 3 run | Stop here. The two paths above remain |
| 2. `@openuidev/lang-core`, `@openuidev/react-lang`, and `zod` are approved as `apps/web` devDependencies, pinned exactly, vendored like React | Phase 1 runs | Decision 1 needs another renderer. None was found |
| 3. A session may write a Rust parser for OpenUI Lang 1.0 from the published spec and its conformance fixtures, under the design provenance rule | Phase 4 runs. This is where the value is | Stop after phase 3, or do not start. Phases 1 to 3 alone are weight without assurance |

This assessment read OpenUI's source to measure it, because the review asked for that. An implementing session works from the spec, the published API, and the fixtures only.

## Standard search

| Searched | Finding |
| --- | --- |
| MCP Apps extension, SEP-1865, specification 2026-01-26 | The standard for screens in an AI tool. Already adopted in `service/mcp_ui.rs`. It carries a document and says nothing about what is inside it |
| OpenUI Lang 1.0 (Thesys) | Examined here. A published spec with fixtures, MIT, one vendor |
| json-render (Vercel), A2UI (Google) | Named in OpenUI's own comparison. JSON formats from single vendors. Not examined in this session |
| W3C, WHATWG, ISO, IETF | No published standard for a model-written declarative UI was found |
| OpenSSF Scorecard, the `Maintained` check (Linux Foundation) | The published measure of whether a project is maintained. Its API was not reachable from this session's network, so the health script reads release dates from the registries instead. A person may add the Scorecard action to CI later |

## Conditional brief

Run only after decisions 1 and 2. Phase 4 also needs decision 3.

### Out of scope

- `@openuidev/react-ui`, `browser-bundle`, `react-headless`, `server`, `langchain`, `a2ui`, `assistant-ui`, `observability`, `observability-cloud`, `devtools`, the CLI, and OpenUI Cloud or its gateway.
- Streaming. A page is stored, whole, by its hash.
- Incremental editing. A changed page is a new source and a new hash, as now.
- `@ToAssistant` and `@OpenUrl`. The frame cannot open a window, and the web desk has no assistant.
- The Vue, Svelte, and Angular runtimes.
- Any change to the page sandbox, its policy, the bridge contract, or `page_call`.

### Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing.
2. The renderer is vendored. No code in `apps/web/src` imports an OpenUI package.
3. Every install of an OpenUI package runs with the telemetry opt-out set, or with `--ignore-scripts`. No build runs with `NODE_ENV=development` for a vendored file.
4. The library's prop order only grows at the end. A prop is never reordered or removed (spec, overview section 3).
5. A program reaches data only through the bridge, so only through `page_call`. Grants stay the whole of what a page can do.

### Phase 1 — the renderer is vendored and proven in the frame

Before this phase, a person sets `OPENUI_TELEMETRY_DISABLED=1` in four places: the web job in `ci.yml`, `build-agent.yml`, `deploy/Dockerfile.web`, and the dev container. Agents do not edit those files.

Add exact pins of `@openuidev/lang-core`, `@openuidev/react-lang`, and `zod` to `apps/web` `devDependencies`. They stay in the workspace so `audit-licenses.py` reads their transitive licenses from the root `package-lock.json`. The pull request carries the `deps-approved` label.

Add both OpenUI packages to `STALE_DAYS_FOR` in `governance/scripts/audit-dependency-health.py` at 120 days. An agent branch cannot change an existing `audit-*` script (`agent-guard.py`), so a person adds this entry.

`apps/web/scripts/vendor-runtime.mjs` gains one entry, `openui.js`, global `OpenUI`, with `react` and `react/jsx-runtime` external to the shell's `React`. Commit the output under `crates/scaffoldry-server/src/ui/vendor/`. Record its size and the package versions in the session output.

Tests:

1. The vendored `openui.js` contains none of `cdn.jsdelivr`, `posthog`, `cloudfront`, `thesys`, `fetch(`, `XMLHttpRequest`, `WebSocket`, `eval(`, `Function(`, or `import(`.
2. In `scripts/ui`, Chromium opens a document built from `page-shell.html` and the vendored files in `<iframe sandbox="allow-scripts">`. A program with one `Query` renders the rows the host returns. No request leaves the page.
3. `governance/scripts/audit-licenses.py` passes.

### Phase 2 — the Scaffoldry library in the frame

A library written here, in `apps/web/vendor-entry/openui-library.jsx`, built by the vendor script into `ui/vendor/scaffoldry-openui.js`. Its id is `scaffoldry`, version 1.

| Component | Built from |
| --- | --- |
| `Stack`, `Row`, `Header`, `Text`, `Badge` | The kit's spacing, type, and `StatusBadge` |
| `Metric` | `StatChart` |
| `Table` | `DataTable`, with a caption |
| `BarChart`, `LineChart` | `apps/web/src/ui/charts`, hand-written SVG with a table alternative |
| `Form`, `Input`, `Select`, `Button` | `FormField` and the kit's controls, labelled, keyboard-operable |

Tools the library names, each mapped to one bridge method: `records` (`records.list`), `record` (`records.get`), `create_record`, `update_record`, `remove_record`. Any other tool name is an error. `@OpenUrl` and `@ToAssistant` are errors.

The vendor script also writes two files from the library. `library.toJSONSchema()` becomes `governance/schema/openui-library.schema.json`. `generateSystemPrompt()` becomes `crates/scaffoldry-server/src/ui/openui-prompt.txt`. Both are committed, so the server never runs JavaScript.

Tests:

1. Each component renders a fixture program in jsdom, with an accessible name and role.
2. Every chart shows its table alternative.
3. The committed schema is compatible with the schema of the previous commit: no prop removed or reordered.
4. A program calling `@OpenUrl`, or a tool not in the list, renders the error panel and posts nothing.

### Phase 3 — the page format

`PageFormat` gains `OpenUi`, serialized `openui`. A program is at most 64 KB. `validate_manifest` is otherwise unchanged.

`render_page` for an `openui` page leaves Sucrase out and adds `openui.js` and `scaffoldry-openui.js`. The loader gains one branch: it renders the program with the library and a tool provider over `window.scaffoldry.records`. Errors from the renderer go to the existing error panel.

Tests:

1. `get_page` for an `openui` page holds the policy element and the program, holds no Sucrase, and is under 1 MB. Record its size.
2. In jsdom, the loader renders a program with a `Query`. The one request it posts is a `page_call`.
3. A program naming an unknown component renders the error panel.
4. The `page_call` grant tests from `mcp-apps.md` phase 6 pass for an `openui` page.

### Phase 4 — the server reads the program

Needs decision 3.

A parser in `crates/scaffoldry-engine/src/openui.rs`, with no new crate. It parses statements, calls, references, and literals (spec `language.md` sections 1 and 2). It does not evaluate. Copy the spec's grammar fixtures to `crates/scaffoldry-engine/tests/fixtures/openui/1.0/` with a `PROVENANCE.json` naming the commit.

The library requires a `fields` list on every `records` and `record` query, so what a page reads is written in the program.

New proposal check `page_program`, a `fail`:

- the program does not parse;
- it names a component or tool the library does not have;
- it uses `@OpenUrl` or `@ToAssistant`;
- a query or mutation names a table with no grant;
- `create_record` without `create: true`, or `remove_record` without `delete: true`;
- a `fields` entry outside the grant's `read`, or a written field outside `write`.

The change list gains sentences from the program. Example: `Page "Award board" shows a table of Awards: award number, amount, status. It has a form that changes status.`

Tests:

1. Every grammar fixture parses or fails as its expected file says.
2. Each failure case above fails `page_program` and stores no proposal.
3. The sentence for a fixture page names its tables, its fields, and its writes.

### Phase 5 — agents use it

`prompts/list` gains `build_openui_page`. It returns `openui-prompt.txt` with Scaffoldry's rules: name `fields`, propose with `format: "openui"`, and grants come from the program. Add scenarios to `crates/scaffoldry-server/tests/agent_scenarios.rs`: a clean page proposed and approved, a page reading a sensitive field needing a second person, and a page failing `page_program`.

Tests: the new scenarios pass.

## How to prompt a session

```
Read docs/plans/declarative-pages.md, docs/plans/mcp-apps.md phase 6, and docs/plans/README.md.
Confirm that Johann has decided decisions 1 and 2, and decision 3 for phase 4. If not, stop.
Implement phase N only. Write the failing test first and run it.
Install OpenUI packages only with OPENUI_TELEMETRY_DISABLED=1 set, or with --ignore-scripts. Do not read OpenUI's source. Work from its spec, its API docs, and its fixtures.
Do not change the page sandbox, its policy, the bridge contract, or page_call.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| A startup's language will be abandoned or relicensed. | It is MIT, so the vendored version can never be taken back. The language is 1,400 lines of spec with fixtures. After phase 4 Scaffoldry holds its own parser. Only the browser renderer would need replacing, and it sits behind one loader branch. |
| The package phones home. | Install telemetry is on by default, measured above. A person sets the opt-out in every place that installs with scripts before the package arrives (phase 1). The vendored production file was scanned and holds no network call. Phase 1 test 1 keeps it that way. |
| A development build loads code from a CDN. | Only with `NODE_ENV=development`. The vendor script builds in production, and phase 1 test 1 fails on `cdn.jsdelivr`. No code in `apps/web/src` imports OpenUI, so the desk's own dev server never loads it. |
| 550 KB is a lot of new code in a dean's browser. | It runs in the same frame as today's pages, with no origin, no token, and no network. An `openui` page leaves Sucrase out, which offsets 206 KB of it. Phase 3 test 1 measures the whole document against 1 MB. |
| This is the layout language the plans refused. | It is. That is why it is decision 1, and why nothing runs until it is made. The refusal was against Scaffoldry inventing and growing one. This adopts a published one, for one page format, with the custom page left as it is. |
| An agent's program can still do anything the grants allow. | Yes, as any page can. The gain is that the server can read the program. Phase 4 refuses a program that reaches past its grants before anyone approves it, and writes what it shows in sentences. |
| A reviewer still cannot see the layout. | A reviewer can open the proposed app read-only (`proposal-preview.md`). The sentences say what data the page shows and changes, which is what a reviewer decides. |
| Our components will drift from the kit. | The library is built from the kit's own components and the hand-written charts. Phase 2 tests check names, roles, and the table alternatives. |
| Stored pages will break when the library changes. | The spec's rules for stored programs are tested in phase 2 test 3: props are only added at the end, never reordered or removed. |
| Agents will not write the language well. | The system prompt is generated from the library itself, in phase 2. Phase 4 refuses a broken program when it is proposed, with the error, so the agent can correct it before a person is asked. |
