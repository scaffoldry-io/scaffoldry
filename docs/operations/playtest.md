# Playtest

How to run the whole product on one machine and what to try. The commands were checked on a clean checkout.

## Start it

You need PostgreSQL 16, Node, and Rust. The commands assume a local cluster on port 5433 with the `scaffoldry` user that the tests use. Change `DATABASE_URL` for your own setup.

```sh
# 1. A fresh database
pg_ctlcluster 16 scaffoldry start
su postgres -c "psql -p 5433 -c 'create database playtest owner scaffoldry'"

# 2. Build the web app and the server (from the repository root)
(cd apps/web && npm ci && npm run build)          # writes apps/web/dist
cargo build -p scaffoldry-server                   # debug build is fine

# 3. Settings and one-time files
export DATABASE_URL=postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/playtest
mkdir -p /tmp/pt/drop /tmp/pt/files
openssl rand -base64 32 > /tmp/pt/master.key && chmod 400 /tmp/pt/master.key
export SCAFFOLDRY_DROP_DIR=/tmp/pt/drop SCAFFOLDRY_FILES_DIR=/tmp/pt/files SCAFFOLDRY_MASTER_KEY_FILE=/tmp/pt/master.key

# 4. Demo data, a sign-in token, and the server
B=target/debug/scaffoldry-server
$B seed-demo
$B setup-token          # prints a token that starts with scf_. It lasts 24 hours.
SCAFFOLDRY_WEB_DIR=$PWD/apps/web/dist PORT=8080 HOST=127.0.0.1 $B
```

Open http://localhost:8080/. Paste the `scf_` token into "Paste your token" and choose "Sign In". You are the setup identity, a Platform Admin.

Each of the optional settings switches on one feature:

| Setting | Without it |
| --- | --- |
| `SCAFFOLDRY_DROP_DIR` | The demo data connections are not created. Set it before `seed-demo` |
| `SCAFFOLDRY_FILES_DIR` | File uploads are off |
| `SCAFFOLDRY_MASTER_KEY_FILE` | The secrets page says the store has no master key |

A new `setup-token` revokes the one before it. To start again, drop the `playtest` database and repeat from step 1.

## What to try

1. **The core loop.** On the desk, choose "Co-Build New App". Open the app and choose "+ Add Record". Fill in the required field and save. Reload the page: the record is still there. Open it and edit it, then open its history.
2. **The intake form.** Open the app's Intake Form tab and submit it. The confirmation shows the record id the server stored. The record appears in the grid.
3. **The studio.** Choose "Configure in Studio". Add a plain field, such as a Date, and rename the app. Save. Reload: both changes are kept.
4. **The demo app.** Open `physics-admissions-review` and add an application.
5. **The admin console.** Open each section from the navigation. Try a change that asks for a reason: disable and enable a data connection, or change a platform setting.
6. **A phone.** Narrow the window to about 390 px. The navigation starts collapsed and wide tables scroll inside their box.

## Known gaps

- **Studio pages.** Changes to page layouts on the studio canvas are not stored yet. The studio says so when you save.
- **Studio views and domain.** Saved views and filters in the studio are local to the browser. The custom domain field saves nothing.
- **The People page** is empty, because the only person is the setup identity.
- **Templates.** "From a template" lists none.
- **Admin-console phases 5 to 7** are waiting on a stale branch to be removed.

Write down anything that surprises you: the page, what you did, and what you expected.
