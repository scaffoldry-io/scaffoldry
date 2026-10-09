# OSCAL catalog and coverage — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 has no code dependencies. Phase 2 needs `admin-console.md` phase 5.

A compliance officer asks, "Which of the NIST Moderate controls do we cover, and what is the evidence?" The platform must answer from the real catalog, with real control titles, and show the gaps.

Today a control is a bare id such as `AC-02`, typed by whoever wrote the code. Nothing checks that it exists. Nothing says what it means. The catalog route returns four hand-written controls with `sub-millisecond` in the text. ARCHITECTURE promises "pre-mapped baselines: FERPA, HIPAA, NIST SP 800-171, and CMMC". None of that exists.

## Standard

NIST publishes its controls as OSCAL catalogs and its baselines as OSCAL profiles, in the `usnistgov/oscal-content` repository. The catalogs are US government works. They are data. They are not a software dependency, so the license whitelist does not apply, and the provenance of each file is recorded.

| Source | Use |
| --- | --- |
| NIST SP 800-53 Rev 5 catalog | The control list and prose |
| NIST SP 800-53 Rev 5 Low, Moderate, and High baseline profiles | Which controls a baseline selects |
| NIST SP 800-171 Rev 2 catalog | CUI controls. CMMC Level 2 is built on this revision. Check the revision CMMC uses when building, and say so in the session output |

FERPA and HIPAA are statutes. They are not NIST catalogs and they are not OSCAL baselines. They stay as `@source` annotations on Cedar policies. The documents stop calling them baselines.

## Prerequisite from Johann

Do not download anything in the session. Johann places these files in `governance/catalog/source/`:

- `NIST_SP-800-53_rev5_catalog.json`
- `NIST_SP-800-53_rev5_LOW-baseline_profile.json`
- `NIST_SP-800-53_rev5_MODERATE-baseline_profile.json`
- `NIST_SP-800-53_rev5_HIGH-baseline_profile.json`
- `NIST_SP800-171_rev2_catalog.json`

and writes `governance/catalog/PROVENANCE.json`: for each file, the name, its SHA-256, the upstream `metadata.version`, the source URL, and the date retrieved. If a file or the provenance is missing, stop and say so. Add `governance/catalog/source/` to `.gitignore`. The sources are large and are not committed.

## Out of scope

- Importing assessment results, POA&Ms, or SSPs.
- Profiles other than the three NIST baselines.
- Editing the catalog in the product.
- Parameter values. A control's parameters are shown as their labels.
- Translating FERPA or HIPAA into OSCAL.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency. The script uses the Python standard library. CI already installs `jsonschema`.
3. A control id is shown with its title wherever it appears. A bare id is never the label.

## Phase 1 — a slim, validated catalog

1. `governance/scripts/slim-catalog.py`, standard library only. It reads the files in `governance/catalog/source/` and writes, to `governance/catalog/`: `sp800-53r5.slim.json`, `sp800-171r2.slim.json`, and `baselines.json`.
2. Slim catalog format, with a schema at `governance/schema/catalog-slim.schema.json`:

   | Field | Meaning |
   | --- | --- |
   | `catalog` | `sp800-53r5` or `sp800-171r2` |
   | `version` | The upstream `metadata.version` |
   | `controls[]` | `id` (OSCAL form, `ac-2`, `ac-2.1`), `title`, `family_id`, `family_title`, `parent_id` (for an enhancement), `statement` |

   `baselines.json` holds, for `low`, `moderate`, and `high`, the list of control ids the profile selects. Read the profile files and use the structure they actually have. Say what it is in the session output.

3. Rules for the slimming: skip withdrawn controls, which carry a `status` property of `withdrawn`. Flatten each control's statement parts into plain text. Replace each `{{ insert: param, ... }}` with the parameter's label in square brackets, or its id when it has no label. No `{{` may remain.
4. Add a CI step that validates the three slim files against the schema. Add it beside the OSCAL step in `ci.yml`.
5. In `scaffoldry-core`, add `catalog.rs`: embed the slim files with `include_str!`, parse once, and expose `control(id)`, `title(id)`, `search(query, family, catalog)`, and `baseline(name)`. Add `normalize(raw) -> String` for `AC-02`, `ac-03`, `AC-2(1)`, matching `oscal_control_id` from `admin-console.md` phase 5. There is one implementation. If that function exists, move it here.
6. Record each slim file's size in the session output.

Tests.

1. Run the script. Paste the control counts per catalog.
2. The schema validates all three slim files.
3. `control("ac-2")`, `control("ac-2.1")`, and `control("sc-7")` exist, each with a non-empty title and statement.
4. No statement contains `{{`.
5. `low` is a subset of `moderate`, and `moderate` is a subset of `high`. Every id in each baseline exists in the 800-53 catalog.
6. No withdrawn control is present.
7. `normalize` table test: `AC-02` to `ac-2`, `ac-03` to `ac-3`, `SC-07` to `sc-7`, `AC-2(1)` to `ac-2.1`, `ac-2.1` unchanged.
8. `search("account")` returns `ac-2` first or among the first five.

## Phase 2 — titles, validation, and coverage

Needs phase 1 and `admin-console.md` phase 5. Edits that phase's routes and screen.

1. Validate. `PUT /admin/oscal/requirements/{control_id}` rejects a control id that is in neither catalog: 400 `unknown control`. Normalize the id first.
2. Titles everywhere. Every response that carries a control id also carries `control_title`: requirements, ledger rows (`GET /admin/ledger`), and the overview's recent changes. The web `ControlLabel` renders `AC-2 · Account Management` as text, not a tooltip.
3. Requirements rows carry `catalog`, `family_title`, and `baselines: ["low", "moderate", "high"]` for the baselines that select it.
4. Search. `GET /admin/oscal/catalog?q=&family=&catalog=&baseline=&cursor=&limit=`. Platform Admin or `compliance`. Paged, at most 200 controls. Each row says whether it already has a requirement and its status.
5. Coverage. `GET /admin/oscal/coverage?catalog=&baseline=`. Returns, for the chosen set, the count in each status (`implemented`, `partial`, `planned`, `alternative`, `not-applicable`, and `none`), the same counts per family, and the number of controls with ledger evidence. `GET /admin/oscal/coverage/gaps?...&cursor=` pages the controls with status `none`.
6. Export. The `component-definition` from `admin-console.md` phase 5 uses, as each `control-implementation.source`, the catalog's canonical URL from `PROVENANCE.json`, and includes only controls that exist in a loaded catalog.
7. Delete `GET /governance/oscal` and its hand-written four controls. Update `docs/ARCHITECTURE.md` layer 6 to name the real set: the two NIST catalogs and the three baselines. Remove "FERPA, HIPAA, and CMMC" as pre-mapped baselines. Say that FERPA and HIPAA appear as the source of a policy.

Frontend, from the kit, in the OSCAL tab of `admin/PolicyOscal.tsx`.

- `Add a requirement` opens a drawer with a control search box that shows titles and families. Choosing one fills the id. Free text is not accepted.
- The requirements table shows `ControlLabel`, family, baseline badges, status as a `StatusBadge`, and the evidence count.
- A `Coverage` tab: a baseline and catalog choice, a summary in words and numbers (`112 of 287 controls in Moderate have an implemented requirement. 41 have none.`), a table by family, and the gaps table with an `Add requirement` action per row.
- The ledger table in `admin/Audit.tsx` shows `ControlLabel` in its control column.

Tests.

1. A requirement for `zz-99` is 400. `AC-02` is accepted and stored as `ac-2`.
2. A requirement row, a ledger row, and the overview each carry the right `control_title` for `ac-2`.
3. Coverage for Moderate with three requirements set to `implemented`, `partial`, and `planned` counts them correctly, and the remaining baseline controls are `none`. The counts sum to the baseline size.
4. The gaps route pages through exactly the `none` controls.
5. The export validates against the OSCAL schema in CI and every `control-id` exists in a loaded catalog.
6. `GET /governance/oscal` is 404.
7. `search` with `baseline=moderate` returns only Moderate controls.
8. Web: choosing a control in the drawer shows its title. The coverage summary text matches the response numbers. The ledger table shows a title beside each id.

## How to prompt Gemini

```
Read docs/plans/oscal-catalog.md and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency. Do not download files.
Stop and report if governance/catalog/source/ or PROVENANCE.json is missing.
Do not show a control id without its title.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| The catalog will go out of date when NIST revises it. | The provenance file records the version and the hash. Updating is one script run and a diff of the slim files. The product never claims a newer revision than the file. |
| FERPA and CMMC are promised in the documents and absent here. | The documents are corrected. FERPA and HIPAA are the source of policies. CMMC Level 2 rests on the 800-171 catalog, which is loaded. |
| A compliance officer will mark a control implemented without evidence. | Status is the officer's claim. Evidence is the ledger count, shown beside it. A control marked implemented with zero evidence is visible in the table. |
| The slim files lose NIST's wording. | They keep the title and the statement text. They lose parameter values and guidance, and say so. The source file and its hash are recorded. |
| The embedded catalog bloats the binary. | The session records the size. The statement text is the bulk. If it is too large, drop `statement` from the embedded copy and keep titles. |
| Auditors want an SSP. | Not in this plan. The component definition is the machine-readable part a GRC tool can import. |
