# The data layer — design note

```
Title:  Data is a layer of its own, with its own API. Datasets keep the labels. Apps and automations are clients.
Risk:   high
Ask:    Approve the direction, the flexibility principle, and the Version 1 cut. Then phase 1 can start. The open questions below are about later parts, so they can wait. Nothing here is built.
IfYes:  Every read and write of data goes through one layer that checks the labels on the data and the grants of the person and the app. Apps, pages, automations, and AI agents need no policy review of their own, except where they decide something about data.
IfNo:   Policy stays split. Each app is reviewed on its own, and labels protect fields inside apps. A new app can still leak what an old one protects.
Checks: None yet. This is a design. Each phase names its failing test first.
```

Approved by Johann on 2026-11-19. Scheduled in `README.md`: data-layer phases 1 and 2 are rows 193 and 194, and MCP guidance phases 2 to 4 are rows 195 to 197.

## The decision

These are your words, in order.

1. **The data platform is separate from apps.** It is an API-enabled data layer. Apps, pages, automations, and AI agents are clients of it.
2. **Data is grouped into datasets.** The dataset keeps the labeling correct.
3. **A tagged row is the default.** The row's own tag decides. **A dataset can override.**
4. **AI tools evaluate the data against policy and review the tagging.**
5. **Apps and automations need no policy review, except for decisions that affect data.**
6. **A row generally carries its organization in a column.** When the column is missing or empty, the row takes the dataset's unit.
7. **A steward is a position defined in the system.** It is either inherited from a higher unit or specified on the dataset. Some datasets are owned by a function, such as faculty affairs.
8. **A row's unit must come from the governed list of units.** A value that is not on the list is rejected. There is no "bad value" state.
9. **Nothing raises or lowers a label by itself.** AI proposes. A person reviews the plain-language evaluation, with its downstream impacts, before anything applies. No AI may lock someone out in the middle of a process.
10. **A steward is a position for a function and a unit.** Stewards start at the top and may be deputized. Lower units may define their own, in a hierarchy that follows the org. Authority is delegated, not overridden. See "Stewards" below.
11. **An automation runs as a system user.** A person approves its grants. It follows the same ownership options as an app: a person, a team, or a position owns changes and receives error alerts.
12. **A single row can be shared.** Where it goes beyond the default permissions, it needs a justification and an expiry, and may need an AI review before a person approves it.
13. **A label change is never instant.** The steward is asked each time. A change runs on a schedule with a communications plan and a window for exceptions, which are recorded as grants. See "Changing a label".

## Built on, and reconciled with, what exists

Written on 2026-10-19 after checking this note against the briefs in the README. The note extends the built core. It does not stand beside it.

**Built and reused**
- **`row_scope` and `field_visibility`** (`access-rules.md`, rows 52 to 57) are the one place row and field access is decided. Audience fields, label requirements, and shares are added there, compiled to the same SQL, and checked by the same equivalence test. No second evaluator.
- **Positions, holders, and approver resolution** (rows 21, 23, 24) are what stewards are.
- **Sensitivity categories and detectors as settings** (row 33) are what label requirements attach to.
- **`member_role`** (row 52) lets a position be a member. A position that grants a role on its own is `access-model.md` phase 4, as a `holder_unit` grant. A first version of that was built here and withdrawn.

**Reused, not built yet**
- **The policy library** (`policy-library.md`, rows 66 to 68, 71) is where this note's policy documents should live, as a new bound-object kind. It already holds the rule that an agent drafts and a person who did not submit it approves.
- **The authority matrix** (`authority.md`, rows 69 to 73) decides who must say yes to a change. A label change, an exception, and a share beyond the default are change types in it.
- **Findings and review** (`sensitive-content.md`, rows 151 to 153) are the AI review's home.
- **The inbox and email** (`notifications.md`, rows 107 to 113) carry every notice and reminder here.
- **Synced datasets** (`connections.md`, rows 135 to 140) are one kind of dataset here.

**Rules in the README that this note must obey**
- A user's schema is JSON. No statement creates a table, a column, or an index for a user's table or field. So `record_keys`, `record_audience`, and the unit column are fixed tables and a fixed column.
- Records are filtered, sorted, counted, and totalled in PostgreSQL. Field masking per row is projection, and filtering stays in SQL.
- A rule that grants nothing and is never evaluated does not exist. Every policy document here is evaluated by code, with a validator.
- Admin writes are not MCP tools. An agent proposes. A person confirms in the desk.
- Text from a policy source is data. It never appears in guidance, a prompt, or an error.
- Every change by an administrator carries a reason and is a ledger entry written before the change.

**Two conflicts to settle**
1. **At once or scheduled.** `sensitive-content.md` says a confirmed finding raises a label that "protects the field at once". This note says a label change is never instant. Raising protection is the safe direction, but it takes access from people mid-work. Version 1 offers a steward a **suspend** switch for emergencies. You have not chosen between "at once" and "scheduled" for an ordinary raise.
2. **Rules that grant.** `access-rules.md` says a rule narrows and never grants. Audience fields grant from the row. You accepted this if it performs. It is now recorded in that brief.

**The build agent.** The README's run order drives an automated agent that starts the first open numbered row. After pull request 123 merged, that row is `access-model` phase 1. The `1f-n` rows are labelled, not numbered, so the agent does not see them. They wait until the access model reaches phase 4.

## Alignment with the access model, and decisions made (2026-10-18)

`access-model.md` is on pull requests 123 and 127 and is not on `main` yet. It is the maintainers' design for grants. It covers much of what this note sketched, with a firmer base. This note now defers to it. Where the two disagree, `access-model.md` wins, and this note keeps only what it does not cover. The references below are by name and become links once it merges.

**Vocabulary.** Use the access model's words.

| This note said | Access model says | Notes |
| --- | --- | --- |
| Unit (the row's unit) | A coordinate: a unit in a hierarchy. A row is placed by a field that names a unit | Same thing. Our "unit column" is a placement field |
| Function | Data domain | A domain is an axis. A function steward is the approver for a domain |
| Category and its requirement | Sensitivity, and a grant's clearance | A grant carries a clearance. A category requirement is what a clearance must meet |
| Grant: role or position, function, reach | One row: holder, role, hierarchy, anchor, depth, domain, clearance, validity window, source, reason | Use theirs. Our reach and "unit and below" are anchor and depth |
| Exclusions in a reach | Guards forbid | A guard only forbids. An exclusion is a guard, not a field on the grant |
| Condition (training current) | A clearance with a validity window and a source | The Registrar's training record is a grant of clearance that ends on the expiry date, with source `sync` or `manual` |
| Delegated layers with a ceiling | An Org Unit Admin grants inside their own subtree, never above their own role | Same ceiling rule. Do not add a second mechanism |
| Request access | Phase 13: requests, rules that approve alone, grants with a source of `request` | Reuse. Our catalog adds a new place to start a request, from an entry, before any denial |
| Evaluation page | `proposal-preview.md` and phase 7 impact | Reuse. Our evaluation is that preview, plus notice and exceptions |
| Synthetic samples | Phase 12 masks: a closed list that reduces a real value for a reader not cleared | Complementary. A mask serves someone who has partial access. A synthetic sample serves someone who has none. The catalog shows each field's mask in words |
| System users for automations | `workflows.md`: flows run as a service identity with approved grants | Already settled. Reuse |
| AI drafts, people decide | Phase 14: `draft_access_change`. A widening draft cannot be confirmed by the person whose tool drafted it. Drafts expire in seven days | Adopt as written |

**What this note keeps, because the access model does not cover it**
1. Datasets as stored things with ids, and the crate boundary that makes the data layer the single door.
2. A declared key per dataset, and `record_keys`.
3. The catalog: metadata, synthetic samples, discoverability, and search on the desk and in MCP.
4. Label changes as managed change: a steward decides, an effective date, a notice period, a communications plan, and exceptions recorded as grants.
5. Relationship access (an advisor sees the notes of their advisees) and cross-listing.
6. Row shares: one record to one grantee, with a reason and an expiry.
7. External collaborators, and a key credential for them and for system users.
8. The guidance layer for MCP (`mcp-guidance.md`).

**Decisions.** I made these from what you have told me. Say which to change.

1. **At once or scheduled for an ordinary raise.** Scheduled, by default. The exception is computed: when the evaluation shows that nobody is affected (no grant, no open work, no automation, and no app reads the data), the change applies when the steward confirms, because no one loses anything. If anyone is affected, it is scheduled with a notice. `sensitive-content.md`'s "protects the field at once" is amended to this. For an emergency, version 1 has the **suspend** switch, which changes no label.
2. **The notice period.** The institution sets a minimum for each category class in settings. I propose starting values of 14 days for a restricted category and 7 days for others, for a change that removes access. A steward may lengthen it and may not shorten it below the minimum. The numbers are placeholders for the institution to change.
3. **The expedited path.** Not in version 1. Suspend covers the emergency.
4. **The communications plan.** An AI tool drafts it from the evaluation and a template. The steward edits and approves. This is the same rule as phase 14: AI drafts, a person decides.
5. **The build order.** The run order follows your handoff. `access-model.md` comes first, because it is the foundation the data layer sits on. The data layer starts after the grant shape exists (access-model phase 4) and the explanation exists (phase 7). The catalog's request step is access-model phase 13 with a new starting point.
6. **The policy library.** The data layer's policy documents live in it, as a bound-object kind, `data_policy`. Check at the start of the catalog work.
7. **Relationship access and cross-listing.** Express them as placement fields in extra hierarchies, which the access model says are data. Cross-listing is a second `units` hierarchy whose placement field names the other unit. An advisor is a `people` hierarchy whose placement field names the advisor. This uses the access model's own mechanism and adds no per-record table. **I am fairly sure but not certain** that the access model's rule "at most one unit in each hierarchy" allows this. The access model's owner should confirm. If it does not, audience fields stay as this note describes them and must pass the performance gate.
8. **Row shares.** A small, expiring grant with a `record` anchor, under a quota in settings. The access model puts "a per-user or per-record table of who can see what" out of scope, so this needs its owner's agreement. It is the exception path, not the main one.
9. **External collaborators.** A principal kind, sponsored by an internal person. Both a registered public key and federated login are supported. A grant to an external collaborator requires an agreement reference and an end date by default. An institution can change the default.
10. **No capacity switch,** and the **peer** rule is parked. A student worker is a position with a student-eligible clearance requirement. This is policy, not code.

## The principle: mechanism, not business rules

You asked that every answer lean toward flexibility, not toward business rules built into the system. This is the rule for the rest of the note, and where it conflicts with an earlier line, it wins.

- **The platform provides mechanism.** Attributes, grants, labels, dates, approvals, and a decision that can be explained.
- **The institution provides the rules, as data.** Each rule is a document that follows a published, plain schema. No institution's habits are written into the code. "Student-eligible", "needs an agreement", and "peers" are examples of policy an institution writes. They are not features.
- **A closed, small set of operators.** A policy can say "has", "is one of", "is before", "names", and "and / or / not" over attributes. It cannot contain code. A closed set can be understood by a person, checked by a validator, and compiled to indexes. An open language cannot.
- **AI tools help write and explain policy.** The schemas are small and have worked examples. Each policy type has a short guide an AI tool reads. An AI tool can draft a policy from a plain description, show the person what it means in words, test it against sample cases, and explain any single decision ("this person was allowed because of these two grants"). It never applies a policy by itself.
- **Every decision can be explained.** The data layer can answer "why was this allowed or refused" with the rules and facts it used. That serves audit, support, and AI tools.
- **Performance is a budget, not a hope.** Every policy compiles to sets or indexes. A policy has a size limit. The million-row limits hold with real policies switched on. A feature that cannot meet that is held back or narrowed.

What this changes in the note: where I wrote a specific rule (student-eligible positions, peer exclusion, agreement references), read it as an **example policy** the schema must be able to express, not a feature to build.

## Version 1: the smallest thing that is safe

The sections below describe the whole direction. Most of it should wait until a pilot shows it is needed. Each part added early is a part an administrator must learn and a steward must keep right. Version 1 is the smallest set that keeps data safe and the platform manageable.

**Six nouns.** An administrator should be able to hold all of them in their head:

| Noun | Meaning |
| --- | --- |
| Unit | A place in the org tree |
| Dataset | A group of rows. It has an owning unit and an owning function |
| Label | A category on a field or a row, from the settings that already exist |
| Grant | A role or position, for a function, over a reach (this unit and below, with exclusions) |
| Share | One row or a few, to one grantee, with a reason and an expiry |
| Change | A proposal to alter a label, with an evaluation, a date, and a notice |

**In version 1**
- The data layer as the single door (phases 1 and 2). It is the bet that makes everything else cheap, and it adds no feature.
- A row's unit in a column, from the governed list. The dataset's unit when empty. A dataset default for untagged rows.
- Labels on fields and rows, with category requirements. One owning function for each dataset, and a steward position for that function and unit, inherited from the nearest unit above.
- The rule: an app was granted the dataset, and the person holds a grant that covers the row's unit, and meets the label requirement.
- App grants. Automations as system users, with grants that an approver who holds the access signs off. They may only write to datasets they were explicitly granted, and cannot create structure.
- Row shares with a reason and an expiry, approved by the steward.
- Audience fields (with optional acceptance by the receiving unit), and conditions on a requirement, as walked in the scenarios. Both carry dates.
- A label change as a proposal: the plain evaluation page, a steward's decision, an effective date, and one notice period set in settings. The notice goes through the inbox.
- Delegation by date for a steward, using the approvers delegation that is already planned.

**Not in version 1.** Add each only when a pilot shows the need.
- Several owning functions inside one dataset. Split by function into separate datasets instead.
- More than one unit on a row. Use row shares.
- Lower units defining their own functions. Version 1 has the top-level functions only. Lower units name stewards.
- AI-drafted communications plans and AI review of share requests. Version 1 uses a template and a person.
- An expedited change path. Version 1 has a **suspend** switch: a steward can stop all reads of a dataset at once, with a reason. It changes no label.
- Run-level tracking of what an automation read (no write down), and the lineage report. The version 1 rules already stop an automation creating structure or writing outside its grants.
- Per-category notice periods, exceptions as a workflow, acknowledgement tracking.
- Anything about override.

**The test I would use.** If a feature needs a new noun, a new screen for administrators, and a new choice for every unit, it waits.

**One default, not many switches.** Where the institution could choose (does rank give access, does a lower unit's steward outrank a higher one), version 1 ships one answer: access comes only from grants, and a locally named steward is the steward there. Switches are what make a platform unwieldy.

## Override is parked

You said override could get very complicated and hard to manage, and you want to think about it more. So this design does not include a dataset floor, and does not let a dataset label change what a row's tag says.

What stays:
- A row's own tag protects it.
- A dataset may carry a **default** that fills in for rows with no tag, the same way a dataset's unit fills in for rows with no unit. A default only fills a gap. It never changes a tag a row already has. Say so if you want even this left out.
- Labels only add protection. Lowering one is a person's act, written to the ledger with a reason.

What is parked: a dataset **floor** that no row tag can go below, and any rule by which a higher unit overrides a lower one. If we need either, it comes back as its own decision with its own failing tests.

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

A row has one **home unit**. It may also be tagged with **more than one unit**, so that each unit sees it. A joint appointment is the case: the row lives in one department and the other department sees it too.

- The home unit owns the row. Its steward decides who writes it.
- Each additional unit makes the row visible to grants that cover that unit, under the same function rules as the home unit. Reading is the default. Writing by an additional unit is a separate setting, off by default.
- Tagging an additional unit widens access, so it is a governed change. The home unit's steward approves it. It is standing, not time-limited, because joint appointments last. It is **reviewed on a cycle** that the institution sets, instead of expiring. A row share is different: it is one-off and expires.
- The unit tags are stored beside the home unit column and indexed, so lists filter in SQL. I have not measured this.

This unit is what a label requirement's reach is measured against. A requirement of `registrar, unit and below` is met by a registrar whose unit is the row's unit or above it. A Physics registrar does not meet it for a row whose column names Chemistry.

How I would build it:
- The column holds a unit id. It is stored as a real indexed column on the row, copied from the field when the row is written, not read out of the JSON each time. Then a label requirement compiles to SQL the way row rules do, and the `million_rows_test` limits can hold. This is a choice for you to veto.
- The column draws from the governed list of units, so a value that is not on the list is rejected, on a write, an import, or an API call. It is never treated as missing, because that would move the row to the dataset's unit, which may be broader or narrower than the writer meant. The error names the nearest valid units.
- If a unit is retired later, rows keep their id. The review raises a finding for them, and a steward decides. Until then the row stays with the retired unit's place in the tree.
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

A **grant** has three parts: a role or position, a function, and a reach. The reach is `this unit`, `this unit and below`, or `anywhere`, and it may list **exclusions**: units it does not cover. The reach follows the org tree.

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

AI tools do three things. All three produce a **proposal**. None changes a label, a tag, or anyone's access.

| Job | What it does |
| --- | --- |
| Evaluate | Reads a dataset against the organization's policy and the standard it maps to, and reports what the tagging should be |
| Tag | Proposes tags for untagged rows, and proposes raises for rows whose content matches a category |
| Review | Compares proposed tags with existing ones and flags gaps and contradictions, for a steward |

This is the finding model in `sensitive-content.md`, with an AI tool in the role of the detector. It never stores matched values. It stores the field, the category, counts, and a few record ids.

**No automatic raising or lowering.** A person applies every change after reading the evaluation. This is also how the design keeps an AI tool from locking someone out in the middle of a process.

### The evaluation a person reads

It must be plain and complete, because a steward decides from it. One page for each proposed change:

| Part | It says |
| --- | --- |
| What changes | Which rows and fields, how many, and the category. A few examples with values hidden |
| Why | The policy and standard it comes from, and what in the data triggered it, in a sentence |
| Who gains and loses access | By position and unit, with counts. Not a list of ids |
| What depends on it | Open processes and tasks, automations, apps, row shares, and exports that read this data, each named and linked |
| Who would be locked out of unfinished work | People and system users with open work that the change would stop |
| Options | Apply now. Apply on a date. Apply for new work only, and leave open work as it is until it ends |
| How to undo it | The exact step, and who may take it |

The "locked out of unfinished work" line is the one to get right. It is shown first when it is not empty. The steward is asked each time how to treat open work. There is no default that decides for them.

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

### Automations are system users

An automation does not borrow a person. It runs as a **system user**: an identity of its own with its own grants.

- **Grants.** Each grant names a dataset, the action, and the fields or functions it covers. A person approves the grant, and the approval is a ledger entry with a reason. I lean to a rule that the approver must hold the access they are granting. Nobody can grant more than they have.
- **Ownership.** The same options as an app: a person, a team (group), or a position owns it. The owner makes changes to it and receives its error alerts. If the owner is a position, the alert follows the seat.
- **Rules.** Everything in "Who may read a row" applies, with the system user as the person and the automation as the client.
- **Review of change.** Changing what an automation reads or writes is a decision about data and is governed. Changing its schedule, name, or notes is not.
- **Expiry.** A grant should be reviewed on a date the approver chooses. An automation whose grant lapses stops and alerts its owner. It does not fail silently.

This settles the earlier question: an automation acts as itself.

### Stewards

A steward is a **position for a function at a unit**. For example, the registrar function at the university, or the faculty-affairs function in a college.

- Stewards start at the top of the organization. A top-level steward may **deputize** a position or a person for a scope and a period. A deputy's authority is never more than the steward's, and each deputation is a ledger entry.
- A lower unit may **define its own** steward for a function. The stewards follow a hierarchy much like the org's.
- If a unit defines no steward for a function, the nearest one above it serves. This answers only "who is the steward here when nobody is named here". It does not make the higher steward the owner of the lower unit's rows.

**Delegation, not override.** Some units fear the provost overriding a local school official. This design answers that in four rules:

1. **Hierarchy gives no access by itself.** A higher steward reads and acts on a lower unit's rows only through an explicit grant, the same as anyone. "This unit and below" is something a grant says. It is not a right of rank.
2. **A locally defined steward is the steward for that unit.** The higher steward does not act there. Where a higher steward must act, it is by a named grant or by a row share with a justification and an expiry.
3. **Every use of cross-unit access is visible to the local steward.** They get a report of who read or changed their unit's rows from above, and why.
4. **Disagreements are escalated, not overridden.** Where a local steward and a higher one disagree, the question goes to a named position for a decision, and the decision is a ledger entry.

Policy that applies to the whole institution, such as the categories for FERPA or PHI and who may read them, is set by the institution's own stewards at the top. It is not a back door into a unit's data. It only decides what a category means everywhere.

These rules keep the model simple: access comes only from grants and shares, never from rank. I have not tested them on a real institution. This is the part most likely to meet resistance.

### Where functions come from

A **function** is a kind of ownership: registrar, faculty affairs, IT. It is defined at a unit and applies at and below that unit.

- The platform administrator defines the top-level functions, by proposal.
- A lower unit may define a function of its own. Its **local governance function** approves it, by proposal. Data governance is itself a function. Its steward at a unit approves new functions at that unit.
- A lower unit **cannot redefine** a function that exists above it. It may name its own steward for that function, which the stewards rules already allow. This stops two units meaning different things by one name.
- The administrator can see every function defined anywhere. That is visibility, not control.
- A new function starts with no grants. Defining one gives nobody access.

### Moving data around governance

Your concern is real. An automation, or a person, can copy labeled data into a new structure that has weaker labels, and the governance does not follow it. No platform stops a person who may read data from retyping it, so this plan cannot be perfect. It works in layers, so that the easy paths are closed and the rest are seen.

| Layer | What it does |
| --- | --- |
| 1. A system user cannot make structure | It cannot create datasets, fields, or tags. A new dataset needs an owner, a function, a unit, and labels before anything may write to it |
| 2. A grant declares its flow | Approval shows what the automation reads, and what it writes. The approver sees "reads PHI, writes a dataset with no PHI protection" before saying yes |
| 3. No write down | During a run, the data layer tracks the categories the run has read. A write to a target whose labels are weaker than those categories is **refused**. The owner and the target's steward are alerted. The remedy is a labeling change through the normal managed path |
| 4. Lineage | Every write by a system user records where the data came from: source dataset and fields, target, the run, and the grant. A report shows the edges |
| 5. The review compares the ends | The AI review flags a target that is labeled more weakly than its sources, and a field whose values match a category but carry no tag. It proposes. A person decides |
| 6. Humans | A person with read access can still copy data out by hand. Exports and downloads are already disclosures. The audit log and the review are the control |

**Refuse, do not re-label.** I chose to refuse the write in layer 3, and not to raise the target's label automatically. You ruled out automatic raising. A refused write can stop a running process, which is the harm you want to avoid. Approval-time checks in layer 2, and a dry run before an automation is switched on, are there to catch it before then. Layer 3 is the backstop.

**Limits.**
- The tracking is per run, not per field, unless a grant declares a field mapping. A run that reads PHI and writes anywhere will be held to PHI. That can be too strict. A declared mapping lets a grant say which source feeds which target, so a run that reads PHI for one field and writes an unrelated field is not blocked.
- The idea is the "no write down" rule of mandatory access control. I know it from the literature and have not searched for how products apply it. I did not check whether Snowflake or others propagate tags this way.
- Detection by content only finds what a detector can name, such as a Social Security number or a card number.

### Sharing a row

Rows are shared across areas in ways nobody can predict. The design lets a person share a single row, or a few, without waiting for a policy change.

A **row share** names the rows, the grantee (a person, position, or unit), what they may do (read, or read and write), and which fields or functions it covers.

| Case | Needs |
| --- | --- |
| The share stays inside what the grantee could already see | Nothing. It is a convenience |
| The share goes beyond the default permissions | A justification and an expiry. A person approves it |
| The share involves a restricted category | Approval by someone authorized for that category. I lean to this: a share can widen the unit or function reach, but it cannot remove a category's requirement unless an authorized steward approves |

- **Who approves.** The steward of the function and unit of the rows. If the requester is that steward, a second person.
- **AI review before approval.** The request goes to an AI review first. It reads the justification against policy and writes a short assessment: whether the reason fits, what it exposes, how long it asks for, and what is unusual. A person still decides. The assessment is attached to the request. The AI does not approve or refuse.
- **Expiry is by its own terms.** A share ends on the date the approver agreed to. That is not an automatic change to a label or a tag. It is the share ending as granted. It writes a ledger entry and tells the owner.
- **Visible.** Anyone who owns the rows can list the live shares on them, and who made each one.

A share adds one more way into the rule in "Who may read a row": a live share that covers the row and field. It is looked up from a small table keyed by row and grantee. List queries add "or the row is shared with me". Measured in the phase 10 note.

### Changing a label

A change to who may read data is a change for everyone who depends on it. It is managed as a change, not applied as a switch.

**The steps.**

| Step | What happens | Who |
| --- | --- | --- |
| 1. Proposed | A person, or the AI review, proposes a change | Anyone with a reason, or the review job |
| 2. Evaluated | The evaluation page is built, with its downstream impacts | The system |
| 3. Decided | The steward reads it and approves, changes, or declines. A removal of protection needs a second person | The steward |
| 4. Scheduled | The steward sets an **effective date** and approves the **communications plan**. Nothing has changed yet | The steward |
| 5. Notice period | The plan runs. Consumers prepare. **Exceptions** can be requested | Everyone affected |
| 6. Effective | On the date, access changes. Anyone with a granted exception keeps access by that grant | The system |
| 7. Closed | A report lists who was affected, who was notified, what exceptions exist, and what broke | The system, to the steward |

Until step 6, nothing about anyone's access changes. The scheduled change is **visible** on the dataset, so a consumer can see it coming.

**The notice period.** The institution sets a minimum for each category class, in settings. The steward may lengthen it and may not shorten it below the minimum. I have not chosen numbers. A restricted category might need a longer minimum than a sensitive one. The period is long enough for a department to adapt a process, change an automation, or ask for an exception.

**Raising and lowering are different.**
- Raising protection takes access away from people. It gets the full notice period.
- Lowering protection gives access to more people. Nobody loses anything, so consumers need less warning. Exposure is the risk, so it needs a second person, and the dataset's stewards and the privacy office are told before it takes effect.

**The communications plan.** The plan is a structured document, with a published schema. The AI review drafts it from the evaluation. The steward edits it and approves it. Nothing is sent until they do.

| Part | It holds |
| --- | --- |
| Audiences | Built from the impact analysis, not typed by hand: the people who lose access, owners of apps, automations, and processes that read the data, owners of system users, stewards of datasets linked to this one, and the privacy and compliance offices |
| What each is told | What changes, when, why, what they may need to do, how to ask for an exception, and who to contact. In plain language, with no record values |
| Timeline | A first notice, reminders as the date nears, a final warning, a notice on the day, and a confirmation afterward. The dates are relative to the effective date |
| Channels | The inbox and email from `notifications.md`. A banner on affected apps and datasets. A structured notice in the API, so an app or an AI agent that reads the data can show it. Changes to a person's own access always reach their inbox |
| Tracking | Who has acknowledged. The steward sees who has not, by unit, and can chase |
| Follow-up | The count of exception requests, questions raised, and complaints, so a steward can see whether the notice worked |

The API notice matters for the clients that are apps and automations. A read can return an `upcoming_changes` block, with the date and what changes. A well-built client can adapt before the date. An automation's owner also gets an alert, because the automation cannot read a notice.

**Exceptions.** During the notice period anyone affected may ask for an exception. It is a request for a grant that continues their access after the date, with a justification and an expiry. It uses the same path as a row share: the steward decides, an AI assessment is attached, and a person approves. An approved exception is **recorded as a grant**. On the effective date, the people without one lose access, and the people with one keep it until it expires. The expiry is on the grant, so nothing is open-ended. A request still open on the date is decided before the date or the person loses access, so the steward gets a list a week before.

**Urgent changes.** Some protection cannot wait for a notice period. A discovered exposure of restricted data is the usual case. I think there must be an **expedited path**. It would need two people, the steward and the institution's security or privacy officer. It would take effect at once, notify the affected people at once, and open the exception window afterward. It would be rare, and every use would be reported. You said a change should not be instant, so this is your call. See the open questions.

**What it depends on.** The inbox and email, `notifications.md` phases 1 and 2, are not built. They come before this. The job runner schedules the steps and exists today.

### The boundary

I recommend a crate boundary first, not a second service. The doctrine says reach for the smallest thing: a new crate, `scaffoldry-data`, with its own routes under `/api/v1/data/…` and its own tests. It owns the dataset, row, label, and tag code. The server crate calls it. The architecture audit then fails any file outside the crate that reads `dataset_records` or the label tables. A deployable split is a later step, taken if scale or an institution's rules ask for it. The stateless server work already lets it scale out.

## Phases

| Phase | Failing test first | Change |
| --- | --- | --- |
| 1 | An app table is a dataset with an id. A list of datasets returns it, and a record read through the dataset id returns the same row as through the app path | Datasets become stored things with ids. Existing app tables get one each by migration. No behavior changes. The two paths agree |
| 2 | A script that adds a `SELECT` on `dataset_records` outside the data crate fails the audit | The data crate. Records code moves in. The audit enforces the one door |
| 2b | Two rows of a dataset with a declared key that normalizes to the same text cannot both be stored, by a write, an import or a sync. The refusal says only "this key is already used" and names no row. A sync that sees a key change keeps the record id and writes a history entry. An import upsert on a declared key never reports "matched multiple" | Keys: a dataset declares one or more key fields and a normalization (trim, and case-fold if declared). A `record_keys (app_slug, table_id, key_text, record_id)` table with a unique constraint, written in the data crate in the same transaction as the record. The steward sets the key by a proposal. Existing duplicates block the declaration and are listed for the steward. `connections.md` syncs and the import upsert match on the declared key |
| 2c | A dataset that names a unit column refuses a write, an import row or an API call whose value is not a unit on the governed list, and the error names the nearest valid units. An import that leaves it blank takes the dataset's unit and the preview says so. The value is copied to an indexed column on the row, and a list filtered by unit uses it in SQL inside the million-row limits | The row's unit column, as "The row's organization" describes. The effective-label rules that read it stay in phase 3 |
| 2d | A catalog search for words, a function, a unit, a category or a field name returns entries the caller may discover, and no hidden dataset. A sample is synthetic from the schema and contains no stored value: a planted value never appears. A small group's count is masked under the threshold. `list_datasets` with `query` returns the same entries over MCP | The catalog: an entry per dataset holding the metadata this note lists, discoverability as policy (everyone, named units or positions, hidden; a field shown or shown as "a restricted field"), schema-synthetic samples only, and search on the desk's Datasets screen. Over MCP, `list_datasets` gains a `query` argument instead of a new tool, because `core` has 128 bytes free (`mcp-toolsets.md`) |
| 2e | Request access on a catalog entry files an `access-model.md` phase 13 request carrying the fields the dataset's request form lists, routes to the steward position, reminds the steward through the inbox when it waits, and shows the requester where it is. The outcome is a ledger entry, and a refusal carries a reason the person can read | Request access from the catalog, using the existing request, routing, inbox and email. The form fields are a dataset setting from a fixed list (reason, project, duration, agreement reference) |
| 3 | A row tagged `PHI` is not returned to a reader the requirement excludes. A dataset floor raises an untagged row. A write cannot lower a floor. A Chemistry row is read by the Chemistry registrar and not the Physics one. A row with the column empty is read by the dataset's unit's registrar. A row naming no unit is refused | Row tags, the row's unit column, and dataset labels, with the effective-label function from this note. The `is_ferpa_sensitive` flag stays, derived from the effective label, so Cedar keeps working |
| 3b | A person named in an audience field reads the row and nobody else does. An unaccepted value grants nothing. A million-row table with audience grants stays inside the million-row limits. A lapsed condition withdraws access with no job running | Audience fields, `record_audience`, and conditions on the person record. The performance test is the gate |
| 4 | An app with no grant gets 403. An app with a read grant cannot write. A person outside the label requirement gets nothing through an app that has the grant | App grants. The two-identity check |
| 5 | The AI review writes a finding for a row that matches a category, stores no value, and changes no tag or access | The review job, using the existing job runner and the existing finding shape |
| 5b | A function defined at the top needs an administrator's proposal. A function defined at a unit is approved by that unit's governance steward. A lower unit cannot redefine a function above it. A new function gives nobody access | Functions as settings |
| 6 | A steward reads the evaluation and decides. A removal of protection needs a second person. Nothing applies by itself. A change that would stop someone's open work shows them first | The evaluation view and the decision |
| 6b | A scheduled change alters no one's access before its date. On the date, those without an exception lose access and those with one keep it. Every audience member on the plan has a recorded notice. A read returns `upcoming_changes`. Withdrawing the change cancels the rest. The notice period cannot be shorter than the minimum | Scheduled changes, the communications plan, the notice period, and exceptions as grants. Needs `notifications.md` phases 1 and 2 |
| 6c | An expedited change needs two people, takes effect at once, notifies at once, and shows in a report | The expedited path, if you want it |
| 7 | An app change that grants a dataset needs approval. A change to a view does not | The approval rules narrowed to decisions about data |
| 8 | A steward position resolves for a function and unit from the nearest unit that defines one. A deputy acts only within the steward's scope. A higher steward has no access to a lower unit's rows without a grant, and the local steward sees the cross-unit read | Stewards as positions, deputies, and the visibility report |
| 9 | An automation reads only what its system user was granted. A grant needs an approver who holds the access. When its grant lapses it stops and its owner is alerted | System users for automations, with the same ownership options as apps |
| 9b | A system user cannot create a dataset or field. A run that reads a PHI field cannot write to a dataset without PHI protection: the write is refused and the owner and the target's steward are alerted. A declared mapping lets an unrelated write through. Every cross-dataset write records a lineage edge | Layers 1 to 4 of "Moving data around governance" |
| 10 | A row share beyond the default needs a justification and an expiry. It carries an AI assessment and a person's approval. It ends on its date and writes a ledger entry | Row shares |

Phases 1 and 2 add no feature. They put the code where the rest stands.

**Phase 1 is built (2026-11-19).** Migration `0114_datasets.sql` adds a `datasets` table of identity only: an id, an app slug, and a table id, unique for the pair. It gave each stored app table a dataset. Every manifest write gives a new table one in the same transaction. `GET /api/v1/data/datasets` lists the datasets of the apps the caller may read. `GET /api/v1/data/datasets/{id}/records/{record_id}` returns the row the app path returns, through the same service function, so row rules and field masks agree. A record in another table of the app is not found through this dataset. The name and fields stay in the manifest. The older `/datasets` catalog is a different thing and is unchanged. No access rule reads the new table yet. Tests: `datasets_phase1_test`.

**Phase 2 is built (2026-11-19), in part.** The crate `scaffoldry-data` exists. It owns the SQL on three tables: `dataset_records`, `record_index`, and `data_labels`. `record_keys` does not exist yet, so the audit guards the name for phase `keys`. Nothing changed for a caller. The crate depends only on `postgres`, `serde_json`, and `uuid`, and every function takes a `&mut` client or transaction and plain types. The server's repository calls it.

- **Moved.** `records`: the page queries (`PAGE_SQL`, `FIRST_PAGE_SQL`), insert, upsert, set data, read-for-update, save, live version, soft delete, and restore. `index`: insert rows, and delete by record, by fields, and by table. `labels`: list, set sensitive, remove, and add category. The callers are `repository.rs`, `repository_index.rs`, `repository_labels.rs`, and `repository_content.rs`. Service-level access logic did not move.
- **Not moved.** About 20 server files still hold SQL on these tables: the filtered and scoped page reads, aggregates, counts, trash, undo, snapshots, import and export, recalculation, purge, holds, and the guest screens. Moving them all is more than one phase, and the filtered reads are built by the same code as the access scope. The audit lists each file with a reason and a "to move" note in `DATA_DOOR_ALLOWED`. The list can only shrink: an entry for a file with no such SQL fails the audit.
- **The audit.** `audit-architecture.py` fails any `.rs` file outside `crates/scaffoldry-data` with SQL text (`FROM`, `JOIN`, `INTO`, `UPDATE`, or `TABLE` before a guarded table) that is not on the list. A new file that adds a `SELECT` on `dataset_records` fails. Comment lines are ignored. Integration tests under `crates/*/tests` are exempt, because they seed rows and read them back to prove behavior. Migrations are `.sql` files and are not scanned. The test is `governance/scripts/test_audit_architecture.py`. It failed first, with no `audit_data_door` function, and passes now.
- **Labels.** The `data_labels` statements moved. The rest of label handling, such as re-flagging the records of a table after a label change, touches `dataset_records` and stays on the list until the record reads move.
- **CI.** The crate is a workspace member, so the test groups and nextest pick it up. It is added to the `cargo clean -p` lists and to the path filter for the million-row proof.
- **Tests.** `cargo test -p scaffoldry-data` runs against temporary tables on the test database. The server's own tests cover the real schema.

**Phase 2b is built (2026-11-26).** A table declares a key in the manifest: `key: { fields: [...], case_fold: bool }`, one to five fields. The key text is each value trimmed, lower-cased when `case_fold` is true, and joined. A row with a blank key field holds no key. The migration `0116_record_keys.sql` adds `record_keys (app_slug, table_id, key_text, record_id)` with a unique constraint, and a `key_spec` column on `datasets` that holds a copy of the manifest key so a write can read it in its own transaction. The data crate owns both: `keys::write` runs in the same transaction as every insert, update, upsert and restore, and `keys::clear` runs with every delete. A trashed row frees its key. A restore takes it again, or is refused in the same words.

- **The refusal.** A second row with a taken key is the unique violation of `uq_record_keys`. It becomes the existing `constraint_violated` error (HTTP 400) with the message `this key is already used`. No record id, no value and no field of the other row is in it, and the writer's own access does not change the words. The key fields are in the manifest, which the writer can read.
- **The steward sets the key by a proposal.** The key is part of the manifest, so it goes through `propose_app_change` and `decide_proposal` as any manifest change does. The change list says "The key of table ... is set to ...". At approval, in the same transaction as the manifest, the dataset's key is declared: the old keys of the table are dropped and every live row is keyed again. If live rows share a key text, the approval is refused with `constraint_violated` and a list of the groups, by record id, up to 200 ids. Nothing is kept, and the proposal stays pending. Taking the key off frees every key. The same step runs for any manifest write, so a dataset app saved by the connection code is keyed the same way.
- **Import.** A mapping with `upsert_key` on a table that declares a key matches by the declared key and ignores the column the file names, so an upsert never meets "matched multiple". Without `upsert_key`, every row creates a record. A row whose key is taken is rejected with `this key is already used`, or stops the import if errors are not accepted. The dry run says the same. Each row of a keyed table is written in a savepoint, so one refusal does not undo the batch.
- **Sync.** Each row keeps the record id made from its source key. A changed key value is an ordinary update of that record: the id stays and the history entry shows the old and new key text. When no record has that id and another record holds the row's key, the row goes to that record, so a source that renumbers its rows and keeps the key keeps each record, and it is not trashed as removed. Two rows of one run that mean the same record fail the run with the same words. The fault names no row.
- **What is not done.** The key is not Unicode-normalized, because that needs a dependency. A key over about 2,600 bytes fails in the index. Import duplicates inside one file show only when the run reaches them, not in the dry run. A scenario where the source's identity and the declared key are the same column cannot tell a rename from a delete and an add.
- **Tests.** `data_layer_keys_test` (seven cases: write, composite and folded keys, delete and restore, proposal over duplicates, bad key, import, sync) and the data crate's `data_door_test`. The audit already guarded `record_keys`, and only the data crate has SQL on it.

**Phase 2c is built (2026-11-26).** A table names its unit column in the manifest: `unit_field: "<field>"`, the name of one of its Unit fields (the field already names its hierarchy). The migration `0117_record_units.sql` adds `dataset_records.unit_id` (a nullable UUID), a partial index `idx_dataset_records_unit (app_slug, table_id, unit_id, created_at, id) WHERE deleted_at IS NULL AND unit_id IS NOT NULL`, a `unit_spec` column on `datasets` that holds a copy of the manifest setting, and the database function `set_record_unit`. The data crate owns it: `units::write` runs in the same transaction as every insert, update, upsert and restore, beside `keys::write`. A table that names no unit column costs one read of `datasets` and nothing else.

- **The governed list.** The units of the field's hierarchy, in `organizations`. `set_record_unit` refuses a value that is not a unit of that hierarchy with a check violation of `unit_not_on_list`. A blank value clears the column: the row is the dataset's own. A value that is not a unit is never read as blank. There is no foreign key, so a unit that is retired later keeps its rows with their id. The finding for those rows is later work, as the design says, and nothing in this phase follows a retirement. Today `organizations` has no retired state for a single unit. A whole hierarchy can be retired, and its units stay in the table.
- **The error names the nearest units.** Every door that knows who is asking (the API, the MCP tools, an import) says `'<value>' is not a unit of the '<hierarchy>' hierarchy. The nearest units you can see are Name (code C, id U); ...`. The rule is in `units::rank`. A unit's distance is the smallest Levenshtein distance, over characters and ignoring case, between the trimmed value and the unit's name, its code, and, when the value is as long as an id, its id. Units are ordered by distance, then name, then id, and the first three are named. There is no cut-off. The units compared are the ones the caller may see, by the rule that already limits the list of units: a Platform Admin sees every unit, a unit administrator sees their unit and those below it, and anyone else sees none. A caller who sees none is told so, and no unit is named to them. A write that reaches the database by a door that does not know who is asking (a sync) is refused in the same words with no unit named.
- **Import.** A row that leaves the column blank takes the unit the dataset sits at in the field's hierarchy (the table's place, as the access rules compute it). The dry run result carries `unit_default` with the field, the unit, the number of blank rows and a sentence, and the rows then hold the unit in their data and in the column. A value that is not a unit is an error for its row, with the nearest units, and follows `accept_errors` as any row error. An API write or a sync that leaves it blank stores no unit.
- **The list.** A list whose filter holds one `Equals` clause on the unit field with a unit id, in an AND with no group, reads the column: `unit_id = $n`, in the page order, through the index. The rest of the filter reads the index of the other fields as before. A list of several units (`In`), an OR, a group, a count, an aggregate and a view window keep reading the field's index (`record_index`) and so need the field marked as indexed on a large table. A multi-unit read from the column is later work.
- **The steward names the column by a proposal.** It is part of the manifest, so it goes through `propose_app_change` and `decide_proposal`, and the change list says "The unit column of table ... is set to ...". At approval, in the same transaction as the manifest, the dataset's `unit_spec` is set and every live row is copied to the column. If a live row holds a value that is not a unit, the approval is refused with `constraint_violated` and the record ids, up to 200, and the proposal stays pending. Taking the column off clears every `unit_id` of the table.
- **What is not done.** The effective-label rules that read the column stay in phase 3, and nothing reads `unit_id` for access yet. The additional unit tags (a joint appointment) are later, with their own governed change and review cycle. The Unit field is the only way to name the column, so a hierarchy of people cannot be one. The schema `app-manifest.schema.json` now knows the `Unit` field type, `hierarchy` and `unit_field`. It still lacks `row_place`.
- **Tests.** `data_layer_unit_column_test` (seven cases: the refusal and the nearest units for an administrator, a unit administrator and an editor who sees none; the column following a write, an update and a blank; the data crate's refusal and a unit of another hierarchy; the import preview, a bad row and an importer who sees no unit; the list on a 12,000-row table with the unit field not indexed, and the statement's plan using `idx_dataset_records_unit`; the proposal over bad stored values; a unit column that is not a Unit field) and the unit tests of `scaffoldry_data::units`. The `million_rows` proof was not changed: its rows have no unit column, so the new partial index holds nothing there. The index was measured apart from it, on a scratch table of 1,000,000 rows of about 400 bytes in 2,000 units of 500 rows each: the first page of 101 rows for one unit was an index scan on `idx_dataset_records_unit` and took 0.6 ms in the database, and a page after a cursor took 0.05 ms. Loading the rows with the index in place took 14 s. Adding a unit case to `million_rows` itself is left for when that proof next runs.

**Phase 2d is built (2026-11-26).** The catalog has one entry for each dataset, on the phase 1 `datasets` table. The migration `0118_catalog.sql` adds `discoverability` (a JSON policy, default `"everyone"`), `restricted_fields`, `steward_position` and `published_id`. The code is `service::catalog`, `repository_catalog` and `scaffoldry_data::catalog`.

- **One catalog, two sources.** An app table is a `datasets` row, as in phase 1. A published dataset (the older `/datasets` catalog and `published_datasets`) now has a row too: its `app_slug` is `~published` (no app can have that name), and its `table_id` and `published_id` are its published id. The migration made rows for the ones that exist, and `upsert_published_dataset` makes one in the same transaction for a new one. `published_datasets` is unchanged and is still the source of the published name, fields and counts, so the connections code and `/admin/datasets` read it as before. An entry's `id` is the published id for a published dataset and the dataset id for a table, so the older routes and tests keep working. `dataset_id` is always the `ds_` id. `GET /datasets` and `GET /datasets/{id}` now return catalog entries. A connection's dataset app (`dataset-faculty`) is also an app table, so a demo dataset can show twice, once for the published description and once for the synced table. Joining them is later work.
- **What an entry holds.** The name, a description (the table's, else the app's), the function (`department`), the unit (the unit whose code or name is the app's `organization_code`, or the text itself when no unit matches), the steward position (null when none is set), fields with type and description (the label, because a manifest has no field description), category labels from the fields' effective sensitivity, freshness (the UTC day of the newest live row, or the publish day for a published dataset), an approximate size, and `used_by_apps` (the other apps that link to it). None of it is a stored value.
- **Discoverability.** `"everyone"`, `"hidden"`, `{"units": [...]}`, `{"positions": [...]}`, or both lists (either is enough). A unit matches a caller who holds a role in that unit or below it. A position matches a holder of it. A Platform Admin and a compliance officer discover every dataset, because they set the policy, and they alone see the policy on an entry. A policy that cannot be read hides the dataset. The gate runs before anything about a dataset is built, so a hidden dataset is in no list, total, facet, search hit or MCP result. `GET /datasets/{id}` for one is the same 404 as for an id that does not exist.
- **Restricted fields.** A dataset may name fields that show as `a restricted field`: no name, type, flags, category or sample column, and they are left out of search.
- **The write.** `PUT /admin/datasets/{id}/discoverability` (Platform Admin or compliance) takes `discoverability`, `restricted_fields`, `steward_position` (a live position key, or null) and a required `reason`. A part left out stays. It appends a `DataLabelChanged` ledger entry with the dataset, the old values and the new, in the same transaction. It is in `ADMIN_ROUTES` and has a two-instance case. No new decision type was added, so the count stays 68.
- **Samples.** Made from each field's type, label and declared options, in `service::catalog::synthetic`, which is never given a row. The manifest's own `sample_records` are not read. Two rows. The test plants a value in a record and in `sample_records` and checks that no response holds it.
- **Small groups.** A count under `catalog.small_group_min` (a new setting in the closed list, 2 to 10,000, default 11) is not given: `record_count` is null and `size.label` is "fewer than 11 rows". Above it the count is rounded to two significant figures. A table is counted up to 100,000 and then reported as "over 100,000 rows", so the catalog never scans a large table. A published dataset's declared count follows the same rule. Facet counts are counts of datasets, not of people, and are not masked.
- **Search.** Words are split on spaces (at most 8, and 200 characters). Each must match the entry's metadata text (name, description, function, unit name and code, app title, category ids and names, field names and labels, steward position). The match is SQL: `ILIKE ALL` over `unnest`, with `%`, `_` and `\` escaped. It never reads a row. The first 50 entries are returned, with the `total` and the `facets` (units, functions, categories) of everything that matched and that the caller may discover.
- **MCP.** `list_datasets` gains `query`: 26 bytes of schema and none of description. It returns the same entries as `GET /datasets`, up to 50. `core` is 26 bytes closer to its limit, so 128 bytes free became 102. The largest agent list is 14,052 bytes (it was 14,026).
- **The desk.** The Datasets screen searches the server a moment after typing stops and says how many matched. It shows unit, steward ("No steward position set" when none), size, freshness, used-by and categories. Its Schema tab shows a restricted field as such, and its Sample tab is labelled as made up. "Use in New Application" is offered for a published dataset only, because an app table cannot be linked by this id. `apiClient.listDatasets()` still returns published datasets only, for the linking code.
- **What is not done.** Request access (row 204). The steward position is stored and shown, and nothing routes to it yet. There is no screen for setting the policy: the route is there. The other two kinds of sample (from the shape, masked real values) stay out, as the design says. Search is not ranked. It lists by name.
- **Tests.** `data_layer_catalog_test` (eight cases: each kind of search, a hidden dataset in no list, count, facet, lookup or MCP result, discoverability by unit and by position, a restricted field, a synthetic sample with planted values, small-group masking and its setting, `list_datasets` with `query` equal to the route, and the write with its ledger entry), the two-instance case for `PUT /api/v1/admin/datasets/{id}/discoverability`, four unit tests in `service::catalog`, and six cases in `dataset-explorer.test.tsx`.

**Phase 2e is built (2026-11-26).** A catalog entry has a "Request access" action. It files an `access-model.md` phase 13 request. It adds no new request, routing, inbox or email code. The migration `0119_catalog_requests.sql` adds `datasets.request_fields` and four columns on `access_requests`: `dataset_id`, `answers`, `reminded_at` and `decision_note`.

- **The target.** A request with `object_kind: "dataset"` and `object_id` set to the dataset id is stored as a request for the dataset's app, with `dataset_id` set. The grant is the ordinary phase 13 grant: `source = request`, anchored on the app, dated, and expiring by itself. The request names the dataset in its `object` sentence. A published dataset (`~published`, no app) holds no rows to grant, so the request is refused with a 400 that says so and says to ask its steward or a Platform Admin. No `dataset` object kind was added to the table's check, so every read of `object_kind` still sees `app` or `workspace`.
- **The form.** `request_fields` is a dataset setting, a subset of `reason`, `project`, `duration`, `agreement_reference`, default `["reason"]`. It is written by the 2d route, `PUT /admin/datasets/{id}/discoverability`, as a new field beside `steward_position`. It is kept in the form's order, each once, and a name outside the list is a 400. It is in the ledger entry, in `from` and `to`. Entries show it to everyone who may discover them, because the form needs it. A request that leaves out a listed field is a 400 that names each one (`project is required`). A field the form does not list, `project` or `agreement_reference`, is refused by name. `duration` is the `days` of the request, and `reason` is its `reason`. With no `reason` listed the server writes one that names the dataset. With no `duration` listed the request is for 30 days, or the limit if lower. The answers are kept in `answers`.
- **Discovery.** The caller must be able to discover the dataset. For one they cannot, and for one that does not exist, the answer is the same 404 as 2d, and nothing is stored.
- **Routing.** The request goes to the holders of the dataset's `steward_position`. They are found by the existing approver resolver (`approvers::resolved_approvers`) for a position, from the dataset's unit and walking up, so a delegate counts and a vacancy is a vacancy. The dataset's unit is the unit its app names by code or name, else its workspace's unit. The requester is never one of them. Only those holders can decide the request: a Platform Admin who is not a holder cannot. The rules that approve alone still run first, as phase 13 says, so a request that fits the default rule is approved at once and goes to nobody.
- **Fallback.** With no steward position, with a position that nobody holds, or with a dataset at no unit, the request goes by the existing routing (`authority::required`, the workspace's approvers). The status says so in `routing`: `{ "to": "approver", "fallback": true, "sentence": "Nobody holds the steward position 'Dean' for this dataset now, so the access rules route this request" }`. A steward routing reads `{ "to": "steward", "position", "position_name", "holders", "sentence": "Waiting for Department chair: ..." }`. `waiting_on` names the same people. `GET /me/access/requests` carries both.
- **The inbox and email.** A new notice kind, `access_request_waiting` (`by`, `dataset`, `reminder`, with a new `DatasetRef` that renders the dataset's name). It is sent through `notifications::notify` to the people the request waits for, when it first waits (on filing, or when the person confirms a request an AI tool filed). It carries no app, so a steward who cannot read the dataset's app is still told. It follows each person's channel, so mail goes as for any kind. It is a forced kind: it cannot be turned off below the inbox. The notices count rose from 17 to 18.
- **The reminder.** The job `remind_requests`, hourly, reminds the people a pending catalog request waits for once for each 3 days it waits (`reminded_at`, claimed in one `UPDATE ... RETURNING` with `SKIP LOCKED`, so two workers never remind for one wait). It decides nothing. A request that is decided, withdrawn or rejected is not reminded. The 3 days are a constant, not a setting.
- **Where it is, and the outcome.** The requester reads the request's `status`, `routing`, `waiting_on` and `object`. A decision now keeps the decider's reason on the request (`decision_note`), for an approval, a refusal and a withdrawal. A refusal needs one already (`note` is required), so a refusal carries a reason the person can read. The ledger entry is the existing `AccessRequestDecided`, whose payload now holds `dataset` and `answers`. No decision type was added, so the count stays 68. The filing itself is not a ledger entry, as in phase 13.
- **MCP.** `request_access` takes `object_kind: "dataset"` with a dataset id. Its schema grew by `project` and `agreement_reference` and by the words "app, workspace or dataset": 77 bytes. `core` went from 4,898 to 4,975 of 5,000 bytes, so 25 are free. The largest agent list is 14,129 bytes (it was 14,052). No limit was raised. `explain: true` maps a dataset the same way.
- **The desk.** The Datasets screen has a "Request access" button on an entry. It opens a form with the dataset's fields (`RequestAccessDrawer`), names a missing field beside it, and then shows the request's status and who has it. A refusal's words are shown as the server sent them. The schema `access-request.schema.json` has a `dataset_request` definition.
- **What is not done.** A requester told of the outcome by the inbox (phase 13 item 7 reads "when notifications phase 1 has run", and it has run, but phase 13 itself never sent it). A function steward and a unit steward both asked, which is phase 8. The AI assessment of a request. A request for one table of an app: the grant is on the app. A screen to set `request_fields`: the route is there.
- **Tests.** `data_layer_request_access_test` (six cases: the full path with form, steward routing, inbox, email, reminders and a dated grant that works; a refusal with its reason; the fallback and its sentence; the form setting and its 400s; a hidden dataset and a published one; MCP), the `notifications_test` kinds count, the `access-request` schema test, and `request-access.test.tsx`.

**Phase 3 is built (2026-11-27), in part.** A row has tags, a dataset has a floor, a category may carry a requirement, and every read checks the row's effective label against the reader. The migration is `0120_row_labels.sql`. The code is `scaffoldry_engine::effective_label`, `scaffoldry_data::gate`, `service::row_labels` and the `scope` module.

- **The effective label.** `effective_label(floor, tags, fields, categories)` in the engine, next to `effective_sensitivity`, is one pure function. The label of a row is the largest of the dataset floor, the row's tags, and the categories of its fields' effective sensitivities. Categories are a set, so the largest is the union. The row is protected when any field is protected or any category it carries is marked protected in settings. Nothing in it lowers anything. `is_ferpa_sensitive` is `protected` of that label, over the floor, the tags and the fields the row holds a value in, so Cedar reads what it read before. A write sets it from the engine (`submit_record_floored`). A floor change and a label change set it again for the table (`reflag_table`, which keeps a row flagged while it carries a protected tag or its dataset's floor holds a protected category).
- **The requirement.** A category in settings may carry `requires`: a list of `{ position, reach }`, where the reach is `unit`, `unit_and_below` or `anywhere`. Any one is enough. It is validated with the categories (at most 8, no repeats, the position key in the id shape) and published in `sensitivity-settings.schema.json`. A category with none asks nothing, so nothing changes until an institution sets one. A requirement names a position only. A role is not one yet, and a position is the seat `data-layer.md` already uses for stewards.
- **Who may read a row.** A reader meets a category for a row when they hold one of its positions today (a holding outside its dates is not counted) at a unit that covers the row's unit. The row's unit is its unit column (`dataset_records.unit_id`, phase 2c), or the unit its dataset sits at when the column is empty (`catalog::unit_of_app`, the rule 2e routes by). A row with neither is covered by nobody, so it is read only through an `anywhere` reach. A value in the column that is not a unit id takes no unit. A Platform Admin is not held back, as with a row rule. A guest reads only the record they made, as before. The categories a table's fields carry count for every row of the table, whether or not the row holds a value in them. That is the careful side of the rule, and it keeps one fact for the table and not a test for each row. The stored flag Cedar reads still follows the values held.
- **One gate, in two forms.** `service::row_labels::gates_of` builds a `Gate` for a reader and a table: the covers every row needs, and the cover a tagged row needs. A reader who meets every requirement everywhere has no gate. A server whose settings hold no requirement reads settings and does nothing else. The gate has two forms that must agree. `Gate::admits` reads one row's JSON in Rust, and `Gate::sql` reads its columns in a statement. `scope` carries both to every reader. `allows_data` (so `allows`, `readable_ids`, `retain_readable`, and every write check) calls `admits`. `plan_of` puts the gates in `ScopePlan.labels`, and `ScopePlan::clause_at` and `clause_for_page` add them after the row rules, so a list, a count, an aggregate, a window and a link picker never return a row the person may not read. A person who may not read a row may not change it, and a write may not tag a row or move its unit out of the writer's own reach. The test `every_path_gives_the_same_answer_for_every_reader` compares the list with each single read for five readers over 18 rows.
- **How it compiles to SQL.** Placeholders are the app and the table. Every other value in a gate is a unit id or a category id of the allowed shape (checked, never built from text a person sent). A requirement every row carries is `id IN (SELECT lg.id FROM dataset_records lg WHERE app AND table AND live AND lg.unit_id = ANY(units) [OR lg.unit_id IN (SELECT descendant FROM org_closure WHERE ancestor = ANY(units))] [UNION ALL ... lg.unit_id IS NULL])`. It reads the unit index of 2c, and it uses `org_closure` for "and below", so a reach of many units is a subquery and not a long list. The blank-unit part is added only when the cover reaches the dataset's unit, and it is one fact for the table that the server works out from the unit tree. A table with no unit column places every row at the dataset's unit, so the whole requirement is `TRUE` or `FALSE` and no subquery is built. A requirement a tag adds is `NOT EXISTS (tagged rows the reader does not cover)`, read from a partial index that holds only tagged rows, and a null verdict excludes the row (`IS NOT TRUE`). A tag below the floor is accepted and adds nothing.
- **Row tags.** A record's data holds `_tags`, a list of category ids. A trigger (`trg_dataset_records_tags`) copies it to `dataset_records.tags` on every write, through any door, so the copy and the JSON cannot disagree. The same trigger keeps the stored tags when a write replaces the JSON without `_tags`, so a sync, an import or an update that forgets them cannot lower a row. The service checks a write that names them: a list of at most 20 categories that settings define (else 400, naming no value), kept sorted. A tag is added by anyone who may write the row and still reads it afterwards. A tag whose category is protected or carries a requirement is removed only by a Platform Admin or a compliance officer (else 403). A guest sets none. Undo never changes tags, and restore, which names table fields, cannot. A tags-only change is a write and shows in history as `_tags`. This took no new table: the field labels (`data_labels`) are keyed by field, and the findings hold no row, so neither fits a row. The tags are a column of `dataset_records`, which the data crate already owns, so `DATA_DOOR_TABLES` is unchanged. The floor is a column of `datasets`.
- **A write cannot lower a floor.** A floor is not in the row. It is `datasets.label_floor`, and the effective label is a union, so no tag, no missing tag and no write can lower it. A tag below the floor is accepted and adds nothing. The floor is changed only by `PUT /admin/datasets/{id}/label-floor` (a Platform Admin or a compliance officer, a reason of 1 to 500 characters, a ledger entry `DataLabelChanged` with setting `label_floor`, `from` and `to`, in one transaction with the flags). A compliance officer may raise it. Lowering widens who reads, so it is a Platform Admin's. The second person and the notice that "Changing a label" asks for are phase 6, and this route is the governed change until then. It is in `ADMIN_ROUTES` and has a two-instance case. No decision type was added, so the count stays 68.
- **`unit_required`.** A table that names a unit column may set `unit_required: true` in the manifest (the schema knows it, and a table that sets it without a unit column is not a manifest). A write, an import row and a sync that name no unit are refused with `constraint_violated` and the constraint `unit_required`. The import does not take the dataset's unit for a blank row of such a table. The setting goes through a proposal like the column, and the approval is refused while a row names none. It is off by default, so a table that named its column in 2c is unchanged. The data crate refuses it in the write's own transaction (`set_record_unit_checked`). `set_record_unit` stays, and calls it with the flag off, so a server that has not been updated keeps working.
- **Performance.** `million_rows_test` is unchanged. A server with no requirement adds nothing to any statement, and a requirement-free reader has no gate. The compiled forms were measured on a scratch table of 1,000,000 rows of about 400 bytes in 1,980 units with the same indexes (not through the routes, and not in CI): a reader covering 5 divisions (25% of rows) read the first page in 2.7 ms, a page near the end in 1.7 ms and counted in 205 ms. A reader covering one division (5%) took 12.5 ms, 1.9 ms and 306 ms. A reader covering one unit (0.05%) took 2.0 ms, 1.1 ms and 2.5 ms. With 1% of rows tagged, the tag condition took 0.6 ms for a page and 224 ms for a count. Two earlier forms were measured and dropped: a test of each row by its id (`EXISTS ... OFFSET 0`) took 300 ms for a narrow reader's first page and 30 s for a broad count. **Known limit:** a table that names a unit column, does not require it, and is read by a reader whose cover reaches the dataset's own unit adds a scan for the rows with no unit, because the unit index holds only rows that have one. That scan took about 1.5 s on 1,000,000 rows with none. It is fixed by a partial index on unitless rows, which costs every table that names no column, so it waits for a pilot. A table that requires its unit has no such rows and no scan.
- **Not done.** The desk screens for tags, floors and requirements (the routes are there, and settings take `requires` through the existing settings route). A steward's bulk tagging. A second person to lower a floor, and the notice period (phase 6). The access review and `explain` counts, which still count rows by the row rules and the grants, not by the label. The additional unit tags (a joint appointment, phase 3b territory). A row share. A role in a requirement. Position holders listed by the OSCAL props that `position-roles.md` asks for. A category's requirement is stored in settings, and the report for one category (5c) is later. A tag changed by a door that writes `_tags` itself (none exists today: a sync and an import map table fields, and undo and restore leave tags alone) would not meet the removal rule, because the rule is in the service.
- **Tests.** `data_layer_labels_test` (eleven cases: PHI tag, the floor, a write against the floor, a tag removed or a row tagged out of reach, Chemistry and Physics, a row with no unit, `unit_required`, a table with no flag, every path for five readers, a category a field carries, and a proposal that requires the unit), `effective_label_test` in the engine (eight), the gate's unit tests in `scaffoldry_data::gate` (eight) and `service::row_labels` (four), the two-instance case for the floor route, and a manifest fixture for `unit_required`.

**Phase 5 is built (2026-11-27), in part.** The review job is `review_records`, on the existing job runner. It writes into the existing findings table. The migration is `0121_review_findings.sql`. The code is `content_review.rs`.

- **What it does.** The job reads one dataset in batches of 1,000, in the order of the paging index, as `scan_records` does. Each batch is one transaction, and the job's checkpoint is written in it, so a killed job resumes after the last batch that counted and counts no row twice. For each row it asks the detector which categories the content of each text and number field signals. It writes one finding for each field and category. The finding holds the field, the category, a match count, a row count and up to five record ids. It never holds a value. The review writes only to `content_findings`. It changes no tag, floor, label, grant or access.
- **Already carried.** A row that carries the category gets no finding. A row carries it when the dataset floor holds it, when its `_tags` hold it, or when the field's own sensitivity holds it. The review does not write a "consistent" note for these rows. A note is one more thing to read and asks for no decision, so the smaller option is nothing. A finding that counted nothing on a later run is withdrawn when it is still open. A decided finding stays.
- **The detector is a plug.** `Detector` is a trait with one method: a field and its text in, categories with match counts out. The first detector, `Patterns`, is the content scan's own detectors. It honors authorized stores, so a match inside a store for its category signals nothing. An AI tool is another detector. The server calls no model.
- **Category to pattern.** There is no new setting. The `detectors` list of each category in `sensitivity.categories` is the mapping, and it was already there. The scan and the review both read it. With the default presets: `pii` is signalled by `us_ssn` (and `email` when it is switched on), and `pci` by `payment_card`. The first category that lists a detector wins, as for the scan. The schema `sensitivity-settings.schema.json` gained a description of that list and no new field.
- **Where findings live.** In `content_findings`, with `source = 'review'` and `detector_id = 'review:<category>'`. That makes one finding for each field and category, apart from any finding of a scan. The migration widens `detector_id` from 64 to 80 characters to hold it. The findings are listed by the existing `GET /admin/content-findings`. The detector statistics leave review findings out, because they tune detectors.
- **Starting it.** `POST /admin/content-scans` takes `review: true`, with `app_slug` and `table_id`, or `all: true`. It is the same governed route, for a Platform Admin or a compliance officer, with the same two-instance case. A review already waiting for the table is not queued again. No ledger entry is written, as for a scan: the job row names who asked, and no decision type fits a review that decides nothing.
- **Findings respect the gate.** A finding holds record ids, and an id says a row exists. The findings list now checks each finding's sample rows against the reader's phase 3 gate, for every source. A reader with no requirement to meet sees the page as before. A gated reader sees a finding only when they may read at least one of its sample rows, and only those ids. Its counts are withheld (`null`, with `counts_withheld`), because a count says how many rows were left out. The `total` is withheld when anything was left out. Deciding a finding the reader cannot see returns 404. Confirming a review finding returns 409: a confirm would label the whole field, and applying a tag is phase 6. A review finding can be dismissed.
- **Not done.** The agent path. No existing MCP tool takes findings. A new tool is left out because this phase may not change the MCP tool budget. The evaluation page a person reads, and the decision that applies a tag (phase 6). A "raise" finding does not yet say which rows to tag, only a few examples.
- **Performance.** A review of 200,000 rows took 16.3 s in 201 batches on a test database, and the longest single batch took 174 ms. The scan keeps the limit of 2 s for a batch, and the test keeps it. No limit was raised.
- **Tests.** `data_layer_review_test` (six cases: a finding with no value, no access change, a row with no match or the category already, a killed job resumed, a reader who may not read the row, and 200,000 rows).

**Phase 6 is built (2026-11-27), in part.** A steward reads the evaluation and decides. A removal of protection needs two people: the proposer and one approver who is not the proposer. Nothing applies by itself. The migration is `0122_label_changes.sql`. The code is `service::label_changes`, `repository_label_changes.rs`, `routes/label_changes.rs` and `scaffoldry_data::changes`.

- **Where proposals live.** No existing table fit. `access_changes` has one decider and its decision function turns any unknown kind into a grant. So there is one new table, `label_changes`. It holds the dataset, the kind (`tags` or `floor`), the ask (`spec`), the reason, the source (`person` or `review`), the finding, the proposer, the approvals, the choice about open work, and the status (`Pending`, `Approved`, `Applied`, `Declined`). It names rows, so it is behind the data door: `scaffoldry_data::changes` holds every statement on it, and `label_changes` is in `DATA_DOOR_TABLES`. A finding can have one live proposal at a time (a partial unique index).
- **Proposing.** `POST /data/datasets/{id}/label-changes` takes `kind: "tags"` (with `add`, `remove` and 1 to 1,000 `records`) or `kind: "floor"` (with `floor`), and a reason. It checks the categories against settings and that every row is a live row of the dataset. It stores a pending change and applies nothing. It is for the dataset's steward, a Platform Admin or a compliance officer. Confirming a review finding (`POST /admin/content-findings/{id}/decide`) no longer returns 409. In one transaction it confirms the finding, stores a proposal that adds the finding's category to the finding's sample rows, and writes the `ContentFindingDecided` entry with the change id.
- **Who is the steward.** The people who hold the dataset's `steward_position` today, found from the dataset's unit upward. This is the rule request access uses. A Platform Admin or a compliance officer may also decide. A person who is none of these gets 403 for the change and its evaluation, and the same 403 for an id that does not exist, so the answer says nothing about it.
- **The evaluation** is built on request by `GET /data/label-changes/{id}`, and stored nowhere. It has the parts of the table in "The evaluation a person reads": `locked_out`, `what_changes`, `why`, `access`, `depends_on`, `options` and `undo`. `order` lists them, with `locked_out` first when it is not empty. "What changes" gives the rows, the count, the categories and up to five example ids, with no value. "Why" is the reason in a sentence, with the finding if there is one. "What depends on it" names the app, the waiting process instances, the open tasks, the automations, the live share links and the running exports. "Options" are apply now and apply for new work only. Apply on a date is listed as not available, because it is phase 6b. "Undo" is the opposite change, and says how many approvers it takes.
- **Who gains and who loses access** is computed from the phase 3 gate. `row_labels::gate_over` is the gate function with the floor and the readers' positions as arguments, so the evaluation asks the exact question every read asks, of the world as it is and of the world as it would be. The readers are every person who holds a role and every member of the app's workspace, except Platform Admins, who read every row. People who hold the same relevant positions at the same units get the same answer, so they are grouped and each group is asked once. A person counts as gaining when the gate refuses some row now and admits it after, and as losing the other way. The answer gives totals and a count by position and unit, never a name or a row id. **Cost.** One read of every holding. One grouped read of the rows the change touches: for named rows that is a read by key, and for a floor it is one pass over the table that returns a row for each distinct unit and tag list. Then each group of readers is tested against each distinct unit and tag list, twice. It reads no row twice and asks the gate about no person. On a scratch table of 1,000,000 rows of about 400 bytes in 1,980 units, with 1% tagged, the grouped read of the whole table took 213 ms and returned 2,079 groups. The same read for three named rows took 0.9 ms (not through the routes, and not in CI). The rest is the number of groups of readers times the number of groups of rows.
- **Who would be locked out.** A person with open work on a row the change touches: an open task grant, or a waiting process instance they started. They are listed when they read the row now and would not after. The list is checked for each person with the same gate. A task's own grant still opens its one record, so a task holder loses the rest of their reading of the row, and the entry says so.
- **The decision.** `POST /data/label-changes/{id}/decide` takes `approve`, `decline` or `change`, and a reason. Raising protection takes one approval. Removing protection takes two people: the proposer, and one approver who is not the proposer (decided 2026-11-27; it was three people before). Removing protection means any category leaving a floor, or a tag removed whose category is protected, carries a requirement, or is no longer in settings. This replaces the phase 3 rule that a Platform Admin lowers a floor: `PUT /admin/datasets/{id}/label-floor` now raises only, and answers 409 to a lowering. `change` replaces what the proposal asks and takes back every approval. When the lock-out list is not empty, an approval must say `open_work`: `now`, or `new_work_only`. Without it the answer is 400. There is no default. `date` is refused with a sentence that says it is phase 6b.
- **Applying.** The last approval, in one transaction, sets the floor or the tags through the phase 3 code (`labels::set_floor`, then `reflag_table`), writes a history row for each row whose tags changed (`_tags`, old and new), sets the stored flags again, appends one `DataLabelChanged` ledger entry, and sets the status. The entry names the proposer, the deciders, the finding and the open-work choice. No decision type was added, so the count stays 68. With `new_work_only` the change is `Approved` and held. `POST /data/label-changes/{id}/apply` applies it once nobody is locked out any more.
- **Not done.** Apply on a date, the notice period, the communications plan and exceptions (6b). The held change is applied by a person who calls the route, and no job does it when the work ends. A finding names only its sample rows (up to five), so the proposal from a finding tags those rows, and a steward may `change` it to name more. A review that stored every matching row id would fix that. The "what depends on it" part lists no system users, app grants or row shares, because those phases are not built. A decline writes no ledger entry, because no label changed. It is on the change row with its reason and its decider. A proposal is limited to a steward, a Platform Admin or a compliance officer, and not to anyone with a reason. A person who proposes is naming rows, which is a disclosure. A write that removes a protected tag directly is refused as before for everyone but a Platform Admin or a compliance officer. The desk screens for the evaluation and the decision. The access review and `explain` counts.
- **Tests.** `data_layer_decision_test` (nine cases: a proposal changes nothing, the evaluation's counts and the lock-out first, a removal needs an approver who is not the proposer, raising needs one steward, an open-work choice is required, the ledger names everyone, a finding makes a proposal, a protected tag is still refused directly, and a steward may change a proposal), unit tests in `service::label_changes` (two), three cases in the two-instance test (propose, decide, apply) and one changed (the floor route), and `data_layer_labels_test`, which lowers a floor through a proposal now.

**Phase 6b is built (2026-11-27), in part.** A decided change can run on a date. The migration is `0123_label_change_schedule.sql`. The code is `service::label_changes::schedule`, `repository_label_schedule.rs`, `scaffoldry_data::changes` and `row_labels::gate_waiving`. Two schemas are published: `label-change-plan.schema.json`, and `sensitivity-settings.schema.json`, which gained `sensitivity.notice_days`.

- **Scheduling.** `POST /data/label-changes/{id}/decide` takes `effective_at` with `approve`. It stands in for `open_work` (the choice is stored as `date`). The approvals are the same: one to raise protection, two to remove it. The date is checked on every approval. The approval that completes the change sets the date, as it sets the open-work choice today: a completing approver who gives no date applies the change now. When the approvals are complete the status becomes `Scheduled`, the plan is stored, and nothing is applied and nobody is told. The evaluation now lists "Apply on a date" as available, with the minimum days.
- **The notice period.** The minimum is the setting `sensitivity.notice_days`, by class of category. `protected` is a category marked protected, or with a requirement, or one settings no longer define. `other` is any other. A change takes the class of the most protective category it touches. A change that adds a label uses the `raise` period, even if it also removes one. A change that only removes labels uses `lower`. A shorter date is a 400 that names the minimum and the earliest date. A date in the past, or more than 366 days ahead, is a 400. A steward may lengthen the period and cannot shorten it. **Defaults I chose:** `protected` 30 days to raise and 14 to lower. `other` 14 days to raise and 7 to lower. They are in code and apply when settings hold none. The note said I had not chosen numbers. These are a starting point for you to change. The minimum is checked again when the plan is approved, so a plan approved late cannot give less notice than the minimum.
- **The communications plan.** It is stored on the change and follows `label-change-plan.schema.json`: the date, the notice days and the minimum, what changes (category ids), four audiences with a count and what each is told, a timeline, and channels. The audiences are built from the phase 6 evaluation, not typed: the people the gate says lose access (the evaluation's own loop, now returning people), the owners and admins of the app's workspace (read through `service::members`), the stewards, and the people who hold a compliance role. A person in two audiences is on the plan once and is told once. The plan holds counts and wording, never a person and never a value. The steward sees the people by name in `GET /data/label-changes/{id}`, under `schedule`.
- **Approving the plan.** `POST /data/label-changes/{id}/plan/approve`. Until then no notice exists: `label_change_audience` holds the people with `notified_at` empty, and the inbox holds nothing. On approval each person is told through `notifications::notify`: the inbox, and email where the person's preferences allow it. There are two new kinds, `label_change_scheduled` (dataset, days, whether you lose access) and `label_change_withdrawn`. Both are forced into the inbox, because they change a person's own access. Their arguments are references and a count, so no value can enter. Each notice is recorded on the person's row (`notified_at`, `delivered`). A person the inbox does not take is recorded as not delivered. Approving again sends only what is missing, so a crash half way is repaired by calling it again.
- **`upcoming_changes`.** `GET /apps/{slug}/tables/{table}/records` (both paths) adds the block for a reader who has a recorded notice for the table, until the date or a withdrawal. Each entry has `id`, `effective_at`, `days_left`, `adds`, `removes`, a sentence, `you_lose_access`, the reader's own exception, and the route to ask for one. It names no other person and no row. A reader who is not on the plan gets no `upcoming_changes` key at all. One indexed lookup is added to a read of the table.
- **Exceptions.** `POST /data/label-changes/{id}/exceptions` is for a person on the plan who loses access. It needs a justification (1 to 500 characters) and `expires_at` after the date and within `access.request_max_days` of it. It is a 409 before the plan is approved and after the date. Anyone else gets the 404 a missing change gets. The steward decides at `.../exceptions/{exception}/decide`. An approval inserts a grant into `access_grants`: the person, role `viewer`, anchor `label_change:{id}`, source `request`, `valid_until` the expiry, and the justification in its reason. The anchor names the change, which is no workspace, app or table, so the grant reaches no object and adds no role or row. Only the gate waiver keeps the person's read. An exception is refused with a 409 when the person reads nothing of the app today. It writes an `AccessGrantChanged` entry. A decline writes `AccessRequestDecided`. A request nobody decided on the date lapses, in the same transaction that applies the change. Withdrawing the change ends the exceptions and revokes their grants.
- **How an exception reaches the gate.** The gate did not read grants, so it needed a narrow extension. `gate_for` now asks `label_waivers(person, app, table)`: the categories of the person's approved exceptions, where the exception is in force (its change is applied), and its grant is neither revoked nor over. `gate_waiving` leaves those categories out when it works out what the reader must be covered for. Nothing else about the row, the floor or the tags changes. An exception waives only the categories the change added (for a floor, the new floor minus the old). A category the person already met is unchanged. The grant's own dates decide, so a lapsed grant closes the read with no job running. Evaluation, listings, counts, single reads and exports all use the gate, so all honour it.
- **A limit on that, by choice.** A waiver is by category and dataset, not by row. For a change that tags named rows, it would also open rows that already carry the category. So the steward cannot approve such an exception when other live rows already carry the category (the answer is a 409 that says why). A row tagged with it later, while the exception stands, is open to that person until the exception ends. A floor change has no such limit, because every row carries the category.
- **The date.** The job `apply_label_changes` runs every minute. `apply_due(state, now)` takes the time as an argument, so tests move the clock by passing it and never wait. It reads only `Scheduled` changes whose plan was approved and whose date has come, and applies each through the phase 6 `decide_label_change` (the tags or the floor, history rows, flags, status) with one `DataLabelChanged` entry. A change whose plan was never approved does not apply, because nobody was told. It stays `Scheduled` until the steward withdraws it.
- **Withdrawing.** `POST /data/label-changes/{id}/withdraw`. The status becomes `Withdrawn`, so the job finds nothing. Pending and approved exceptions end and their grants are revoked. Everyone who was told is told it is withdrawn, and each of those is recorded too. Scheduling, plan approval and withdrawal each write a `DataLabelChanged` entry with `setting: "label_change_schedule"` and an `event`. No decision type was added, so the count stays 68.
- **Not done.** Reminders on the timeline, a final warning, a notice on the day and a confirmation afterward (the plan lists the first notice and the day only). Acknowledgement tracking and the steward's list of who has not acknowledged. The count of exception requests and questions. The closing report (step 7). A week-before list of open exception requests for the steward. The AI assessment on an exception. A banner on affected apps. An alert to the owner of an automation, because an automation has no owner field yet (phase 9). Owners are the workspace's owners and admins only. `upcoming_changes` is on the table record list, and not on a single record read, on the app-wide list or over MCP. Rescheduling: a steward who needs another date withdraws the change and proposes it again. The urgent path (6c) is left out, as it is an open question.
- **Tests.** `data_layer_schedule_test` (twelve cases, the last proves an exception reaches no other row or table: nothing changes before the date, the date with and without an exception, every person on the plan has a notice and none before approval, `upcoming_changes`, withdrawal, an unapproved plan, the notice minimum and its setting, no record value in a notice, forced kinds, a floor exception, and the exception that would open other rows). `test_label_change_plan_schema.py` (six). A two-instance case, `label_change_scheduled`, covers the four new routes. `data_layer_decision_test` now expects "date" to be available, and `notifications_test` expects 20 kinds.

**Phase 10 is built (2026-11-27), in part.** A person shares rows with another person for a time. A steward approves it. It ends on its date. The migration is `0124_row_shares.sql`. The code is `scaffoldry_data::shares`, `repository_row_shares.rs`, `service::row_shares`, `routes/row_shares.rs`, and `scope::{plan_reading, share_masks, share_cover, reach_of}`. One schema is published: `row-share-assessment.schema.json`.

- **What a share is.** It names rows of one table (1 to 100 record ids), one grantee, what they may do (`read`, or `write`), the fields it covers, a justification (1 to 500 characters) and an expiry. **The grantee is a person.** A position or a unit as grantee is left out: it would turn a share into a standing grant, and the plan calls a share one-off. Each share is for one dataset, so one table.
- **The routes.** `POST /data/datasets/{id}/row-shares` asks. `GET /data/datasets/{id}/row-shares` lists the live shares (`?status=Requested` lists the queue, `?record=` keeps one row). `GET /data/row-shares/{id}` reads one. `POST /data/row-shares/{id}/decide` approves or declines. `POST /data/row-shares/{id}/withdraw` ends a request or a live share early.
- **The three cases.** The server asks the grantee's own rights about every named row, with no share counted: their row rules, their reach, their role for a write, and the phase 3 gate.
  - **Inside what they already do.** Nothing is needed and **nothing is stored**. The answer is a 200 with `stored: false` and `kind: "default"`. This is the smallest correct choice: a stored share that adds nothing would only end, and write a ledger entry for no change. It also means a convenience share cannot outlive the grantee's own rights and keep a row open.
  - **Beyond.** A row their rules or role do not reach. It needs a justification and an expiry (400 without them), and a steward approves. The expiry must be in the future and within 366 days.
  - **Restricted.** A row whose effective label asks something the grantee does not meet. The approver must meet it too: the gate must admit every named row for the approver. A Platform Admin has no gate. The kind is worked out again when the share is approved, so a row that gained a label after the request needs the stricter approval.
- **Who may ask, and who may decide.** The requester must read every named row and field (and, for a write, update it). A row they cannot read is a 404, the same as a row that is not there. The grantee must be able to open the app today: a share gives no seat in an app (409 otherwise). A share to oneself, and to a guest, is refused. The dataset's steward, a Platform Admin or a compliance officer decides. **Nobody decides a share they asked for, and the grantee never decides theirs.** A request that waits is told to the stewards through the existing `access_request_waiting` notice.
- **The assessment.** Stored on the request as `assessment`, from `row-share-assessment.schema.json`. The server calls no model. It holds `reason` (the length of the justification, and `fits: "not_assessed"`), `exposure` (row counts inside, beyond and restricted, the field names, the access, the categories the rows and fields carry, the protected fields), `duration` (days asked, the setting `access.request_max_days`, and whether it is too long), and `unusual` (a closed list: `write_access`, `restricted_category`, `longer_than_setting`, `many_rows`, `covers_every_field`, `protected_field`, `short_justification`). It holds no record value and no row id. A long duration does not refuse the share. `narrative` is a slot, null until an agent or a person fills it. **No stored proposal or assessment pattern exists to reuse** (the guided prompt for shared links is advice with nowhere to keep it), so the slot has no route yet.
- **How a share reaches the read path.** An approved share inserts one row for each named row into `row_share_rows`, keyed by the grantee, app, table and record. The header in `row_shares` holds the status, the fields, the access and the date. Three reads consult it, and no other does.
  - **A list or a count.** `scope::plan_reading` adds `shared` to the scope plan only when the reader holds a live share on that table (one indexed lookup). `ScopePlan::with_share` then turns the whole scope into `(rows and gates) OR the row is in the reader's live lookup`. The lookup is `scaffoldry_data::shares::live_sql`: the grantee, the app and the table must match, the header must be `Approved` with `expires_at > NOW()`, and the share's fields must cover every field the statement reads (`needs`: its filter, its sort, its total and groups, and every field for a search). Both `clause_at` and `clause_for_page` apply it, so a list, a count and a page agree.
  - **A single row.** `fetch_record`, which every single-row route calls, asks `scope::share_cover` only when the reader's own rights do not reach the row.
  - **A write.** `update_record_noted` asks `scope::share_cover` with `write` only when the reader's own rights do not reach the row.
  - **What the reader sees.** A row they read only through a share is cut to the fields the share names, and `created_by` is cleared (`share_masks`, `through_share`). A row they read by their own right keeps every field it had: the share never narrows. The share does not override column rules, masks or read-only fields: they apply as before, because the row is redacted first and cut after.
- **Why it opens nothing else.** The lookup names rows, one grantee and one table, and it is read with the database's clock. The share is not an `access_grants` row and has no anchor on an app or a table, which is what 6b avoided too. It does not pass through the label gate by a waiver, because a waiver is by category and a share is by row. It adds one way in at the one place the verdict is made, after the gate has been built, so a row a share opens is still checked in full for every other reader. These reads do not see a share: the app-wide list, the older offset list (a `sort_field` or a formula filter), link pickers, history, comments, trash, undo, exports, and a process or a button that reads a record. They call `allows` or `plan_of`, which are unchanged. A guest, a view-as, a call through an outside app's ceiling, a preview of a proposal, and a Platform Admin do not count shares. A share does not delete: a delete is never allowed through one. A filter or a sort on a field the share does not name finds nothing in a shared row. The tests show each of these.
- **A write share.** It lets a person whose own rights do not reach the row write the fields it names, and only those. A field it does not name is a 403 for the whole save. The rest of the row is kept as it was. The row's own rules and gate are not asked again for the fields written, because the row stays where it was. A read share allows no write. The write is the person's own, with their own name in the history. A person with the role of viewer may write through a write share, as a task grant lets a person write: the steward approved exactly this.
- **Expiry.** A read asks the date itself. A share closes on its date with no job running, for the list, the count, the single row and the write. The job `end_row_shares` runs every minute. `end_due(state, now)` takes the time as an argument, so a test never waits. For each approved share whose date has come it ends the share, removes its lookup rows and writes one `AccessGrantChanged` entry (`op: "end"`, `reason: "expired"`), in one transaction. It then tells the owners (the person who asked, and the owners and admins of the app) through a new notice, `row_share_ended`. It names the dataset and a count of rows, never a row. It is forced into the inbox. A notice that failed is tried again, because it is recorded only when made. It closes a request nobody decided by its date as `Lapsed`, with no entry, since nothing was granted. The notice kinds are now 21.
- **The ledger.** An approval writes `AccessGrantChanged` (`op: "create"`, `source: "row_share"`). A decline writes `AccessRequestDecided`. An end, by date or by a person, writes `AccessGrantChanged` (`op: "end"`). Filing a request and withdrawing a request that was never approved write nothing, as in phase 13. No decision type was added, so the count stays 68. The payload holds the share id, the dataset, the grantee, the access, a count of rows, the field names, the kind and the date. It holds no row id.
- **Visible.** `GET /data/datasets/{id}/row-shares` lists the live shares on the dataset's rows: who asked, who approved, the grantee, the rows, the fields and the date. It is for the owners and admins of the app's workspace, the dataset's steward, a Platform Admin and a compliance officer. Anyone else gets a 403. The person who asked and the grantee read their own share.
- **Measured (1,000,000 rows, debug build, 20 runs through the routes).** The reader advises 3,003 rows. The share lookup holds 10,000 rows from 100 shares, and the reader holds 100 of them.

  | Read | No share, median / p95 | With shares, median / p95 |
  | --- | --- | --- |
  | First page of 50 | 92.6 / 106.6 ms | 95.8 / 126.0 ms |
  | Count | 119.2 / 145.0 ms | 164.3 / 203.6 ms |

  The page keeps its early stop: the planner hashes the 100 lookup rows once and tests each row it walks against the hash. The count costs about 38% more at the median. I did not study its plan, and it is the part to look at if this needs to be faster. A reader with no live share has no `shared` and runs the statement they had. The cost of finding that out, one indexed lookup, is in the first column. The test is `list_cost_with_shares_on_a_million_rows` and is `#[ignore]`. It asserts a first page under 200 ms and a count under 1,000 ms at p95, which are the existing scoped limits for a page and a grouped count. Those limits did not move. The numbers are from a debug build, which is slower than the release build CI runs, so a release run can only be faster. I have not run it in release mode.
- **Not done.** A position or a unit as grantee. The narrative of the assessment has no route to fill it. The requester is not told when a request is decided, and the grantee is not told when a share opens. A reminder for a request that waits. The end of a share is told to owners, and not to the grantee. A share cannot name a field the requester cannot see, and cannot open a field a column rule hides from the grantee. A quota of live shares per person (the plan mentions one in settings). A share on the older offset list, the app-wide list and exports. Naming rows by a filter instead of by id. A share that renews: the requester asks again.
- **Tests.** `data_layer_row_shares_test` (sixteen cases and one measurement that is ignored: the eight the brief names, a share inside the default that is not stored, a decline, a second approver for a beyond share, naming only live rows of the table, a grantee who cannot open the app, a request that lapses, an early end, and a reader with no share). `data_door_test` has one case for the data crate's functions. `test_row_share_assessment_schema.py` (five). A two-instance case, `row_share`, covers the three write routes. `notifications_test` expects 21 kinds. `access_scope_paths_test` lists `rows_of` among the functions that load records and return none. Three tests that build a `ScopePlan` by hand gained `shared: None`.

**Phase 3b is built (2026-11-27), in part.** A person named in an audience field reads the row, and nobody else does. A condition on the person record can close a category to them, and a lapse takes effect when the row is read. The migration is `0125_record_audience.sql`. The code is `scaffoldry_data::audience`, `repository_audience.rs`, `service::audience`, `routes/audience.rs`, `scope::{plan_reading, principals_of, audience_rules, way_in_masks, share_cover}`, `ScopePlan::{sizing, audience_sql, clause_at_rows}`, `repository_audience::size_audience`, `row_labels::{needs_when, conditions_hold}` and `scaffoldry_engine::access::{to_sql_or, to_sql_probes_or, probe_sets}`. Two schemas gained parts: `app-manifest.schema.json` (`audience` on a table) and `sensitivity-settings.schema.json` (`conditions` on a category, and the setting `sensitivity.conditions`). The schemas were changed first.

- **The policy.** A table in the manifest lists audience fields: `audience: [{ field, grants, fields, unit_acceptance }]`. `field` is a Text, Collaborator or Unit field of the table. `grants` is `read` (the default) or `write`. `fields` are the fields being named reaches, and a person named sees those and no others. A write grant may not list a computed field. A write grant may not list an audience field, its own or another, because a person who could change a name could add anyone. At most 8 audience fields. The dataset keeps a copy in `datasets.audience_spec`, written in the same transaction as the manifest, as `key_spec` and `unit_spec` are. A proposal carries a change, and its change list says so. A new or changed policy rebuilds the rows of the live records in that transaction.
- **What a value names.** A value is one name or a list. A text is split on commas, semicolons and spaces. The shape says what a name is: a bare unit id is a unit, `group:id` is a group, `key@unit-id` is a position, and any other `a@b` is a person. Names are kept in lower case. A row holds at most 100 names in one field, and a name after the last grants nothing. Text that names nobody is ignored.
- **`record_audience`.** Columns: `app_slug`, `table_id`, `record_id`, `field`, `kind`, `principal` and `accepted` (with who accepted and when). The key is `(record_id, field, principal)`. A partial index over accepted rows, `(app_slug, table_id, principal, field, record_id)`, answers "rows naming any of these principals" from the index alone. `specs::write` keeps it current inside the record transaction, with the key and the unit column, so every door writes it: a request, an import and a sync. It reads the policy in the same read of `datasets` that the key and the unit already used, so a table with no policy adds no statement to a write. A name that stays keeps its row and its acceptance. A trashed record keeps its rows, and the read leaves it out by its own table. The purge deletes them. The table is in `DATA_DOOR_TABLES`.
- **Who a reader answers to.** `scope::principals_of`: their address, each position they hold as `key@unit-id`, each unit they sit in and every unit above it, and each group, as one small set for a request. Naming a unit reaches the people at that unit and below.
- **How it reaches the read path.** `plan_reading` adds `audience` to the scope plan only when the table's manifest has an audience field whose policy covers every field the statement reads (`needs`) and the reader's rows are not already every row. It then asks the database two small questions in one statement (`ScopePlan::sizing`, each count has a limit): does the audience name this reader anywhere, and do the rules and the audience together reach few rows (`FEW_ROWS`, 5,000)? A reader the audience names nowhere has no `audience`, so their statement is the one they had. `ScopePlan` then widens the rows' predicate only. When the reach is few, a count and a page read it as one small set: `id IN ((what the rules select) UNION (the audience set))`. When it is many, a count reads two memberships, `id IN (rules) OR id IN (audience)`, and the page keeps its early stop with one more `EXISTS (... OFFSET 0)` probe. The category gates are added after it, with `AND`, so a name never passes a requirement. The share of phase 10 is added after the gates, with `OR`, as before. Both work together. A table with no audience field adds no question: the manifest says so, and the engine holds it.
- **Why it opens nothing else.** The audience set names rows of one app and one table, by principals the caller worked out, and only in the audience fields whose policy covers the statement. A value outside the policy's fields finds nothing, because the field is not in the set. A name in another table, or another app, is not in the set. A unit that waits is not accepted, so it is not in the index. The read of a single row and a write go through `share_cover`, which asks the same question of one row, and then asks the reader's own gate of the row's data. A list or a count adds the audience; these reads do not: the app-wide list, the older offset list, link pickers, history, comments, trash, undo, exports, a process or a button. They call `allows` or `plan_of`, which are unchanged. A guest, a view-as, a call through an outside app's ceiling, a preview of a proposal, and a Platform Admin do not count audiences.
- **What the reader sees.** A row they read only because it names them is cut to the fields of the policy, as a share cuts it (`way_in_masks`), and `created_by` is cleared. A row they read by their own right keeps every field. A write through a write policy names only the policy's fields: any other field is a 403 for the whole save. Naming never lets a person delete a row, and never lets them change an audience field: the manifest refuses a write grant that lists one.
- **Acceptance.** A policy may set `unit_acceptance`. A unit named in that field is stored with `accepted = false`, and nothing reads it. The steward of the receiving unit (the dataset's steward position, held at that unit or above), a Platform Admin or a compliance officer accepts: `POST /data/datasets/{id}/records/{record_id}/audience/accept` with `unit` and `reason`. `GET /data/datasets/{id}/audience/pending` lists what waits, for the units the caller may accept. The acceptance and one `AccessGrantChanged` entry (`op: "create"`, `source: "audience"`, the unit as holder, the access and the fields) commit together. No decision type was added. A second acceptance is a 409. A later save that leaves the unit named keeps the acceptance, and a save that names another unit starts again.
- **Changes are in the history.** A change to an audience field is a change to the row, so `record_history` holds it with the field, the old and new values, and the actor. Nothing new records it. A test shows it.
- **A synced value.** A synced dataset (kind `Dataset`) refuses every write by a person already (`read_only_source`, 409). When the app has an audience field, the message now says the names come from the source and points to a row share: `POST /data/datasets/{id}/row-shares`. The sync writes the audience through the same door, so access follows the source.
- **Conditions.** `persons.conditions` is a map from a condition id to the last day it holds (`YYYY-MM-DD`). `persons.condition_sources` holds the owner, the source (`manual` or `sync`), who set it and when. The closed list is the setting `sensitivity.conditions`: `{ id, name, owner }`, at most 20, where the owner is a position key. A category names conditions in `conditions`, only ids on the list, and a save of either setting checks the other. A category with conditions and no `requires` asks only the conditions. A reader who lacks one is covered by nobody for that category, whatever positions they hold, so the rows that carry it close to them. `PUT /admin/people/{eppn}/conditions/{id}` sets a date or clears it with `until: null`, with an owner (it must be the list's owner) and a source. A Platform Admin or a compliance officer does it. A date already past is recorded as it is. Each change writes one `UserAccessChanged` entry. `GET /admin/people/{eppn}/conditions` reads them.
- **How a lapse is checked.** When settings name a condition, the reader's gate is built from one read of their person: their positions and their conditions together. A check is a date comparison on that map. It runs at read time, in `row_labels::gate_for`. No job runs, and a reader who meets the date is no slower. When settings name no condition, the reader's positions are read as they always were.
- **The statement for a table with no audience.** `plan_reading` does not touch the plan, and `clause_at` and `clause_for_page` build the text they built before. A test compares the two clauses.

- **Measured (1,000,000 rows, 20 runs through the routes, release build, p95, the CI gate).** A fifth of the rows (200,000) name someone in the audience field `reviewer`: one in twenty-five a division of the unit tree (2,000 rows each), the rest one of 1,999 people (about 80 rows each). A row rule narrows each reader to the rows they advise, so the audience is their only other way in. Each reader is timed against the same table with the policy taken out of the running manifest. Each count is checked against a count of the reader's rows made by a statement that shares nothing with the server's.

  | Reader | Operation | No policy, median / p95 | Audience, median / p95 | Limit |
  | --- | --- | --- | --- | --- |
  | Sparse: advises 1,000, named on 2,080 | First page of 100 | 558 / 631 ms | 64 / 80 ms | 100 ms |
  | | Page near the end | 31 / 39 ms | 44 / 51 ms | 100 ms |
  | | Count | 27 / 30 ms | 42 / 50 ms | 1,500 ms |
  | | Count grouped by status | 26 / 29 ms | 40 / 51 ms | 1,500 ms |
  | Dense: advises 250,000, named on 2,080 | First page of 100 | 36 / 42 ms | 49 / 58 ms | 100 ms |
  | | Page near the end | 33 / 37 ms | 45 / 53 ms | 100 ms |
  | | Count | 401 / 464 ms | 489 / 514 ms | 1,500 ms |
  | | Count grouped by status | 429 / 522 ms | 516 / 544 ms | 1,500 ms |
  | Named only: advises nothing, named on 2,081 | First page of 100 | 4,596 / 4,795 ms | 52 / 58 ms | 100 ms |
  | | Page near the end | 29 / 34 ms | 38 / 42 ms | 100 ms |
  | | Count | 23 / 25 ms | 37 / 44 ms | 1,500 ms |
  | | Count grouped by status | 22 / 26 ms | 43 / 50 ms | 1,500 ms |

  This is the run of the whole file as CI runs it (`cargo test --release -p scaffoldry-server --test million_rows_test -- --ignored million_rows --nocapture --test-threads=1`): three tests, 1,685 s, all passing. The first page of the sparse reader is the closest to its limit: 80 ms at p95, and its slowest of twenty runs was 106 ms. An earlier run of the same test gave 73 / 82 ms. The gate is p95, as in `million_rows`.

  The limits did not move. The test is `million_rows_with_audience` in `million_rows_test.rs`. CI runs it with the other two, because its name holds `million_rows`. It also checks that no page or grouped count scans the records, and prints their plans.

  **The first design failed the gate, and what changed.** The first design added the audience to every statement the same way: a union with the rules' set for a count, and one more probe for a page. It failed: first page, sparse reader, 315 ms at p95; first page, named-only reader, 406 ms; count of the dense reader, 1,739 ms at p95 and 2,015 ms grouped, against 1,500. The numbers beside it, with no policy, showed two causes.
  - **A page walks the records in order and probes each one.** That is right when many rows match, and wrong when few do. It costs about 700,000 / n ms for n matching rows. A reader whose rules reach 1,000 rows already needed 558 ms, and a reader whose rules reach nothing needed 4,596 ms, before there was an audience. Naming rows helped them. What cured them is reading a few rows as one small set and sorting it, which costs about 15 microseconds for each row. The two cost the same near 6,900 rows. `FEW_ROWS` is 5,000, a little before.
  - **A union of a quarter of the table is the wrong shape.** The planner could not run it in parallel and it spilled to disk: 1.6 s against 0.55 s for the same count as `id IN (rules) OR id IN (audience)`, which it runs in parallel. So a count over many rows uses two memberships, and over few rows one union, which the planner joins to the records through the index in 46 ms.
  - The cost of the two small questions is in the numbers above. A reader the audience names nowhere asks the first and stops: the statement is the one they had.
- **Not measured.** A list sorted or filtered on an indexed field, for a reader the audience names: it reads the same two forms through the indexed statement, and only the default order was timed. A table of 10,000,000 rows, where the crossing between the two forms (about the square root of the table size times the page) is higher than the fixed 5,000. A rule that is not one test or an OR of equality and membership tests (an AND, a NOT, a range) cannot be counted by the two questions, so it is read as many: a count of two memberships and a page with its probes. The count of a dense reader is 11% slower with the policy than without (514 against 464 ms at p95), and its grouped count 4% slower (544 against 522 ms). Both are about a third of the limit.
- **Not done.**
  - A reminder before a condition lapses, and a notice when a condition does.
  - The steward of a condition's owning function maintaining it, and a sync that sets conditions. An admin sets them in version 1. The `source` field records a sync, and nothing writes one.
  - A condition that applies to some actions and not others, such as writing.
  - Dates on an audience value. A name carries no start or end date. A row share is the way to give a person a row for a time.
  - A notice to the receiving unit's steward when a value waits for acceptance, a refusal to accept (the value stays unaccepted), and withdrawing an acceptance.
  - The audience in the reads that call `allows` or `plan_of`: the app-wide list, the older offset list, link pickers, history, comments, trash, undo, exports. They do not see a name. An export of the rows a person is named on is a later step.
  - A write through a policy that creates or deletes a row. Naming grants neither.
  - The retention job that strips a field from every row does not rewrite `record_audience` for an audience field, and a snapshot restore was not checked against it.
  - The desk screens for the policy, the waiting values and a person's conditions.
  - The access review and `explain` counts, which count by the row rules and the grants and do not count a name.
  - Conditions are not part of the evaluation of a label change: it names positions.
  - Group names (`group:id`) are read, but nothing here shows a steward which group ids exist.
- **Tests.** `data_layer_audience_test` (nine cases: the seven the brief names, a change to an audience field in the record history, and the two forms of a statement reading the same rows). `data_door_test` has one case for the data crate's functions. `manifest_format_test` has the audience rules and two fixtures. `test_audience_conditions_schema.py` (five). Two two-instance cases, `audience_accept` and `condition_set`, cover the two write routes. `million_rows_with_audience` is the gate. `access_scope_paths_test` lists `audience_cover` among the functions that load records and return field names only. Test files that build a `ScopePlan` or an `AppTable` by hand gained `audience: None` or `audience: Vec::new()`. The label gate gained `needs_when`, and `needs` is its caller with every condition met.
**Phase 9 is built (2026-11-27), in part.** An automation is a flow, and a flow already acts as `flow:{rule_id}`. A flow can now be made a system user: it gets an owner and grants that a person approved, and it reads and writes only what its live grants name. The migration is `0126_system_users.sql`. The code is `service::system_users`, `repository_system_users.rs` and `routes/system_users.rs`. No second identity was built. No schema was added: the tables hold plain columns, and the ledger payload follows the other grant entries.

- **The routes.** `POST /data/system-users` makes a flow a system user, with an owner. `GET /data/system-users/{flow_id}` reads it, with its grants as sentences. `PATCH /data/system-users/{flow_id}` changes the name, the notes or the owner. `POST /data/system-users/{flow_id}/grants` approves a grant. `POST /data/system-users/{flow_id}/grants/{grant_id}/revoke` ends one early.
- **A grant.** It names a dataset (an app table), an action (`read`, `write` or `create`), fields (none for `create`), a reason and a review date. The date is ahead of now and at most 366 days away. A link field is a write of that field. A grant of a sensitive field is refused with a 400, as an outside app's ceiling refuses it. A computed field is never written.
- **Nothing by default.** A system user with no grants reads and writes nothing, whoever started the run. A run starts only when every read, write and create that the flow's stored steps need is covered by a live grant. If not, the run is stored as `Failed` with the reason first in its log, and nothing runs. The reason starts with `system_user_not_granted:` and names what is missing in words. The breaker does not count it.
- **The approver holds the access.** `holds` asks the approver, by their own rights, the questions a read or a write of theirs would be asked: they open the app (read, or write records), the row rules reach every row of the table for the operation, and no column rule hides the field, masks it for a read, or makes it read-only for a write. If any answer is no, the approval is a 403 with the sentence "You do not hold read access to Financial Aid / Awards: amount yourself. Nobody can grant more access than they hold. Ask someone who holds it to approve this". The check is the one way a grant is stored, so no other route can store one. A guest, a view-as session, a call through an outside app's ceiling and a shared link are refused first, for registering, changing, approving and ending.
- **The ledger.** An approval writes one `AccessGrantChanged` entry (`op: "create"`, `source: "system_grant"`), with the reason as its rationale and the grant, the flow, the dataset, the action, the fields, the review date and the approver in the payload. It commits with the grant. An end writes the same decision type with `op: "end"`. No decision type was added, so the count stays 68.
- **The owner.** A person, a group or a position, stored as a membership key (`members::stored_key`). The people are read when needed (`members::people_of_key`), so a position owner follows the seat. The owner, and whoever manages the flow's app, change the system user and end its grants.
- **Expiry.** `admit` compares each grant's review date with the database's clock on every run. A grant reaches nothing on its date, and no job is needed. A run that finds a grant it needs has lapsed is refused with `system_user_lapsed:` and tells the owner through a new notice, `system_grant_lapsed`. The notice names a dataset and a count of others, never a row. It is forced into the inbox. The lapse is claimed by one `UPDATE ... RETURNING` that sets `lapse_notified_at`, so two runs cannot both send it, and a claim whose notice failed is given back. If nobody holds the owner's seat, the owners and admins of the flow's workspace are told. The notice kinds are now 22.
- **Change.** What a flow reads and writes comes from its steps, which already derive their grants when a version is saved. A new version that needs more than the live grants cover is refused until a grant for it is approved. A name, notes, an owner or a schedule (the flow's trigger) changes no need, so it needs no approval and writes no entry.
- **Category gates.** A run reads each row it addresses (the trigger record, found records and linked records) as the system user. It holds no position, so a category that asks for one closes the row: a found row is dropped, and a trigger row stops the run with `system_user_category:`. A test shows a flow that links a tagged row when it has no system user, and does not when it has one.
- **Not done.**
  - A flow with no system user keeps running inside its own derived grants, as it did before. Making a system user required for every flow would stop every existing flow until grants are approved, so it is left as a decision for a later phase. Nothing is widened: a system user can only narrow what a flow reaches.
  - No job sends the lapse alert. A lapse is told when the next run meets it. A flow that never runs again is not told.
  - Only a flow has a system user. A rule of the older single-app kind does not read data this way.
  - A sensitive field cannot be granted to an automation. A later phase could allow it with a second approver.
  - A grant is for a whole table, not for some rows. The system user reads rows by the flow's addressing and the category gates, not by row rules.
  - A test run (dry run) does not check grants or gates.
  - No desk screen. The grants are listed by the route.
  - The `upcoming_changes` alert to an automation's owner (phase 6b) does not use the owner yet.
- **Tests.** `data_layer_system_users_test` (nine cases: nothing by default, a grant opens only what it names and a wider version needs a new one, an approver without the access is refused and the approval is in the ledger, a guest, a view-as and an outside app cannot approve, a sensitive field is refused and a category gate closes a row, a lapse refuses the run and tells the owner once, a position and a group owner, a name, notes or schedule change needs no approval, and a revoked grant). `two_instance_test` has the case `system_user`. `notifications_test` counts 22 kinds.

## The catalog: find what exists and ask for it

You called this the biggest problem we could try to solve: what data is out there, and how do I ask for it. A person should be able to find a dataset without seeing a single real value.

**What is already here.** A catalog route and an MCP tool list datasets with fields, owners, and sample rows. The samples are typed by hand today. The tool returned samples to any signed-in caller, even for restricted data, while the HTTP route withheld them. I fixed that on 2026-10-19: both now use one rule, `service::catalog`, and a test covers it. This shows how easily a catalog leaks once samples come from real data.

**What a catalog entry holds.** Plain metadata that follows a published schema: the name and a description in words, the owning function and unit, the steward as a position, the fields with their types and plain descriptions, the category labels, how fresh the data is, an approximate size, how it is used, and **how to ask for access**. None of it is a real value.

**Samples that are safe.** Three kinds, in order of safety. The institution picks per dataset.
1. **Synthetic from the schema.** Values are made up from each field's type and format, with no real value used. It cannot leak by construction. This is the default and the only kind in version 1.
2. **Synthetic from the shape.** Values follow the real distribution of the data. This is more useful and carries real risk of leaking outliers or small groups. It needs a privacy review. I would not ship it without one.
3. **Masked real values.** The riskiest. Masking real rows often leaves rows that can be identified, especially in small groups. I would not offer it.

Counts and ranges are masked for small groups, so a row count cannot identify a person. The threshold is policy.

**Discovery is governed too.** The existence of a dataset can itself be sensitive. Each dataset has a **discoverability** setting in policy: visible to everyone at the institution, visible to named units or positions, or hidden. A field can be shown by name, or shown as "a restricted field". The default is visible, so people can find things.

**Search.** Search runs over the catalog entries, not over data. People can search by words, by function, by unit, by category, and by field names. The same search is an MCP tool, so an AI agent can answer "what data do we have on course enrollment" and offer the person a request. It returns metadata and synthetic samples only.

**Requesting access.** Every catalog entry has a **request access** action.
- The form is a policy. A dataset lists the fields a request must carry: a reason, a project, a duration, an agreement reference. The schema offers these. The institution chooses.
- The request goes to the dataset's steward position, which is a seat and so survives people changing. A function steward and a unit steward may both be asked, if policy says so.
- An AI tool may write a short, advisory assessment of the request against policy. A person decides.
- The outcome is a dated grant or share, with a reason, that follows the rules already in this note. A refusal gives a reason the person can read.
- The person can see where their request is, and who has it. A reminder goes to the steward if it waits. The reminders follow `notifications.md`.
- Every request and outcome is a ledger entry.

**What it needs that is not built.** Everything above. The Datasets screen and its API already list datasets. They do not search, show discoverability, hold synthetic samples, or take requests. The inbox that reminders use (`notifications.md`) is not built.

**Where it fits in version 1.** I would put the catalog in version 1 as the first visible feature. It needs the dataset ids from phase 1, but not the full label and grant machinery. It would also give early pilots a reason to use the platform, and it would show what questions people ask. Request access can start as a form that sends a notice to the steward and records the outcome, before the rules engine exists.

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
| "A dataset floor will lock out the people who need the data." | The steward sees who a floor reaches before saving, as `access-model.md` phase 13 shows a requester the effect first. A platform admin keeps a way in, and every admin read of labeled data is a disclosure |
| "Row tags will be slow at a million rows." | Not measured. The tag set is indexed with the record index, and a label requirement compiles to SQL the way row rules do. It must pass `million_rows_test` before phase 3 is called done |
| "This is a rewrite." | Phases 1 and 2 move code and add ids. They change no behavior and each has its own tests. The new behavior starts at phase 3 |
| "Who decides what the policy says?" | The organization does, in settings and OSCAL. The AI tool reads the policy; it does not write it |

## Decisions so far

| Question | Answer |
| --- | --- |
| Where does a row's unit come from? | A column on the row, from the governed list of units. A value not on the list is rejected. When the column is empty, the dataset's unit |
| Who is a steward? | A position for a function and a unit, defined in the system, inherited from a higher unit or specified. Deputies allowed |
| May AI raise or lower labels? | No. It proposes. A person reviews a plain evaluation first |
| Does an automation act as a person? | No. It is a system user with approved grants and the same ownership options as an app |
| Can one row be shared? | Yes. Beyond the default permissions it needs a justification and an expiry, and may need an AI review before a person approves |
| Override | Parked. No dataset floor, and no override of a lower unit by a higher one |
| A dataset default for untagged rows | Kept. It only fills a gap |
| A row with more than one unit | A home unit, plus additional unit tags that show the row to those units. Row shares cover the rest |
| Where functions are defined | The administrator defines top-level ones by proposal. A unit's local governance function approves local ones. A lower unit cannot redefine one above it |
| Approving an automation's grant | The approver must hold the access they grant |
| A share touching a restricted category | Approval from someone authorized for that category |
| The tree used for reach | The org tree |
| Exclusions in a reach | Included from the start |
| Platform principle | Mechanism in the platform, rules as policy data with small published schemas. AI tools help write and explain policy. Every decision is explainable. Performance is a budget |
| The catalog | A first-class feature. Metadata and synthetic samples only. Search and request access. Discoverability is a policy |
| Audience fields (access by being named in a row) | Accepted, if performant. A test gates it |
| Cross-listing and similar | The receiving unit's steward accepts. A synced value from the student system is authoritative |
| Training and similar conditions | Stored as dated values on the person record, owned by a function, from a sync or by hand |
| Conditions owned lower down | Yes. A suborg may add and maintain conditions for its own people and data, in addition to the central ones |
| Delegated provisioning | A steward may delegate a dataset to a suborg, which adds its own rules. See "Delegated rules" |
| Connectors | File feeds first. Connectors to the major platforms later |
| A change of major | Notes stay with the unit that provided the service. The new unit may request them. Units vary in how freely they share, so each sets its own policy from the closed set |

## Open questions (all about parts after version 1)

1. **Details of "Changing a label".**
   - The minimum notice period for each category class. The institution sets it in settings. I have not proposed numbers.
   - An expedited path for urgent protection, needing two people. Do you want it?
   - Who drafts the communications plan. I assumed the AI drafts and the steward approves.
2. **Extra unit tags.** Did I read you right? A row has one home unit and may be tagged with more units so each sees it. I made reading the default for an additional unit, writing off by default, approval by the home unit's steward, and a review cycle instead of an expiry. Is that the shape you want?
3. **Moving data around governance.** Do you agree with the layers, and with refusing a write instead of raising the target's label? The cost is that a refused write can stop a running automation. How strict should layer 3 be by default: hold a run to everything it has read, or only to declared mappings?
4. **Who is the "local governance function"?** I made data governance a function, with a steward at each unit that has one. A unit with none would use the nearest above, which brings back the override worry. Should a unit be able to opt out of inheriting that, and say its functions need no outside approval?

## How datasets are shared and extended locally (not understood yet)

I checked the code and the earlier briefs on 2026-10-19. The honest answer is that we have a plan for one half of this and nothing for the other.

**What exists.** Nothing real. The desk's "use this dataset in a new application" and "link a published dataset" buttons only change state in the browser. They call no server route. A link field they add (`Relation`, with `linked_dataset_id`) is a local copy of a field description, not a stored link. The Datasets screen now reads the server's catalog, but the catalog holds metadata and sample rows. It holds no live rows and no grants.

**What `connections.md` plans.** A dataset is a governed, read-only copy of an authoritative source system (student information, HR, grants), refreshed on a schedule. It lives in a system app that only Platform Admins own. Only the sync writes to it. A source that changes shape stops the sync and makes a proposal. Apps use a dataset by a `Link` field into it, with lookups and rollups. A local unit never adds a column to the dataset. It adds its own table and links to the record. None of this is built (rows 135 to 139).

**What that leaves open.**
1. **Two meanings of "dataset".** `connections.md` means a read-only, synced copy of an outside system. This note means any governed group of rows, including editable ones a unit owns. They need one definition. I would use one: a dataset is a governed table with an owning function and unit, and its source is either a sync or local entry. A synced one is read-only.
2. **Who decides the labels.** `connections.md` says a Platform Admin sets sensitivity at the connection. This note says the steward for the function and unit owns labels. I would let the administrator set the first labels, then hand them to the steward.
3. **How a unit shares data it owns.** There is no mechanism. This note gives one: an app is granted a dataset, and a person's grants and the labels decide the rest. I do not know whether that matches how units want to share. Do they offer a dataset to the whole institution, to named units, or only on request?
4. **How a unit adds its own fields about someone else's record.** The pattern in the briefs is a local table that links to the central record, with the central fields shown by lookup. That is a sidecar. Open parts: who may see a local sidecar row beyond its unit (the central owner?), how the sidecar is labeled when it describes a sensitive central record, and what happens to it when the central record is deleted or merged.
5. **What people actually do today.** I do not know the real cases at your institution. I expect some of these: a department keeps its own spreadsheet about each faculty member, a unit wants extra columns on a central list, a unit needs its own code table, and someone pastes an extract into a new sheet. The last one is the path around governance.

### Keys and local columns (checked in the code on 2026-10-19)

**What exists.**
- A row's identity is its `id`, a string primary key on `dataset_records`, scoped by app and table. Links point at that id (`record_links.to_record`), so a link survives a rename of any field.
- There is **no key concept**. A field has no `unique` or `key` flag. A table's `primary_field` is a display name, used for example to label a copy. The only index flag is `indexed`, which feeds `record_index` and is not unique. No unique constraint exists on any user field.
- The import in PR #121 takes an `upsert_key` for each import and matches by lookup. It already reports "matched multiple records" as ambiguous. That means duplicates of a business key can exist today, and an upsert on a duplicated key cannot be trusted.
- Field values are stored as JSON on the row. Adding a field to a table changes the app's definition, not the database. So adding a column is cheap in storage. The cost is governance: who may change the definition of a table that someone else owns.

**So the answer to "will our keying allow local columns" is: not yet.** A local table can link to a central row by id, and that works. But nothing guarantees the central key is unique, so a sidecar, an upsert, or a sync that matches on the key could attach to the wrong row.

**What I would add.**
1. **A declared key for a dataset.** One field or several, which the dataset's steward sets. Uniqueness is enforced by a small table, `record_keys (app, table, key_text, record_id)`, with a unique constraint, written in the same transaction as the record. This follows how `record_index` and `record_links` already work, and it avoids per-dataset database changes. The key text is normalized by rule (trimmed, case-folded if declared).
2. **The record id stays the identity that links use. The key is the business identity.** A sync matches rows by the key and keeps the record id stable. If a source changes a key value, that is a key change with a history entry. It must not be a delete and an insert, or every sidecar is orphaned.
3. **A sidecar is a local table with a link to the central row.** For one local row per central row, add a unique link: at most one sidecar row for each central row. Without it, it is one central row to many local rows. Both are useful.
4. **Merges need an alias.** If two central rows become one, links to the old id must follow. Nothing in the briefs covers this yet.
5. **Local columns on the central table are allowed only by a change to its definition.** The central steward approves it. Version 1 prefers the sidecar.
6. **Keys and secrecy.** A key such as an employee number is often wanted by everyone who links, while other fields are restricted. If the key is itself sensitive, use an opaque surrogate key for linking. A rejected duplicate must not tell the writer anything about a row they cannot see.

Not built or tested. This also changes `connections.md`: its sync must match on a declared key.

**What I would do before building.** The template is [data-layer-scenarios.md](data-layer-scenarios.md). Take five real examples from your organization and walk each through the model. For each one, write down who owns the data, who needs to see it, what the local unit adds, and what the central owner should be able to see back. If four of the five fit the sidecar pattern, it is right. If not, the model changes.

## Scenarios walked

Two real-shaped cases, talked through on 2026-10-19. Each exposed something the model lacked. These are proposals until you confirm them.

### Advisor notes on students

The registrar owns the student record. A department's advisors keep notes on their advisees. The registrar, advising leadership, and the advisor read them. An advising team may need access across teams. If a student changes major, the old department may or may not be willing to share. FERPA training matters, but only for some uses: coding into the student system, or looking up information in the advising system.

What it needs that the model did not have:
1. **Access by being named in the row.** The advisor reads these notes because the advisor's name is in a field on the row. The model had grants by role, position, function, and unit, and shares for single rows. A share for every advisee is not manageable. See "Audience fields" below.
2. **A condition on a requirement.** "FERPA training is current" is neither a role nor a unit. It is a fact about the person, with an expiry, and it may apply to some actions and not others. See "Conditions" below.
3. **A group as the grantee**, for the advising team across teams. The member principals already allow a group. The reach for such a grant is a listed set of units, not the unit tree.
4. **A change of major is a request, not an automatic share.** The default: the notes stay with the department that wrote them. The new department's advisor can request them. The source steward approves or declines, as a row share with a reason and an expiry. A department that is willing can later pre-approve it with a standing policy. Standing policies are not in version 1.

### A cross-listed course

The registrar and the School own the catalog row. Both departments see it and edit it. The second department gains visibility when the cross-listing attribute is set. Each school should be able to set its own policy, with an override and manual sharing.

What it needs:
1. **A unit-valued field that grants.** The cross-listing attribute names a unit. Naming it makes the row visible to that unit. This is the same mechanism as the advisor field, with units in place of a person. See "Audience fields". It also means the extra-unit tags I deferred are needed. They come from a field, so they are one mechanism and not two.
2. **A split between two kinds of steward.** The registrar is the steward of the **function**. It owns the definition: the fields, the categories, and the key. The School is the steward of the **unit**. It owns who may see and edit its own rows. Two owners are real here, but they own different things. This is also the answer to the provost worry: a function steward does not decide who sees a unit's rows.
3. **A policy each school sets.** A school decides who may set the audience field, and what the named unit may do. The platform offers a small closed set of choices and a manual override. It does not offer a rules language.

### Audience fields (proposed for version 1)

An **audience field** is a field of a dataset whose value names people, positions, or units. The dataset's policy says what being named grants: read, or read and write, on which fields or functions.

- The advisor field, the cross-listing field, an assignee, and a principal investigator are all audience fields.
- Who may change the field is an ordinary field-level write grant. Changing an audience field is recorded as a grant in the history, with who did it.
- It grants only up to what the policy says. A category's requirement still applies, so naming someone cannot bypass the FERPA requirement.
- A policy is set by the unit's steward, for that dataset.
- **This departs from an earlier rule.** `access-rules.md` says a rule narrows and never grants. An audience field is a grant that comes from the row. **Decided: accepted, on condition that it is performant.** The condition is a test, below.
- **Acceptance by the receiving unit.** A policy may require that the unit named in an audience field accepts before it gains access. Coordination between departments is the usual case. Until the receiving unit's steward accepts, the value is a proposal and grants nothing. The acceptance is recorded as a grant.
- **The authoritative system decides when it owns the rule.** Many of these rules live in the student information system. The platform does not copy the rule. The audience field arrives by sync, and access follows it. A person cannot override a synced audience value, except by a recorded share.

**Making it fast.** The names in audience fields are copied into a small indexed table, `record_audience (app, table, record_id, kind, principal)`, in the same transaction as the record. This follows `record_index` and `record_links`. A list adds "or the record names one of my principals", where my principals are me, my positions as `key@unit`, my units, and my groups. That set is small and is known when the person signs in. The test: a table of a million rows with audience grants on a fifth of them must stay inside the `million_rows_test` limits for a first page, a page near the end, and a count. I have not measured it. If it does not pass, the design changes before anything ships.

### Conditions (proposed for version 1)

A label's requirement may carry **conditions**: facts about the person that must hold, each with an expiry. FERPA training current is the first. A condition may apply to some actions and not others, such as writing into the student system.

- The fact comes from a system of record, such as the learning system or HR. The platform stores the status and the expiry date.
- A lapse withdraws access. That is a change in a person's status, not a label change, so it does not use the label change process. The person gets reminders before the date. This is the same kind of notice as for a share that is about to expire.
- The list of conditions is a short closed list the institution switches on. It is not an expression language.
- **Where the fact lives.** Training records are in Workday here, kept by the Registrar. Elsewhere they may be in HR or another system. So a condition has an **owner**, a function whose steward may maintain it, and a **source**, which is either a sync or manual entry. Every value has a start and an end date.
- **How it is checked fast.** The current conditions sit on the person record, as a small map from condition to expiry date. They are loaded when the person signs in. A check is a date comparison at read time. No job has to run for a lapse to take effect, and no extra query is made. A sync or the owner updates the map when a record changes.

**Dated access in general.** A grant, a share, and a condition all carry a start and an end date. The platform shows them together as a person's access, with dates, so that an auditor can ask who had access on a given day. The ledger and these dated records already hold the facts. A point-in-time report over them is a later feature.

### Delegated rules

Many suborgs need to manage rules, and some are handed a dataset to provision internally. The model is **layers**.

- A steward may **delegate** a dataset, or part of one, to the steward of a suborg. Each delegation sets a **ceiling**: the most access the suborg may hand out.
- A suborg may always **add restrictions** to its own rows: a narrower audience, an extra condition, an extra approval. It cannot remove or loosen a rule from a layer above.
- A suborg may **add grants and audience policies** only up to the ceiling.
- Each rule records the layer that added it. The evaluation page and the access report show the layers, so an auditor can see which rules are central and which are local.
- Conditions follow the same layers. The Registrar's FERPA training condition stays central. A department may add one of its own, such as a lab safety course, and maintain the dates for its own people.

I read "additive" as: rules stack, and a lower layer never removes a higher one. Please say if you meant something else.

### Student workers

A student who holds a job has two capacities in one identity. Student workers generally must not see instructor and peer data, because it is sensitive. Some jobs need it.

What it needs:
1. **Being a student grants nothing, and by default excludes.** A label's requirement can exclude student identities. A position can be designated **student-eligible** by the data's steward. Only grants through such a position reach the data. A student worker in the registrar's office gets what that position was approved for, and nothing from the student side.
2. **Access comes from the position held for the job,** never from the person's other capacity. This is already how grants work. The rule above is what keeps a student from reaching data by being an employee at a desk where it is not needed.
3. **Peer data** is the harder part. A student worker should not read records of classmates or of instructors they teach under. A simple default is the exclusion above. A sharper rule would compare the worker's classes with the row's subject. I would not build that until a pilot needs it.
4. **FERPA.** I recall that records of students employed because of their student status can count as education records. I have not checked this in this session. Your compliance office should confirm how it applies before anything relies on it.

### External collaborators

Research collaborators from outside the institution need access to some research datasets. They are not the guests in `guests.md`. A guest is an applicant who submits one form and sees only their own records. A collaborator reads a shared dataset for a project's length.

What it needs:
1. **A principal kind for people outside the directory,** sponsored by an internal person who is accountable for them.
2. **Always dated.** A collaborator's grant has an end date, usually the project's. It records the agreement it rests on, such as a data use agreement, with its reference and end date.
3. **A key credential, not a password.** The collaborator registers a public key and signs requests with their private key. The platform stores only the public key. Keys expire and can be rotated and revoked. The same mechanism serves system users for automations, so there is one credential type for non-directory principals. I know OAuth 2.0 "private key JWT" client authentication as the standard for this, from memory. I have not checked it in this session. `foundation.md` phase 10 already plans verified tokens.
4. **Federated login where it exists.** A collaborator at another university may sign in through their own institution's identity provider. That is likely better than a key for them. I would support both.
5. **Narrow by default.** Their access is a grant on named datasets and fields, through the same function and field rules. Export is off unless the grant says so. Every access is a logged disclosure. A grant needs the sponsor and the dataset's steward to approve.

## Answers to the questions from the scenarios

You asked me to answer these. Each answer follows the principle above: mechanism in the platform, rules in policy data.

| Question | Answer |
| --- | --- |
| "Additive" layers | The platform enforces one invariant: a rule from a lower layer never removes or loosens a rule from a higher layer, and a delegate cannot grant beyond its ceiling. Everything else about what a layer adds is policy |
| Choosing a capacity for a session | Not built in. A person has attributes (affiliation, positions held, conditions). A policy can use any of them. "Student worker" is a position with an attribute, not a mode. If an institution wants to separate the two roles, it writes that as policy |
| What "peer" means | Not defined in code. The platform offers one generic comparison, a row field against a person attribute, such as the sections a person is enrolled in. An institution writes "peer" as a policy with that. It is parked until someone needs it, because it is the costly one to make fast |
| How external collaborators sign in | A principal has an authentication method, and the methods are pluggable: federated login, a registered public key, or both. The grant does not care which. Nothing assumes where collaborators come from |
| Requiring an agreement | A grant type lists its required fields, and the institution decides. "External collaborator grant" can require an agreement reference and an end date. "Student worker grant" need not. The platform ships the fields available and some sensible starting templates, and no fixed answer |

## Not decided, and not verified

- **Performance.** Nothing here is measured. The indexed unit column is a design choice I have not tried. The `million_rows_test` limits do not move.
- **OSCAL mapping.** The control ids in `position-roles.md` phase 5 are not checked against `oscal-catalog.md`.
- **Existing data.** I have not counted rows or apps. Phase 1's migration must be tested on a copy of real data before it runs on any institution's.
- **Cedar.** The note keeps Cedar for questions about workspaces and apps. I have not worked out which dataset questions, if any, belong in Cedar. The audit says mutations must meet Cedar, and phase 2 must keep that true.
- **The web desk.** Several screens read app routes. They keep working through phase 2 because the paths agree. Moving them to dataset routes is not planned here.

## Prompt for the session

Read this file, `position-roles.md` phase 5, `sensitive-content.md`, and `access-rules.md`. Do not start until the open questions are answered. Phase 1 first. Stop when its tests pass.
