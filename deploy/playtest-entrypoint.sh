#!/bin/sh
# Starts a throwaway playtest of the whole product on Cloud Run.
#
# The database is the Postgres sidecar in the same instance, so everything is lost when the instance stops. The master
# key is made fresh here for the same reason. The sign-in token is printed to the log once, for the people who may
# read this service's logs.
set -eu

work=/tmp/scaffoldry
mkdir -p "$work/drop" "$work/files"
head -c 32 /dev/urandom | base64 > "$work/master.key"
chmod 400 "$work/master.key"
export SCAFFOLDRY_DROP_DIR="$work/drop" SCAFFOLDRY_FILES_DIR="$work/files" SCAFFOLDRY_MASTER_KEY_FILE="$work/master.key"

# The sidecar may still be starting. Seeding is safe to repeat, so try until it works, for up to two minutes.
tries=0
until scaffoldry-server seed-demo; do
  tries=$((tries + 1))
  if [ "$tries" -ge 60 ]; then
    echo "The database did not answer within two minutes." >&2
    exit 1
  fi
  sleep 2
done

echo "PLAYTEST SIGN-IN TOKEN (valid 24 hours, paste it into 'Paste your token'):"
scaffoldry-server setup-token

exec scaffoldry-server
