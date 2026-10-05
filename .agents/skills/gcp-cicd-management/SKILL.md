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

## Cloud Appliance Lifecycle and Cost Containment (Shutdown and Resume)

To conserve cloud budget during active local development cycles, the cloud deployment can be hibernated:

### 1. Current State (Hibernated)
- Cloud Run service `scaffoldry-desk` in `us-central1` is deleted (zero compute running, 404 returned on public endpoints).
- GitHub Actions workflow `GCP Cloud Deploy` (`.github/workflows/deploy.yml`) is disabled to prevent automated redeployment on git push to `main`.
- Core continuous integration workflow (`.github/workflows/ci.yml`) remains active to validate all PRs and commits.

### 2. How to Verify Status
```bash
# Check Cloud Run services (should list 0 items)
gcloud run services list --project=scaffoldry-io

# Check GitHub Actions deployment workflow state (GCP Cloud Deploy disabled)
gh workflow list
```

### 3. How to Bring the Cloud Appliance Back Online
```bash
# Re-enable the deployment workflow
gh workflow enable "GCP Cloud Deploy"

# Trigger an immediate build and deployment run
gh workflow run "GCP Cloud Deploy"

# Wait for deployment and verify live endpoint
gcloud run services list --project=scaffoldry-io
curl -s https://<cloud-run-url>/health
```

### 4. How to Hibernate Again
```bash
# Delete the Cloud Run service
gcloud run services delete scaffoldry-desk --region=us-central1 --quiet

# Disable automated deploy workflow
gh workflow disable "GCP Cloud Deploy"
```
