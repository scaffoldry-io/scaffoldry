# Agent instructions and repository governance

This repository enforces strict sovereign software architecture, NIST OSCAL compliance, and disciplined engineering practices. All agents working in this repository—whether executing inside the Podman development container or running supervisory tasks on the host—must follow the rules defined here.

---

## 1. Operating Planes & Boundaries

1. **Host Plane (Sysadmin / Platform Governor):**
   - Operates on the host outside the container.
   - Manages GCP CI/CD infrastructure, performs PR code reviews, audits drift, and handles release merges.
   - Controls cloud appliance lifecycle and budget hibernation (see `.agents/skills/gcp-cicd-management/SKILL.md`).
   - Guarded by GitHub branch protection on `main`.

2. **Container Plane (Worker / Executor):**
   - Operates inside the rootless Podman container (`scaffoldry-dev`) mounted to `/workspace`.
   - Has no access to host secrets, host SSH keys, host systemd, or parent directories.
   - Works strictly on feature branches (`feat/*`, `fix/*`). Direct pushes to `main` are refused.

---

## 2. Mandatory Core Doctrine

### Least Software (`least-software`)
- Reach for the operating system, container runtime, and standard library before adding third-party dependencies.
- One line is always better than fifty lines of custom abstraction.
- Enforce YAGNI: question whether the feature needs to exist at all.

### Governance Data is OSCAL (`governance-data-is-oscal`)
- NIST OSCAL 1.1.2 is the universal lattice for all governance, controls, baselines, and roles.
- Never invent a field for something OSCAL names. Extend through props and links only.
- All governance files must pass automated schema validation.

### Standards First (`standards-first`)
- Adopt published standards (NIST, ISO, ITIL, CIS, OSCAL) before inventing vocabularies or data shapes.
- Record the standard search before defining new models.

### The Reader is an Engineer (`the-reader-is-an-engineer`)
- Every proposal must carry an enterprise objection list written from an institutional security/engineering team perspective.
- Objections must be answered with empirical facts, not hand-waving.

### Test-Driven Development (`test-driven-development`)
- Write the failing test first, verify that it fails for the expected reason, write minimal code to pass, and refactor.
- Features are never declared complete without automated test evidence in the active session.

### Data Has a Format (`data-has-a-format`)
- Structured data gets a published schema before anything reads it. Prose is for people.
- Never parse markdown tables or front-matter with regular expressions.

### Write for People (`write-for-people`)
- Short, direct sentences. One idea per sentence.
- Plain English for institutional operators and decision-makers. No dense prose or nested semicolon chains.

---

## 3. What Must Never Happen

- Pushing directly to `main` without a reviewed pull request and passing CI
- Introducing third-party dependencies with licenses outside the approved whitelist (Apache 2.0, MIT, BSD-3, PostgreSQL License only; no AGPL/SSPL/BSL)
- Frontend components directly querying PostgreSQL or bypassing Cedar authorization policies
- Writing bespoke hypervisor wrappers, nested VMs, or background bash token scrapers
- Proposing or merging code with unverified test claims

---

## 4. Skills Registry

Detailed procedures, checklists, and references are organized under `.agents/skills/`:
- `code-review-gate`: Pre-merge inspection protocol and 6-line proposal header
- `drift-containment`: License and architectural boundary auditing
- `gcp-cicd-management`: Workload Identity Federation, Artifact Registry, and Cloud Run pipelines
- `least-software`: Anti-bloat, standard library priority, and YAGNI rules
- `test-driven-development`: Red-Green-Refactor testing cycle
- `systematic-debugging`: Root-cause isolation and reproduction playbooks
- `governance-data-is-oscal`: NIST OSCAL 1.1.2 schema compliance
- `standards-first`: Ecosystem and international standard adoption
- `the-reader-is-an-engineer`: Enterprise objection list authoring
- `data-has-a-format`: JSON schema and structured data rules
- `repository-practice`: Git hygiene, conventional commits, and clean history
- `write-for-people`: Style guide for clarity and human readability
- `measurement-discipline`: Empirical verification before asserting facts
- `test-at-the-customers-scale`: Multi-tenant, enterprise-scale stress testing
