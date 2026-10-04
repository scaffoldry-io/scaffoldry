---
name: data-has-a-format
description: Use when about to store, read, or design anything a program will read back — a list, a table, a mapping, a register, a manifest — whether in a new file or added to an existing document. Use especially when the tempting answer is a markdown table, a front-matter block, or lines a script will grep.
---

# Data has a format

## The rule

Structured data gets a structured format and a published schema before
anything reads it. Prose is for people. A file a program parses is data
whatever it looks like, and the test is one question: *will a script
read a value out of this?* If yes, it is not prose, and it does not go
in prose.

## Why this is required

A register of nodes, roles and holders was written as markdown tables
and read by `sed`. In one afternoon that produced three faults, each
silent: a row in the wrong table matched the pattern, a comma in a cell
broke a name, a letter's case was wrong. Silent, because a pattern that
matches nothing returns nothing, not an error. `json.load` on the same
data would have refused each one with a line number. Least software
means the fewest components, never the least structure in the data: a
format with a schema is less software than a parser.

## What to do

- Choose JSON unless a standard for the data names another format.
  Where a standard exists for the data itself, use its shapes; see
  `standards-first`.
- Write the schema, or name the published one, before the first record.
  A closed schema: an unknown key is a finding.
- Every reference between records is checked before anything reads the
  data: a role that is not a role, a parent that is not a node.
- Prose that people read is rendered *from* the data, never parsed back
  into it.
- Git is the system of record. Anything a person browses is read from a
  store rebuilt from git, never assembled from the source on each
  request, once the data is larger than one screen.

## What must never happen

- A markdown table, a front-matter block, or a line format that a script
  reads with a regular expression
- A parser written by hand for a format that has a library, in order to
  avoid the library
- Two files that must agree, maintained separately
- A value that a program reads stored in a file whose purpose is to be
  read by a person

## The eval

`evals/data-has-a-format/`: the register scenario, which is the one
that produced the faults above.
