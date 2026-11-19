# Playtest on Google Cloud

A throwaway copy of the whole product on Cloud Run, for people who should not have to build it. Running it on your own
machine is in [playtest.md](playtest.md). What to try is there too.

## What it is

- One Cloud Run service, `scaffoldry-playtest`, with one instance.
- The instance runs the one image (API and web app) and a Postgres 16 sidecar. The database lives inside the
  instance. **Everything is lost when the service is redeployed, hibernated, or restarts.**
- At start it seeds the demo data and prints a sign-in token to the service log. The token lasts 24 hours.
- Only Google accounts you allow reach it, through Identity-Aware Proxy (IAP).
- It stays up while deployed, about 2 vCPU and 2 GiB in total, so it costs money for every hour it runs. Hibernate it
  when the session ends.

## One-time setup (someone with access to the GCP project)

The workflow reuses the Workload Identity Federation login of the existing deploy workflow: the repository variables
`GCP_PROJECT_ID`, `GCP_WIF_PROVIDER`, `GCP_SERVICE_ACCOUNT`, `GCP_REGION`, and `GCP_ARTIFACT_REGISTRY`.

```sh
PROJECT=<project id>; REGION=<region>; SA=<the GCP_SERVICE_ACCOUNT email>

# Identity-Aware Proxy for Cloud Run.
gcloud services enable iap.googleapis.com --project="$PROJECT"

# The deploy identity needs to deploy and to act as the runtime service account. It never needs Owner or Editor.
gcloud projects add-iam-policy-binding "$PROJECT" --member="serviceAccount:$SA" --role=roles/run.developer
gcloud projects add-iam-policy-binding "$PROJECT" --member="serviceAccount:$SA" --role=roles/iam.serviceAccountUser
gcloud projects add-iam-policy-binding "$PROJECT" --member="serviceAccount:$SA" --role=roles/artifactregistry.writer
```

After the first deploy, let each playtester through IAP, and let the IAP service agent call the service:

```sh
gcloud iap web add-iam-policy-binding --resource-type=cloud-run --service=scaffoldry-playtest --region="$REGION" \
  --member="user:someone@example.edu" --role=roles/iap.httpsResourceAccessor
NUMBER=$(gcloud projects describe "$PROJECT" --format='value(projectNumber)')
gcloud run services add-iam-policy-binding scaffoldry-playtest --region="$REGION" \
  --member="serviceAccount:service-$NUMBER@gcp-sa-iap.iam.gserviceaccount.com" --role=roles/run.invoker
```

## Start a playtest

1. In GitHub, open Actions, choose **GCP Playtest**, then **Run workflow** with `deploy`. It builds the image (about
   15 minutes) and deploys it.
2. The run's summary shows the URL.
3. In Cloud Logging for `scaffoldry-playtest`, search for `PLAYTEST SIGN-IN TOKEN`. The line after it is the token.
4. Open the URL, sign in with your Google account at the IAP screen, then paste the token into "Paste your token".

## End it

Run **GCP Playtest** with `hibernate`. The service is deleted and nothing keeps running. The image stays in Artifact
Registry.

## Known limits

- **One instance.** A second instance would have its own, different database, so scaling is turned off.
- **No uploads after a restart.** Uploaded files live in the instance's memory, like the database.
- **The token is in the logs.** Anyone who can read this service's logs can sign in, so keep log access to the
  people running the playtest.
- **Not built here.** This deploy has not run yet, because the build environment has no Docker and no GCP access.
  The first run may need a fix.
