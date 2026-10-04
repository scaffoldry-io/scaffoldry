---
name: the-reader-is-an-engineer
description: Use before proposing any design, architecture, tool, or process that an institution's engineering or security team will read or run. Use especially when the design is elegant, minimal, or self-contained, because those are the qualities that hide what a professional team would refuse.
---

# The reader is an engineer

## The rule

Every design carries, before it is built, the list of what an
institution's engineering or security team would object to — written
as they would say it. Enterprise credibility is a requirement of the
product, not a courtesy to be paid later.

## Why this is required

This method was built for a week without once asking whether a
university's engineering team would respect it. The result was a
governance layer stronger than most enterprises have, running on an
800-line bash installer, a UI with no tests, a YAML parser written by
hand, and data stored as prose — each a thing a professional team names
in the first ten minutes, and each one an honest objection. The checks
caught every bug. They could not catch the shape, because no check
asked the question a reviewer would.

## What to do

Before the design is built, write the objection list. It has at least
these headings, each answered as a senior engineer or a security
reviewer at the customer would put it:

- *What would they expect instead?* Ansible for configuration; a
  database as the read model; a template engine; a test suite; the
  standard's vocabulary.
- *Where does this not scale?* Name the number at which it breaks.
- *What would a security review flag?* Credentials, quoting through
  shells, anything parsed by pattern, anything world-readable.
- *What is bespoke that has a standard answer?*

An empty list is a finding about the list, not about the design.

Then decide, for each objection, whether to meet it now, meet it at a
named trigger, or refuse it with a reason. Refusing is allowed. Not
knowing is not.

## What must never happen

- A design proposed without its objection list
- An objection list that flatters the design
- "Least software" used to dismiss an objection about structure,
  testing or credentials
- A bespoke component where the reader would expect a standard one,
  without a recorded reason

## The eval

`evals/the-reader-is-an-engineer/`: the installer scenario.
