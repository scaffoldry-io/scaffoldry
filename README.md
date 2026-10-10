# Scaffoldry

**The AI-native decision and application platform for distributed authority environments.**

Scaffoldry bridges the gap between end-user empowerment and institutional data integrity. Designed specifically for higher education, research institutes, and federated organizations, Scaffoldry allows departments to build custom business workflows with AI while ensuring institutional coordination, data sovereignty, and compliance.

## Core Capabilities

- **Build With the AI Tools You Already Use:** People create and change apps directly from a desktop AI app, Claude Code, Codex, or any other tool that speaks the Model Context Protocol, signed in as themselves. There is no separate agent to deploy. Scaffoldry runs no model.
- **Federated Decision Authority:** Built-in business approval workflows ("The Desk") that gate changes, track schema drift, and enforce data boundaries without bureaucratic delay.
- **Tamper-Proof Audit Ledger:** Every schema change, DNS binding, and business decision is cryptographically recorded in a hash-chained ledger in PostgreSQL.
- **Enterprise Compliance by Default:** Machine-readable NIST OSCAL compliance artifacts for seamless integration with institutional GRC and audit tools.
- **Sovereign, Non-Rugpullable Architecture:** Built entirely on foundation-governed open source (Apache 2.0 / BSD / MIT) with zero cloud vendor lock-in.

## Architecture

| Layer | Technology | Purpose |
| :--- | :--- | :--- |
| **AI Tool Interface** | Model Context Protocol | Primary interface for people building apps and working with records from their own AI tools |
| **User Interface** | TanStack (React 19 + Table + Query) | Virtualized spreadsheet data grid, ProcessDesk queue, and admin console |
| **Access & Policy** | Cedar Policy Engine | Real-time Attribute-Based Access Control (ABAC/RBAC) |
| **Calculation Engine** | `scaffoldry-engine` (Rust) | In-memory spreadsheet formula evaluation, relational rollups, and lookups |
| **Durable Store** | PostgreSQL 17 | Enterprise ACID storage for operational records, manifests, and migrations |
| **Audit Ledger** | SHA-256 hash chain in PostgreSQL | Cryptographic audit trail of all approved decisions |
| **Compliance Standard** | NIST OSCAL 1.1.2 | Machine-readable security, governance, and compliance reporting |

## Development & Cloud Operations

### Local Development Environment
The application operates locally inside a rootless Podman development container (`scaffoldry-dev`) or directly on the host:

- **Web Frontend:** Runs on `http://localhost:5173` via Vite and TanStack.
  ```bash
  cd apps/web && npm install && npm run dev
  ```
- **Backend Engine & API Server:** Runs on `http://127.0.0.1:8080` (or `PORT` if set) via Rust Axum.
  ```bash
  cargo run -p scaffoldry-server
  ```
- **Automated Usability & Integration Tests:**
  ```bash
  # Web usability test suite
  cd apps/web && npm test

  # Engine, policy, and API test suite
  cargo test --workspace
  ```

### Cloud Appliance Lifecycle (Cost Hibernation)
To prevent unexpected cloud infrastructure costs during active development cycles, the production Cloud Run deployment is hibernated by default:

- **Current Status:** Hibernated (Cloud Run service `scaffoldry-desk` deleted, `GCP Cloud Deploy` workflow disabled).
- **Bringing Cloud Online:**
  ```bash
  gh workflow enable "GCP Cloud Deploy"
  gh workflow run "GCP Cloud Deploy"
  ```
- **Hibernating Cloud:**
  ```bash
  gcloud run services delete scaffoldry-desk --region=us-central1 --quiet
  gh workflow disable "GCP Cloud Deploy"
  ```

## License

Apache License 2.0. See [LICENSE](LICENSE) for details.
