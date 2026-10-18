# The data layer — design note

```
Title:  Data is a layer of its own, with its own API. Datasets keep the labels. Apps and automations are clients.
Risk:   high
Ask:    Approve the direction and answer the open questions. Then phase 1 can start. Nothing here is built.
IfYes:  Every read and write of data goes through one layer that checks the labels on the data and the grants of the person and the app. Apps, pages, automations, and AI agents need no policy review of their own, except where they decide something about data.
IfNo:   Policy stays split. Each app is reviewed on its own, and labels protect fields inside apps. A new app can still leak what an old one protects.
Checks: None yet. This is a design. Each phase names its failing test first.
```

## The decision

These are your words, in order.

1. **The data platform is separate from apps.** It is an API-enabled data layer. Apps, pages, automations, and AI agents are clients of it.
2. **Data is grouped into datasets.** The dataset keeps the labeling correct.
3. **A tagged row is the default.** The row's own tag decides. **A dataset can override.**
4. **AI tools evaluate the data against policy and review the tagging.**
5. **Apps and automations need no policy review, except for decisions that affect data.**
6. **A row generally carries its organization in a column.** When the column is missing or empty, the row takes the dataset's unit.
7. **A steward is a position defined in the system.** It is either inherited from a higher unit or specified on the dataset. Some datasets are owned by a function, such as faculty affairs.

## How I read "override"

I read it this way. Please confirm or correct.

- Every row may carry a tag. The row's tag is what protects it.
- A dataset may carry a label too. It has two settings:
  - A **default**: it applies to rows that carry no tag.
  - A **floor**: it applies to every row, and a row's tag cannot lower it.
- The effective label of a row is the row's tag, plus the dataset's floor, plus the dataset's default if the row has no tag.
- Labels only add protection. Nothing in this design lowers a label by itself. Lowering one is a person's act, written to the ledger with a reason. This is the rule `sensitive-content.md` already sets.

If you meant that a dataset label can also *lower* what a row's tag says, tell me. I would not build that. It lets one setting on one dataset remove protection from thousands of rows, with no row-level record of it.

## What exists today

I read these before writing. Each is checked in the code.

| Fact | Where |
| --- | --- |
| Live rows are in `dataset_records`, keyed by `app_slug`. There is no dataset id. In practice an app's table is the dataset | `0002_workspaces_and_ledger.sql` |
| `published_datasets` is a catalog: a name, a department, a sensitivity level, and sample rows inside a JSON payload. It holds no live rows | `0003_persist_apps_and_datasets.sql`, `routes/datasets.rs` |
| Labels are on fields only, keyed `(app, table, field)`. They can only raise sensitivity. The set is created empty. The code comment says it stays empty "until the label table exists". That table does not exist | `state.rs` `labels`, `engine/lib.rs` `LabelSet` |
| A row has no tag. It has one derived yes/no flag, `dataset_records.is_ferpa_sensitive`. A job sets it true when the row holds a value in a sensitive field, and Cedar's record entity reads it. It cannot say which category applies or who may read | `repository.rs` flag recomputation, `service/records.rs`, `scaffoldry-policy/entities.rs` |
| Sensitivity categories and detectors are settings, with presets for FERPA, PHI, PCI, and PII. Scanning stored records, findings, and review are not built (rows 151 to 153) | `sensitive-content.md`, `state.rs` |
| The record code is 2,122 lines in `service/records.rs`, inside the server crate, mixed with workspace, app, and approval code | `service/records.rs` |
| Row rules narrow what a member reads. They never grant. They are evaluated in Rust and compiled to SQL | `access-rules.md`, `service/scope.rs` |
| The architecture audit checks that the web app imports no database driver and that mutations meet Cedar. It does not check that apps reach data only through one door | `governance/scripts/audit-architecture.py` |

Two of these matter most. **Row tags do not exist.** There is only the derived yes/no flag. And **a dataset is not a stored thing**; an app's table plays that role.

## The model

### Datasets

A dataset is a named, stored group of rows with a schema. It has an owning unit, a steward, and labels. Apps do not own data. An app is **granted** a dataset, with the access it needs: read, write, or both, on which fields.

### Who may read a row

A request carries two identities: the **person** and the **client** (an app, an automation, or an agent). Access needs both.

| Check | Question |
| --- | --- |
| Client grant | Has this app been granted this dataset, for this action? |
| Label requirement | Does this person hold a role or position, at a unit that covers the row, that the row's effective label requires? |
| Row rules | Do the dataset's row rules narrow the person out? (unchanged) |

All three must pass. A client grant never widens what a person may read. A person's rights never widen what a client was granted. This keeps the rule you already have: rules narrow and never grant.

### The row's organization

Each row generally carries the unit it belongs to, in a column. A dataset says which column that is. When a row has no value there, the row belongs to the dataset's owning unit. A dataset with no such column is wholly the owning unit's.

This unit is what a label requirement's reach is measured against. A requirement of `registrar, unit and below` is met by a registrar whose unit is the row's unit or above it. A Physics registrar does not meet it for a row whose column names Chemistry.

How I would build it:
- The column holds a unit id. It is stored as a real indexed column on the row, copied from the field when the row is written, not read out of the JSON each time. Then a label requirement compiles to SQL the way row rules do, and the `million_rows_test` limits can hold. This is a choice for you to veto.
- A value that names no unit is refused when written. It is not silently treated as missing, because that would move the row to the dataset's unit, which may be broader or narrower than the writer meant.
- If a unit is retired later, rows keep their id. The AI review raises a finding for them, and a steward decides. Until then the row stays with the retired unit's place in the tree.
- Importing a spreadsheet maps a column to this field like any other. An import that leaves it blank takes the dataset's unit and says so in the preview.

### Many owners in one row

A new faculty row is the hard case. Rank, track, and tenure review date belong to faculty affairs. Room and furniture belong to space management. Laptop and memory belong to IT. The row belongs to a department, and that department's roles have their own grants.

**My answer: do not put ownership inside cells.** Keep three axes, each small and stored where it is cheap to read:

| Axis | Lives on | Question it answers |
| --- | --- | --- |
| Unit | The row, in one indexed column | Whose row is it? (the row's organization above) |
| Function | The field, in the schema. A dataset has a default function | Who owns this attribute? |
| Category | The field, or the row's tag | How sensitive is it? |

A **function** is a named kind of ownership: faculty affairs, space management, IT. A function has a steward position. The steward is the position defined in the system, taken from the nearest higher unit that has one, or specified on the dataset.

A **grant** has three parts: a role or position, a function, and a reach. The reach is `this unit`, `this unit and below`, or `anywhere`.

**The rule, in one sentence.** A person reads or writes field *f* of row *r* when they hold a grant for the function of *f*, whose reach covers the unit of *r*, and they meet the label requirement of any category on *f* or *r*, and the client was granted the dataset.

In the faculty example:
- The faculty affairs officer holds a grant for `faculty_affairs`, reach `anywhere`. They write rank and tenure date on every row. They see no room and no laptop.
- The Chemistry chair holds a read grant for all three functions, reach `Chemistry and below`. They read the whole Chemistry row and no Physics row.
- IT holds `it_equipment`, `anywhere`. Space management holds `space`, `anywhere`.
- Nobody can read a field in a row whose unit their grants do not cover.

**Why not nested blocks in cells.**
- Governance metadata in every cell multiplies storage and reads.
- The policy would live in the data, which the schema and the audit cannot see. `data-has-a-format` asks for a schema before anything reads it.
- It cannot be indexed, so a million-row list cannot be narrowed in SQL.
- Each write would have to prove it did not change the metadata, and the AI review would have to read every cell.
- A nested value is still fine as a value, such as an address with parts. If its parts have different owners, they are different fields. A **field group** names the fields that share an owner and categories, so a steward tags the group once.

**Two ways to build the same governance.** Either works with the rule above.

| Way | Good for | Cost |
| --- | --- | --- |
| Split by function. Faculty affairs, space, and IT each own a dataset keyed by the person. A faculty profile is an app that joins them | Clear single ownership, audit, and write rights. It fits "apps are clients" | Several datasets. A new hire is three writes, which an automation does from one form |
| Tag fields by function in one dataset | One table that arrives whole, such as an imported HR sheet | Every field needs a tag. A steward must keep them right |

I would **prefer the split** and allow field tags for the shared table. One owner per dataset is the simple default. Field tags are the exception, and the AI review proposes them.

**Performance, by design and not measured.** The person's grants turn into a small map: function to the set of units it covers. Reading a row is one set lookup per field. No JSON is parsed for governance. A list query filters on the row's unit column, so unauthorized rows never leave the database. A person cannot filter, sort, search, or total on a field they cannot read on every row in their scope. That is the rule `access-rules.md` already sets, and it stops reading by inference. The `million_rows_test` limits must hold with all of this on. I have not run it.

**What is deliberately left out.** More axes. Purpose, time, and project all invite their own tags. Each would multiply the grants a steward must keep right. Unit, function, and category are enough to say who owns, who belongs, and what is sensitive. A fourth axis needs its own case.

**Writes.** A write to a field needs the grant for that field's function. Changing a row's unit needs a grant that covers both the old and the new unit, so a row cannot be moved out of reach to hide it.

### Row tags

A tag is a set of category ids. A row's tag is stored with the row. A write may set it. A steward may set it in bulk. The label requirement for a category (who may read it) comes from `position-roles.md` phase 5: a role or position and a reach in the organization.

### The AI review

AI tools do three things. All three produce a **proposal**, not a change.

| Job | What it does |
| --- | --- |
| Evaluate | Reads a dataset against the organization's policy and the standard it maps to, and reports what the tagging should be |
| Tag | Proposes tags for untagged rows, and raises for rows whose content matches a category |
| Review | Compares proposed tags with existing ones and flags gaps and contradictions, for a steward |

This is the finding model in `sensitive-content.md`, with an AI tool in the role of the detector. It never stores matched values. It stores the field, the category, counts, and a few record ids.

**Open question 2** is whether a proposal that only *raises* protection may apply itself and wait for review afterward. See below.

### Apps and automations

Today an app change goes through a proposal and an approver. Under this design:

- An app, a page, a view, or an automation is changed by its owners. No policy approval is needed to change how data is shown or arranged.
- A change is governed when it **decides something about data**. These stay governed:
  - granting an app a dataset, or widening that grant,
  - an automation that writes to a dataset or sends data out,
  - an export, a share link, or an integration that carries labeled data,
  - any change to a dataset's schema, labels, steward, or retention.
- The data layer enforces labels on every read and write, whoever the client is. That is why most app review is no longer needed to protect data.

This is a real loosening of app review. It is safe only if the data layer is the single door. Phase 2 below makes that a test.

### The boundary

I recommend a crate boundary first, not a second service. The doctrine says reach for the smallest thing: a new crate, `scaffoldry-data`, with its own routes under `/api/v1/data/…` and its own tests. It owns the dataset, row, label, and tag code. The server crate calls it. The architecture audit then fails any file outside the crate that reads `dataset_records` or the label tables. A deployable split is a later step, taken if scale or an institution's rules ask for it. The stateless server work already lets it scale out.

## Phases

| Phase | Failing test first | Change |
| --- | --- | --- |
| 1 | An app table is a dataset with an id. A list of datasets returns it, and a record read through the dataset id returns the same row as through the app path | Datasets become stored things with ids. Existing app tables get one each by migration. No behavior changes. The two paths agree |
| 2 | A script that adds a `SELECT` on `dataset_records` outside the data crate fails the audit | The data crate. Records code moves in. The audit enforces the one door |
| 3 | A row tagged `PHI` is not returned to a reader the requirement excludes. A dataset floor raises an untagged row. A write cannot lower a floor. A Chemistry row is read by the Chemistry registrar and not the Physics one. A row with the column empty is read by the dataset's unit's registrar. A row naming no unit is refused | Row tags, the row's unit column, and dataset labels, with the effective-label function from this note. The `is_ferpa_sensitive` flag stays, derived from the effective label, so Cedar keeps working |
| 4 | An app with no grant gets 403. An app with a read grant cannot write. A person outside the label requirement gets nothing through an app that has the grant | App grants. The two-identity check |
| 5 | The AI review writes a finding for a row that matches a category, stores no value, and changes no tag | The review job, using the existing job runner and the existing finding shape |
| 6 | A steward confirms a finding and the tag applies and is a ledger entry with a reason. A removal needs a second person | The review screen and the decisions |
| 7 | An app change that grants a dataset needs approval. A change to a view does not | The approval rules narrowed to decisions about data |

Phases 1 and 2 add no feature. They put the code where the rest stands.

## Prior art

I searched on 2026-10-18. Vendor pages and university policies are marketing or local practice, not standards. Where I rely on what I already know and could not confirm in the search, I say so.

| Source | What it does | What it means here |
| --- | --- | --- |
| Workday | A security group is built from a role and constrained to the organizations where the role is assigned, with or without subordinate organizations. Data is secured by functional domain. A special group is needed when a constrained group is too broad, such as HR partners who must not see the HR organization | Closest match to the grant here: role, function (domain), reach. Its exception groups show that reach needs an **exclusion**, or groups multiply |
| PeopleSoft Campus Solutions | Row security by a department tree. A parent department gives access to the departments below it, and individual departments can be left out. The tree need not equal the org chart. After a permission list changes, an administrator runs a refresh process | Same inheritance and reach. Two lessons: the security tree and the org chart can differ, and precomputed access goes stale |
| Immuta | Subscription policies decide whether a person can use a dataset at all. Data policies decide what they see inside: rows and masked columns, driven by attributes such as role and department. Catalogs such as Collibra and Alation supply metadata that policies use | The same two layers as here: the client or person grant, and the row and column rules |
| Snowflake | Tag a column, and the masking policy attached to the tag applies at once. Automatic classification proposes tags. Row access policies filter rows. Access history audits reads | Policy attaches to the category, not to each field. AI proposes tags as accepted practice. We add a person's confirmation |
| Salesforce | A baseline for records, widened by a role hierarchy and sharing rules. Field-level security is separate from record access | Record access and field access are different questions, as in the unit and function axes here |
| Higher-ed data governance policies | A **trustee** for each data domain sets policy. **Stewards** manage the data day to day. Data is classed as public, sensitive, or restricted | Our function is their domain. Our steward is theirs. Add the trustee, and map each category to a class for reporting |

Not searched or not confirmed: Ellucian Banner, Anthology, Oracle Label Security and virtual private database, data-mesh "federated computational governance" (I know it as a practice, but the search found nothing), and how any of them handle AI agents as clients.

**Has anyone solved it?** The parts, yes. Workday and PeopleSoft solve role, organization, and inheritance. Snowflake and Immuta solve tag-driven policy. Higher-ed policy solves stewardship. I did not find one product that does all of them at one data API that apps, automations, and AI agents must all pass through, with positions as the grant, an AI proposing tags, and OSCAL evidence. I cannot show that none exists.

**Changes this suggests**
1. **Exclusions.** A reach may say `this unit and below, except these units`. Workday needed special groups for this.
2. **Staleness.** Compute a person's covered units from live roles, or key any cache to the existing cache versions. A refresh job that someone must remember is a known failure.
3. **The security tree may differ from the org chart.** Decide whether the unit tree used for reach is the org tree. Joint appointments and centers that span departments are the usual reason they differ.
4. **A row with more than one unit.** Joint appointments need it. The single unit column above does not allow it. This is an open question.
5. **A trustee.** A function has a trustee who sets policy and a steward who runs it. They may be the same position.

## Enterprise objections

| Objection | Answer |
| --- | --- |
| "If apps need no review, anyone can build anything and leak data." | An app can only reach datasets it was granted, and a grant is governed. Labeled data is checked on every read by the data layer, whatever the app. The app cannot bypass it |
| "An AI tool will mislabel data." | It only proposes. A proposal changes nothing until a person confirms it. The one exception under discussion, raising protection only, can only make data safer, and a person can undo it with a reason |
| "A dataset floor will lock out the people who need the data." | The steward sees who a floor reaches before saving, as in `position-roles.md` phase 2b. A platform admin keeps a way in, and every admin read of labeled data is a disclosure |
| "Row tags will be slow at a million rows." | Not measured. The tag set is indexed with the record index, and a label requirement compiles to SQL the way row rules do. It must pass `million_rows_test` before phase 3 is called done |
| "This is a rewrite." | Phases 1 and 2 move code and add ids. They change no behavior and each has its own tests. The new behavior starts at phase 3 |
| "Who decides what the policy says?" | The organization does, in settings and OSCAL. The AI tool reads the policy; it does not write it |

## Open questions

1. **Override.** Is my reading above right: a floor and a default on the dataset, and no lowering?
2. **Self-applying raises.** May an AI proposal that only raises protection apply at once and be reviewed afterward? It protects faster. It also means an AI tool changes who can read data without a person. I lean yes for raising, never for lowering, with a daily review list for the steward.
3. **Decided: a column on the row, and the dataset's unit when it is missing.** Open detail: whether a value that names no unit is refused at write (my lean, above) or accepted and flagged.
4. **Decided: a position defined in the system, inherited from a higher unit or specified on the dataset.** Some datasets are owned by a function. Open detail: where functions are defined and who may add one. I lean to settings, by proposal, like categories.
5. **Still open (the answer was left blank). Does an automation act as a person or as itself?** I lean to itself, as a client with its own grant, and the person who started it as the person.

## Not decided, and not verified

- **Performance.** Nothing here is measured. The indexed unit column is a design choice I have not tried. The `million_rows_test` limits do not move.
- **OSCAL mapping.** The control ids in `position-roles.md` phase 5 are not checked against `oscal-catalog.md`.
- **Existing data.** I have not counted rows or apps. Phase 1's migration must be tested on a copy of real data before it runs on any institution's.
- **Cedar.** The note keeps Cedar for questions about workspaces and apps. I have not worked out which dataset questions, if any, belong in Cedar. The audit says mutations must meet Cedar, and phase 2 must keep that true.
- **The web desk.** Several screens read app routes. They keep working through phase 2 because the paths agree. Moving them to dataset routes is not planned here.

## Prompt for the session

Read this file, `position-roles.md` phase 5, `sensitive-content.md`, and `access-rules.md`. Do not start until the open questions are answered. Phase 1 first. Stop when its tests pass.
