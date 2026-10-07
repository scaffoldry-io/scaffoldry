# Scaffoldry Platform Architecture

**Version:** 2.0 (Clean Foundation)  
**Date:** October 2026  
**License:** Apache License 2.0  
**Website:** https://scaffoldry.io  
**Repository:** https://github.com/scaffoldry-io/scaffoldry

---

## 1. Executive Summary & Purpose

Scaffoldry is an open-source, AI-native decision and application platform designed for **distributed authority environments**, such as higher education, academic medical centers, and research institutes.

### The Institutional Problem
In distributed institutions, central IT and individual departments exist in structural tension:
- **Central IT** requires security, regulatory compliance (FERPA, HIPAA, NIST SP 800-171), and institutional data coordination. This often leads to rigid ERP implementations that take years to adapt to departmental needs.
- **Deans, Department Chairs, and Lab Directors** hold autonomous decision authority and need immediate business solutions. Facing central IT delays, they adopt commercial SaaS tools (Airtable, Smartsheet, Google Sheets).
- **The Failure Mode:** Data silos, unmonitored compliance exposure, vendor lock-in, and unpredictable per-seat pricing.

### The Scaffoldry Solution
Scaffoldry provides an institution-owned appliance that gives end users the power to build custom business applications with AI, while maintaining hard boundaries on data integrity, decision authority, and regulatory compliance.

---

## 2. Core Architectural Principles

All development in this repository must adhere to four governing principles:

### 2.1 Sovereign and Non-Rugpullable
Scaffoldry must remain free from commercial license traps. 
- Build only on technologies governed by neutral non-profit foundations (Apache Software Foundation, Linux Foundation, PostgreSQL Global Development Group).
- Use strictly permissive licenses: **Apache 2.0, MIT, and BSD**.
- Never introduce dependencies governed by venture-backed open-core licenses (BSL, SSPL, or proprietary enterprise tiers).

### 2.2 The Modular Monolith Appliance
Scaffoldry ships as a self-contained appliance that runs on **one standard Linux machine** that existing enterprise sysadmins already know how to patch.
- Deployable via standard Docker Compose.
- Zero nested hypervisors, zero QEMU virtualization, and zero complex hyper-orchestration.
- No dependency on proprietary cloud serverless runtimes. An institution must be able to run Scaffoldry entirely inside their own on-prem firewall or on a sovereign cloud VM.

#### 2.3 Separation of Concerns
The platform cleanly separates four critical duties:
1. **Calculation:** Performed in-memory by `scaffoldry-engine` for microsecond spreadsheet interactivity.
2. **Durability:** Handled by an ACID-compliant PostgreSQL 17 relational database.
3. **Authorization:** Decided by an embedded Cedar policy engine at the API boundary.
4. **Audit & Governance:** Permanently recorded in an immutable SHA-256 cryptographic decision ledger.

### 2.4 Write for People
All documentation, user interfaces, error messages, and commit histories must follow the `write-for-people` standard:
- One idea per sentence.
- Use plain, precise language instead of academic or artificial jargon.
- State instructions before explanations.
- Name the actor and the action explicitly.

---

## 3. The Six-Layer Architecture

```
+-----------------------------------------------------------------------------------------+
|                                    LAYER 1: USER INTERFACE                              |
|                          React 19, Tailwind CSS & TanStack Query/Table                  |
|                                                                                         |
|       * Reactive Spreadsheet Grid           * Dynamic Application Forms                 |
|       * Business Approval Desk              * Natural-Language AI Builder               |
+-----------------------------------------------------------------------------------------+
                                             |
                                   (HTTP REST / JSON-RPC)
                                             v
+-----------------------------------------------------------------------------------------+
|                                LAYER 2: ACCESS & POLICY                                 |
|                                   Cedar Policy Engine                                   |
|                                                                                         |
|       * Sub-millisecond authorization checks for every cell, view, and action           |
|       * Clear separation of departmental authority vs. central IT controls              |
|       * Declarative rules written in plain policy syntax                                |
+-----------------------------------------------------------------------------------------+
                                             |
                                             v
+-----------------------------------------------------------------------------------------+
|                               LAYER 3: CALCULATION ENGINE                               |
|                         Sovereign Rust Engine (scaffoldry-engine)                       |
|                                                                                         |
|       * In-memory typed representation of manifests, views, and datasets                |
|       * Real-time formula dependency graphs, relational rollups, and lookups            |
|       * High-speed analytical evaluation without database lockups                       |
+-----------------------------------------------------------------------------------------+
          |                                                               |
          | (Sync Records)                                                | (Record Approved Decisions)
          v                                                               v
+------------------------------------+           +----------------------------------------+
|      LAYER 4: DURABLE STORE        |           |       LAYER 5: DECISION LEDGER         |
|            PostgreSQL 17           |           |      Cryptographic SHA-256 Chain       |
|                                    |           |                                        |
|  * Persistent disk storage         |           |  * Immutable cryptographic block chain  |
|  * Normalized core tables          |           |  * Sequential SHA-256 hash validation  |
|  * JSONB for dynamic user fields   |           |  * Tamper detection on server boot     |
|  * Standard daily pg_dump backups  |           |  * OSCAL AU-02 audit record continuity |
+------------------------------------+           +----------------------------------------+
                                             |
                                             v
+-----------------------------------------------------------------------------------------+
|                               LAYER 6: COMPLIANCE STANDARD                              |
|                                     NIST OSCAL 1.1.2                                    |
|                                                                                         |
|       * Machine-readable catalogs, profiles, and System Security Plans (SSPs)           |
|       * Direct export for university CISOs, auditors, and enterprise GRC tools          |
|       * Pre-mapped baselines: FERPA, HIPAA, NIST SP 800-171, and CMMC                   |
+-----------------------------------------------------------------------------------------+
```

---

## 4. Layer Responsibilities & Implementation Details

### Layer 1: User Interface
- **Technology:** React 19, `@tanstack/react-table`, `@tanstack/react-query`, and Tailwind CSS.
- **Responsibilities:**
  - Render high-density, virtualized spreadsheet grids that stay smooth at 10,000+ rows.
  - Expose the **Business Approval Desk**: an intuitive, non-technical dashboard where Department Chairs and compliance officers approve or reject proposed changes.

### Layer 2: Access & Policy (Cedar)
- **Technology:** Cedar Policy Engine (Apache 2.0, Rust).
- **Responsibilities:**
  - Execute real-time Attribute-Based Access Control (ABAC).
  - Enforce data sensitivity gates (e.g., student financial data is hidden from external faculty reviewers).
  - Evaluate decision approval authority across workspaces, records, apps, and datasets.

### Layer 3: Calculation Engine (Sovereign Rust Engine)
- **Technology:** `scaffoldry-engine` (native Rust).
- **Responsibilities:**
  - Hold active sheet schemas, views, and field specifications in memory.
  - Recompute calculated formulas, relational rollups, and lookups across columns in RAM in under 5ms.
  - Eliminate the need to issue complex, blocking SQL queries to PostgreSQL during interactive user editing.

### Layer 4: Durable Operational Store (PostgreSQL)
- **Technology:** PostgreSQL 17 (official image on Alpine Linux).
- **Responsibilities:**
  - Provide durable ACID transactions for live data.
  - Store core institutional entities (Workspaces, Collaborators, App Manifests, Published Datasets, Relationships, Automations, Sessions, SCIM) in normalized tables with advisory-lock migrations.
  - Store dynamic, user-defined fields in indexed `JSONB` columns with generated expression indexes.

### Layer 5: Decision & Audit Ledger (Cryptographic SHA-256 Blockchain)
- **Technology:** Sovereign Cryptographic Ledger (`governance_ledger` in PostgreSQL 17).
- **Responsibilities:**
  - Maintain an immutable, tamper-evident SHA-256 hash chain from genesis block 0 through block N.
  - Every publication, DNS binding, policy revision, and workflow approval appends a block linking `previous_hash` and computing `entry_hash`.
  - On every server boot, verify the cryptographic integrity of the entire chain. Refuse to start if any block hash mismatch or discontinuity is detected.
  - Provides a permanent cryptographic paper trail that aligns with NIST OSCAL AU-02.

### Layer 6: Compliance Standard (NIST OSCAL)
- **Technology:** NIST OSCAL 1.1.2 JSON Schema.
- **Responsibilities:**
  - Serve as the export lattice for institutional compliance reporting.
  - Allow university compliance officers to generate automated System Security Plans (SSPs) directly from the running appliance.
  - Validated via automated CI scripts (`governance/scripts/validate-oscal.py`).

### Standardized Interface: Model Context Protocol (MCP) as Authoritative API
- **Standards Search & Alignment:**
  - Core Protocol: Model Context Protocol (MCP) specification version `2024-11-05` over JSON-RPC 2.0.
  - Interactive UI Extension: MCP Apps extension (`modelcontextprotocol/ext-apps`, SEP-1865).
  - UI Resource Scheme: `ui://` URI scheme serving sandboxed HTML (`text/html;profile=mcp-app`) with bidirectional AppBridge messaging.
- **Responsibilities:**
  - Expose every platform capability (workspaces, apps, tabular records, datasets, audit ledger, and policy simulation) as an MCP tool or resource.
  - The MCP server is the single authoritative API for the platform.
  - REST endpoints exist as a thin compatibility adapter delegating to the same shared service layer.
  - Every MCP tool execution and resource read requires an authenticated session and evaluates Cedar ABAC before execution.

---

## 5. Deployment and Operations

Scaffoldry deploys as a single compose stack:

1. **Host Environment:** Any standard Linux distribution (Ubuntu 24.04 LTS recommended) with 2 vCPUs and 4 GB RAM.
2. **Public Ingress (Zero Open Inbound Ports):**
   - Deployed behind a **Cloudflare Tunnel** (`cloudflared`).
   - All inbound firewall ports (80, 443, 22) remain closed on the public interface.
   - Cloudflare provides edge TLS termination, DDoS defense, and optional Zero Trust access control.
3. **Automated Off-Machine Backup:**
   - A daily timer executes a snapshot script.
   - Dumps PostgreSQL (`pg_dump`) and bundles the local `.git` ledger.
   - Encrypts the snapshot archive and uploads it to off-machine object storage (Backblaze B2, Google Cloud Storage, or Proton Drive).

---

## 6. Application Lifecycle: Review, Deployment, and Security Maintenance

Applications created on Scaffoldry (e.g., Departmental Admissions Review, Grant Tracking) are backed by declarative definitions in Git. This architecture directly enables professional code review, automated security scanning, and fleet maintenance over time without subjecting end users to developer friction.

### 6.1 The Governed Review Workflow

Changes to an application never push directly to live data. Every creation, formula update, or schema alteration follows a governed path:

1. **The Proposal (Branch):** When an end user or AI modifies an app, the platform generates a Git branch: `proposal/<change-summary>`.
2. **Automated Pre-Review Checks:** Before notifying a human reviewer, the platform runs automated checks:
   - **Data Sensitivity Scan:** Scans the schema and fields to detect FERPA, HIPAA, or restricted research data markers.
   - **Policy Evaluation:** Asserts that the requested changes comply with institutional rules via Cedar.
   - **Compliance Mapping:** Verifies that new data structures map to the relevant OSCAL catalog controls.
3. **The Human Review on The Desk:** Reviewers (such as Department Chairs or Compliance Officers) review the proposed changes through a clear business UI on The Desk:
   - Visual before-and-after view of the schema and formula changes.
   - Data sensitivity classification and required approvals.
   - Automated check results (Pass/Fail).
4. **Governed Deployment:** When approved, the system merges the branch into `main` and signs the commit. The production database and calculation engine reload the new schema instantly.

### 6.2 Long-Term Security Maintenance & Fleet Patching

In legacy low-code tools, departmental apps become unmaintained legacy liabilities. In Scaffoldry, apps are maintained as version-controlled repositories:

1. **Upstream Security Patches:** When Scaffoldry Core publishes security patches, dependency upgrades, or formula engine fixes, the system automatically opens patch proposals across the application fleet.
2. **Automated Regression Testing:** The system tests the departmental application against the patch in an isolated sandbox to verify that existing formulas, rollups, and views continue to calculate correctly.
3. **One-Click Approval:** Departmental administrators receive a prompt on The Desk: *"Security patch available for Admissions App. Regression tests: 100% Passed. [Apply Update]"*.

### 6.3 Institutional Deployment Modes

Scaffoldry supports two operating modes to accommodate both non-technical departments and enterprise IT teams:

- **Mode A: The Invisible Git Workflow (Default):**
  Faculty, staff, and departmental administrators interact entirely through The Desk. Embedded Git manages branches, approvals, and merges behind the scenes. Users experience the collaborative ease of low-code without developer tooling.
- **Mode B: Enterprise Git Sync (Central IT Integration):**
  The platform syncs departmental application repositories with the university's existing GitHub Enterprise or GitLab infrastructure.
  - Central IT security tools (Dependabot, Snyk, CodeQL, Trivy) continuously scan application code and dependencies.
  - Central enterprise engineers can review and audit departmental applications through standard GitHub Pull Requests.

---

## 7. Guidelines for AI Agents Working in this Repository

Future AI assistants working on Scaffoldry must strictly follow these instructions:

1. **Never introduce hypervisor or VM nesting.** Do not suggest QEMU, nested libvirt, or host-level Podman quadlet workarounds. Everything runs in standard, portable containers.
2. **Never store long-lived credentials in scripts.** Never create credential-minting bash scripts or SSH scrapers. Use standard environment variables and scoped service tokens.
3. **Never bypass human approval for production changes.** Production deployments and schema modifications must always originate from an approved merge into `main`.
4. **Never write raw SQL migrations for user-defined fields.** User-created columns live in flexible JSONB/Arrow columnar memory; do not run dynamic `ALTER TABLE` DDL queries against PostgreSQL.
5. **Adhere to Write for People.** Write simple, clear prose. Avoid dense sentence structures, pseudo-philosophical aphorisms, and nested subordinate clauses.

