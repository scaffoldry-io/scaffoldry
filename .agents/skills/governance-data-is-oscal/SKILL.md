---
name: governance-data-is-oscal
description: Use when reading, writing, or designing any file about controls, baselines, what an organization adopted, which parts cover which controls, who holds which role, what an assessment found, or what remains open. Use especially when about to invent a field for one of those, because OSCAL already has it.
---

# Governance data is OSCAL

## The rule

NIST's OSCAL is the lattice every governance file here maps onto. A
file about controls, adoption, coverage, roles, assessment or open
findings uses OSCAL's shapes and field names, validates against NIST's
schema where it claims to be a model, and extends only through
OSCAL's own extension points, marked as such.

## The lattice

| The scaffold has | OSCAL model |
|---|---|
| A framework's controls: CIS, NIST, ISO | **catalog** |
| The floor; what an organization adopted, required or optional | **profile**: a baseline plus tailoring |
| Which parts cover which controls | **component definition** |
| What one repository has enabled | **system security plan**, as a fragment |
| What the checks found | **assessment results** |
| Open findings: issues the machine filed, what was parked | **plan of action and milestones** |
| Nodes, people, roles, who holds what | **metadata**: parties, roles, responsible-parties |

## Why this is required

Every one of those files was about to be designed from scratch, by
someone who had that same afternoon fetched NIST's control catalog from
OSCAL and used it only for a list of ids. The customers' auditors and
their tooling read OSCAL; FedRAMP mandates it. A bespoke shape costs
the mapping later and the credibility now.

## What to do

- Use the model's field names even in plain JSON. Full OSCAL is heavier
  than a small practice needs; its *shapes* are free, and the export to
  a full document is then a mapping, not a rewrite.
- A file that claims to be an OSCAL model validates against NIST's
  schema for that model, vendored in `reference/controls/oscal/` at the
  version `reference/controls/pins.json` names, with a JSON Schema
  validator in the check runner. That check is NIST's, not ours. Write
  the pinned version in `oscal-version`, not the newest one you know
  of: moving the pin is a proposal, after the register, adoption and
  coverage validate against the new schema on the machine that can
  fail.
- A file that borrows shapes without claiming the model says which model
  it borrows from in its own metadata.
- Extend through `props` and `links` only, and mark every extension in
  the file's metadata by name. A new top-level key is a fault.
- Read IBM's compliance-trestle before writing tooling: it manages OSCAL
  in git and renders markdown views from the data, which is the
  relationship between prose and data this method uses.

## What must never happen

- A field invented for something OSCAL names
- A file claiming to be OSCAL that does not validate against the schema
- An extension made by adding a key rather than a prop, or made silently
- A markdown view of governance data parsed back into the data

## The eval

`evals/governance-data-is-oscal/`: the adoption file.
