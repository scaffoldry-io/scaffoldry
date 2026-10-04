---
name: test-at-the-customers-scale
description: Use when about to propose anything that touches the register, adoption, the manifest, the desk, or any structure that grows with the size of an institution. Use especially when the test that passed ran against the one-node record this method is built in.
---

# Test at the customer's scale

## The rule

Nothing that grows with an institution is proposed until it has run
against one: a synthetic register with several levels, hundreds of
holders and tens of repositories, on the machine that can fail. One
node and nine parts prove nothing about a university.

## Why this is required

Every governance check in this method passed for a week against a
record with one node, one role, one holder and nine adopted parts. The
desk assembled every page from live calls and was fast. The register
parsed cleanly. Nothing was wrong at that size, and everything that was
wrong was invisible at that size: a page that makes one call per
repository is fine at five and hundreds at fifty; a pattern that matches
one table matches two when a second is added. The customer is a
university. The test machine is cheap. The excuse is gone.

## What to do

- The framework keeps a synthetic institution: five levels, a few
  hundred holders, fifty repositories, generated, not hand-written. Any
  proposal touching the register, adoption, the manifest or the desk
  reports its run against it.
- Report the number where the design breaks, not only that it passed.
  "One call per repository" is a finding to name, not a detail.
- When the machine cannot represent the scale, say so, and say what was
  reasoned rather than measured.

## What must never happen

- A change to the register, adoption, the manifest or the desk proposed
  with evidence from the one-node record only
- A claim that something scales, without the number at which it stops
- A synthetic institution written by hand, so that it is small

## The eval

`evals/test-at-the-customers-scale/`: the register change.
