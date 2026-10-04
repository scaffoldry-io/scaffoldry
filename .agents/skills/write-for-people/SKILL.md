---
name: write-for-people
description: Use when writing or editing any prose a person will read, including documentation, commit messages, PR descriptions, skill files, error messages, and review comments. Use especially when the sentence you just wrote has a semicolon, a "which" clause, or a pronoun whose antecedent is more than one sentence away.
---

# Write for people

## The rule

Every document in this project is written so a person can scan it and
find what they need. One idea per sentence. Name the actor. Lead with
what to do. Use the simplest word that is still precise.

The test: a new team member with no conversation history reads the
paragraph. Can they tell what to *do* within ten seconds? If they have
to re-read it, the paragraph is too dense.

## Why this is required

This project's documentation was written by advanced AI models briefing
other AI models. The prose is dense, allusive, and loads multiple ideas
into single sentences separated by semicolons. Pronouns point backward
across paragraphs. Rules are stated as observations instead of
instructions. Key information hides in subordinate clauses.

The ideas are sound. The governance model, the threat analysis, the
architecture: all strong. But a person scanning a workplan or a skill
file has to re-read sentences three times to extract the requirement.
An engineer evaluating this product will read a few pages and decide
whether it feels trustworthy. Dense, indirect prose signals that the
author is performing rather than communicating.

Agent-to-agent prose also harms the agents reading it. A clear
instruction is followed more reliably than an elegant observation that
implies the instruction.

## What to do

**1. One idea per sentence.**

If a sentence has a semicolon and a "which" clause, split it. Two
short sentences are always easier to follow than one long one.

Before:
> Each required skill's rule and its must-never list are inlined; a
> session begins with them in context, and a commit made without them
> is refused at the gate.

After:
> Required skills are shown in full below. They load at the start of
> every session. Commits that violate them are rejected.

**2. Name the actor and the action.**

Say who does what. "You must do X" is clearer than "X is done." If the
system does something automatically, say "the system does X."

Before: "A commit made without them is refused at the gate."
After: "The server rejects commits that violate these rules."

**3. Lead with the instruction, follow with the reason.**

People scan for what to *do*. The reason supports the instruction but
should not bury it.

Before:
> A clone that stays goes stale within hours and is then a copy that
> looks current and is not, which is how four directories here came to
> disagree about what they were.

After:
> **Don't keep long-lived clones.** They go stale within hours. We
> already had four directories fall out of sync this way.

**4. Use the simplest word that is still precise.**

"Use" instead of "adopt." "Check" instead of "verify on the running
service." "Follows" instead of "maps onto." Save technical precision
for the technical term itself, not the verbs around it.

**5. Define jargon on first use.**

The first time a document uses a project-specific term, give a short
parenthetical definition:

- the desk (the approval dashboard)
- the record (the git repository holding this organization's decisions)
- the runner (the systemd service that carries out approved changes)
- the register (the JSON file listing nodes, roles, and who holds them)

After the definition, use the short form.

**6. Don't write around the point.**

Before: "A clone that stays goes stale within hours and is then a copy
that looks current and is not."
After: "Stale clones look current but aren't."

**7. Use formatting to separate structure from content.**

- Bullet lists for steps or options
- Bold for key terms and actions
- Code blocks for commands and file paths
- Headers for sections

Don't embed structured information (like the six-line commit format)
in paragraph prose. Pull it out into a code block or a list.

**8. Limit backward references.**

If a pronoun ("it", "this", "that") could refer to more than one thing
in the preceding text, use the noun instead. "The service" is always
clearer than "it" when multiple subjects are in play.

Before: "It was refused in review twice in one evening, the second time
because the older reference documents modelled the pattern and it got
copied."

After: "This pattern was rejected in review twice in one evening. The
second rejection happened because older reference documents used status
blocks, and they got copied into new ones."

## What must never happen

- A sentence with more than one semicolon
- A pronoun whose antecedent is more than two sentences away
- A rule stated only as an observation ("X happens") with no
  instruction ("do X" or "don't do X")
- A project-specific term used without definition in a document meant
  for someone who has not read every other document first
- A paragraph of prose used where a list, a code block, or a table
  would let the reader find the information faster

## The eval

`evals/write-for-people/`: a rewrite of an existing document section,
scored by whether a reader unfamiliar with the project can extract the
requirements on first reading.
