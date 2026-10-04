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

### 2.3 Separation of Concerns
The platform cleanly separates four critical duties:
1. **Calculation:** Performed in-memory for microsecond spreadsheet interactivity.
2. **Durability:** Handled by an ACID-compliant relational database.
3. **Authorization:** Decided by an embedded policy engine at the API boundary.
4. **Audit & Governance:** Permanently recorded in an immutable Git ledger.

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
|                          TanStack (React + Table + Query + Form)                        |
|                                                                                         |
|       * Reactive Spreadsheet Grid           * Dynamic Application Forms                 |
|       * Business Approval Desk              * Natural-Language AI Builder               |
+-----------------------------------------------------------------------------------------+
                                             |
                                  (JSON-RPC / WebSockets)
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
|                                Apache Arrow & DataFusion                                |
|                                                                                         |
|       * In-memory columnar representation of tables and sheets                          |
|       * Real-time formula dependency graphs (DAG) and rollups                           |
|       * High-speed analytical queries without database lockups                          |
+-----------------------------------------------------------------------------------------+
          |                                                               |
          | (Sync Records)                                                | (Record Approved Decisions)
          v                                                               v
+------------------------------------+           +----------------------------------------+
|      LAYER 4: DURABLE STORE        |           |       LAYER 5: DECISION LEDGER         |
|            PostgreSQL              |           |              Embedded Git              |
|                                    |           |                                        |
|  * Persistent disk storage         |           |  * Immutable cryptographic history     |
|  * Normalized core tables          |           |  * Proposals as branches               |
|  * JSONB for dynamic user fields   |           |  * Approvals as signed merge commits   |
|  * Standard daily pg_dump backups  |           |  * Full auditability without Git UI    |
+------------------------------------+           +----------------------------------------+
                                             |
                                             v
+-----------------------------------------------------------------------------------------+
|                               LAYER 6: COMPLIANCE STANDARD                              |
|                                        NIST OSCAL                                       |
|                                                                                         |
|       * Machine-readable catalogs, profiles, and System Security Plans (SSPs)           |
|       * Direct export for university CISOs, auditors, and enterprise GRC tools          |
|       * Pre-mapped baselines: FERPA, HIPAA, NIST SP 800-171, and CMMC                   |
+-----------------------------------------------------------------------------------------+
```

---

## 4. Layer Responsibilities & Implementation Details

### Layer 1: User Interface (TanStack)
- **Technology:** React 19, `@tanstack/react-table`, `@tanstack/react-query`, `@tanstack/react-form`, and Tailwind CSS.
- **Responsibilities:**
  - Render high-density, virtualized spreadsheet grids that stay smooth at 10,000+ rows.
  - Implement optimistic UI updates via TanStack Query: cell edits reflect immediately in the browser while syncing in the background.
  - Expose the **Business Approval Desk**: an intuitive, non-technical dashboard where Department Chairs and compliance officers approve or reject proposed changes.

### Layer 2: Access & Policy (Cedar)
- **Technology:** Cedar Policy Engine (Apache 2.0, Rust).
- **Responsibilities:**
  - Execute real-time Attribute-Based Access Control (ABAC).
  - Enforce data sensitivity gates (e.g., student financial data is hidden from external faculty reviewers).
  - Evaluate decision approval authority (e.g., only designated Chairs can approve schema extensions).

### Layer 3: Calculation Engine (Apache Arrow & DataFusion)
- **Technology:** Apache Arrow (memory format) and DataFusion (query engine).
- **Responsibilities:**
  - Hold active sheet data in memory using columnar layouts.
  - Recompute calculated formulas, rollups, and lookups across columns in RAM in under 50ms.
  - Eliminate the need to issue complex, blocking SQL queries to PostgreSQL during interactive user editing.

### Layer 4: Durable Operational Store (PostgreSQL)
- **Technology:** PostgreSQL 17 (official image on Alpine Linux).
- **Responsibilities:**
  - Provide durable ACID transactions for live data.
  - Store core institutional entities (Departments, Users, Roles) in normalized relational tables.
  - Store dynamic, user-defined fields in indexed `JSONB` columns with generated expression indexes.

### Layer 5: Decision & Audit Ledger (Embedded Git)
- **Technology:** Embedded Git library (`libgit2` or `gitoxide`).
- **Responsibilities:**
  - Maintain a local `.git` repository on disk as the immutable decision log.
  - When an end user or AI proposes a schema change, the system writes a proposal file to a Git branch.
  - When a human clicks **Approve** on the Desk, the system merges the branch into `main` and signs the commit.
  - Provides a permanent, cryptographic paper trail that survives server rebuilds.

### Layer 6: Compliance Standard (NIST OSCAL)
- **Technology:** NIST OSCAL 1.1.2 JSON Schema.
- **Responsibilities:**
  - Serve as the export lattice for institutional compliance reporting.
  - Allow university compliance officers to generate automated System Security Plans (SSPs) directly from the running appliance.
  - Validated via automated CI scripts (`governance/scripts/validate-oscal.py`).

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

## 6. Guidelines for AI Agents Working in this Repository

Future AI assistants working on Scaffoldry must strictly follow these instructions:

1. **Never introduce hypervisor or VM nesting.** Do not suggest QEMU, nested libvirt, or host-level Podman quadlet workarounds. Everything runs in standard, portable containers.
2. **Never store long-lived credentials in scripts.** Never create credential-minting bash scripts or SSH scrapers. Use standard environment variables and scoped service tokens.
3. **Never bypass human approval for production changes.** Production deployments and schema modifications must always originate from an approved merge into `main`.
4. **Never write raw SQL migrations for user-defined fields.** User-created columns live in flexible JSONB/Arrow columnar memory; do not run dynamic `ALTER TABLE` DDL queries against PostgreSQL.
5. **Adhere to Write for People.** Write simple, clear prose. Avoid dense sentence structures, pseudo-philosophical aphorisms, and nested subordinate clauses.
