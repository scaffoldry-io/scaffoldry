---
name: gcp-cicd-management
description: Manages Google Cloud Platform infrastructure, Workload Identity Federation, Artifact Registry, and Cloud Run / Cloudflare deployment pipelines for Scaffoldry.
---

# GCP CI/CD management

## The rule

Cloud infrastructure is defined declaratively and securely. No long-lived service account keys are stored in GitHub secrets; authentication uses Workload Identity Federation (OIDC). Deployments must be reproducible, automated via GitHub Actions, and auditable.

## Standard architecture

1. **Authentication:** GitHub Actions authenticates to Google Cloud via Workload Identity Pools (`google-github-actions/auth`) using OIDC tokens tied to the `scaffoldry-io/scaffoldry` repository.
2. **Artifact Storage:** Multi-architecture container images (`linux/amd64`, `linux/arm64`) are built via Cloud Build or GitHub Actions and pushed to GCP Artifact Registry (`scaffoldry-docker`).
3. **Appliance Runtime:** Cloud Run services host containerized frontends and APIs with zero open inbound host ports, routing through Cloudflare Zero Trust / Cloudflare Tunnel.
4. **Secret Management:** Sensitive configurations live in GCP Secret Manager, mounted as environment variables at runtime.

## What must never happen

- Storing static GCP service account JSON private keys in GitHub repository secrets
- Granting `Owner` or unconstrained `Editor` IAM roles to automated build identities
- Deploying manual container images from local machines without Git commit provenance
