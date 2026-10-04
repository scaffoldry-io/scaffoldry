---
name: drift-containment
description: Audits code proposals, PR diffs, and container modifications against docs/ARCHITECTURE.md, foundation license whitelists, and boundary constraints.
---

# Drift containment

## The rule

Every line introduced into the repository must align with the 6-layer architecture, adhere to approved foundation licenses, and respect established boundaries. Any divergence is treated as a defect and refused at review.

## The audit checklist

When reviewing any branch, proposal, or PR:
1. **License Whitelist:** Only Apache 2.0, MIT, BSD-3, and PostgreSQL License dependencies are permitted. Reject any copyleft, SSPL, BSL, or AGPL dependencies.
2. **Architectural Boundaries:**
   - Frontend components (`apps/web`) must never query the database directly or import server database drivers.
   - All data mutation and access must evaluate against Cedar authorization policies.
   - Appliance services must deploy via standard Compose configurations, never nested hypervisors or custom bash daemons.
3. **Governance Shapes:**
   - All compliance, control baselines, and roles must validate against NIST OSCAL 1.1.2 JSON schemas.
   - No un-schematized JSON or ad-hoc data structures.

## What must never happen

- Merging a PR that introduces an unvetted third-party dependency
- Bypassing Cedar policy checks in any API or data route
- Introducing non-standard container execution requirements or root host privilege escalation
