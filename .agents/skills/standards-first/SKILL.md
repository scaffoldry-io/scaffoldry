---
name: standards-first
description: Use when about to name a folder, define a schema, invent a vocabulary, design a data format, or choose a term for a kind of thing. Use especially when the design feels obvious, because obvious designs are the ones that already exist as a standard somebody else maintains.
---

# Standards first

## The rule

Before designing a schema, a vocabulary, a folder name or a format,
look for the published standard — ISO, NIST, ITIL, CIS, OSCAL, or the
ecosystem's own convention — and adopt its shapes, even as a subset,
even when the whole standard is heavier than the need. Invent only when
the search comes back empty, and record the search.

## Why this is required

The folders of this method were named for their history: a policies
folder called `blueprint`, procedures under `lockdown`, a candidates
survey called `library`. A schema for tagging parts with the controls
they cover was drafted from scratch on the same afternoon its author
had downloaded NIST's OSCAL catalog, which is a published schema for
exactly that. The customers of this method are institutions whose
engineering and security teams already speak those standards; a
proprietary word where a standard one exists costs credibility with
every one of them and buys nothing.

## What to do

- Name things by function, in the standard's word: policy, procedure,
  control, baseline, assessment, register, incident.
- For governance data — controls, baselines, what is adopted, what
  covers what, what was assessed — the shapes are OSCAL's. See
  `governance-data-is-oscal`.
- For a data format, the schema published by whoever owns the data
  comes before one written here.
- When adopting a subset, say which standard, which version, and what
  was left out. When extending, use the standard's own extension point
  and mark it.
- Write down what was searched and not found. An empty search is a
  finding worth keeping; the next person will search again otherwise.

## What must never happen

- A vocabulary invented without a recorded search for the published one
- A folder or file named for how it came to exist rather than what it
  does
- A schema written here for data that has a schema elsewhere
- A standard adopted in name while its shapes are not used

## The eval

`evals/standards-first/`: the coverage scenario.
