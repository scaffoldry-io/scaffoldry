# Scaffoldry

**The AI-native decision and application platform for distributed authority environments.**

Scaffoldry bridges the gap between end-user empowerment and institutional data integrity. Designed specifically for higher education, research institutes, and federated organizations, Scaffoldry allows departments to build custom business workflows with AI while ensuring institutional coordination, data sovereignty, and compliance.

## Core Capabilities

- **AI-Native App Creation:** Natural-language to relational tables, forms, and workflows—empowering non-technical domain experts to build departmental solutions.
- **Federated Decision Authority:** Built-in business approval workflows ("The Desk") that gate changes, track schema drift, and enforce data boundaries without bureaucratic delay.
- **Tamper-Proof Audit Ledger:** Every schema change and business decision is cryptographically recorded in an immutable Git history.
- **Enterprise Compliance by Default:** Machine-readable NIST OSCAL compliance artifacts for seamless integration with institutional GRC and audit tools.
- **Sovereign, Non-Rugpullable Architecture:** Built entirely on foundation-governed open source (Apache 2.0 / BSD / MIT) with zero cloud vendor lock-in.

## Architecture

| Layer | Technology | Purpose |
| :--- | :--- | :--- |
| **User Interface** | TanStack (React + Table + Query + Form) | High-performance, reactive data grid and decision desk |
| **Access & Policy** | Cedar | Real-time fine-grained authorization (ABAC/RBAC) |
| **Calculation Engine** | Apache Arrow & DataFusion | In-memory spreadsheet formulas and fast analytical queries |
| **Durable Store** | PostgreSQL | Enterprise ACID storage for operational records |
| **Audit Ledger** | Embedded Git | Cryptographic audit trail of all approved decisions |
| **Compliance Standard** | NIST OSCAL | Machine-readable security and compliance documentation |

## License

Apache License 2.0. See [LICENSE](LICENSE) for details.
