# Connecting to a PostgreSQL source

A connection of kind `postgres` runs one `SELECT` against another PostgreSQL database and copies its rows into a read-only dataset, on the schedule you give it. This note is for the person who sets it up.

## What you need

1. **A read-only account at the source.** Make a user that can read only what the query reads. The server runs the query in a read-only transaction as well, but the account is the real limit.
2. **The password in Secrets.** Add it under Secrets in the console. The connection names the secret and never holds the password.
3. **The host on the outbound allowlist.** Add the host and port under Outbound access. A source on your own network needs a `cidr` entry that names that network on purpose. Without one, a host that resolves to a private address is refused.
4. **A certificate the server trusts.** The server trusts the certificates of its own operating system. If the source uses your institution's own authority, add that authority to the server's system store.

## The connection

```json
{
  "kind": "postgres",
  "secret": "registrar_db",
  "config": {
    "host": "db.registrar.example.edu",
    "port": 5432,
    "database": "sis",
    "user": "scaffoldry_reader",
    "tls": "verify-full",
    "query": "SELECT code, name, credits FROM programs",
    "timeout_seconds": 300
  }
}
```

- `tls` is `verify-full` or `require`. Both encrypt, and both check that the certificate comes from a trusted authority. `verify-full` also checks that it is for the host you named. There is no setting that sends anything in the clear.
- `query` must be one `SELECT` or `WITH` statement. A semicolon outside a string or comment is refused. The key column and every mapped column must be in its result.
- `timeout_seconds` is how long the query may run. The default is 300 seconds. A query that runs longer fails the run, and nothing changes.

## What a run does

- It looks the host up once and connects to that address. The name is used only to check the certificate.
- It runs the query twice inside one snapshot. The first pass checks every row and counts them, so a bad value, a repeated key, or a result under half the size of the last one stops the run before anything is written. The second pass writes only what changed, 1,000 rows at a time.
- It reads 5,000 rows at a time, so memory stays flat. A source of 200,000 rows raised the server's peak memory by about 80 MB.
- If the query gains or loses a column, the run stops and makes a proposal for the dataset, as a changed file does.

## What is not shown

The password is never in an error, a log, a run, or the ledger. An error from the source is shown in words, with the password taken out.
