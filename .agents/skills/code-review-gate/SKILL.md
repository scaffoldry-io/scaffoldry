---
name: code-review-gate
description: Protocol for performing structured, pre-merge reviews on pull requests. Enforces the 6-line proposal header, enterprise objection lists, and empirical verification.
---

# Code review gate

## The rule

No PR is merged to `main` without completing this structured review gate. Reviews verify design rationale, boundary enforcement, empirical test evidence, and enterprise credibility.

## The review protocol

Every PR submission must carry:
1. **The 6-Line Proposal Header:**
   - `Title:` what this change does, in one direct sentence
   - `Risk:` routine | notable | significant
   - `Ask:` what approving and merging this means
   - `IfYes:` what becomes true in the system
   - `IfNo:` what remains broken or deferred
   - `Checks:` exact test commands run and their empirical outputs
2. **The Enterprise Objection List:**
   - Documented concerns written from the perspective of an institution's senior security/engineering team.
   - Explicit answers explaining how the design addresses each concern.
3. **Automated Verification:**
   - Green CI status across all automated linters and schema validators.
   - Clean diff with zero untracked files or leftover debug artifacts.

## What must never happen

- Merging a PR without documented test execution results
- Approving a change with an empty or superficial objection list
- Rubber-stamping PRs that fail automated CI
