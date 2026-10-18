# The data layer — design note

```
Title:  Data is a layer of its own, with its own API. Datasets keep the labels. Apps and automations are clients.
Risk:   high
Ask:    Approve the direction, the flexibility principle, and the Version 1 cut. Then phase 1 can start. The open questions below are about later parts, so they can wait. Nothing here is built.
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
8. **A row's unit must come from the governed list of units.** A value that is not on the list is rejected. There is no "bad value" state.
9. **Nothing raises or lowers a label by itself.** AI proposes. A person reviews the plain-language evaluation, with its downstream impacts, before anything applies. No AI may lock someone out in the middle of a process.
10. **A steward is a position for a function and a unit.** Stewards start at the top and may be deputized. Lower units may define their own, in a hierarchy that follows the org. Authority is delegated, not overridden. See "Stewards" below.
11. **An automation runs as a system user.** A person approves its grants. It follows the same ownership options as an app: a person, a team, or a position owns changes and receives error alerts.
12. **A single row can be shared.** Where it goes beyond the default permissions, it needs a justification and an expiry, and may need an AI review before a person approves it.
13. **A label change is never instant.** The steward is asked each time. A change runs on a schedule with a communications plan and a window for exceptions, which are recorded as grants. See "Changing a label".

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

A share adds one more way into the rule in "Who may read a row": a live share that covers the row and field. It is looked up from a small table keyed by row and grantee. List queries add "or the row is shared with me". I have not measured that.

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
| "A dataset floor will lock out the people who need the data." | The steward sees who a floor reaches before saving, as in `position-roles.md` phase 2b. A platform admin keeps a way in, and every admin read of labeled data is a disclosure |
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
