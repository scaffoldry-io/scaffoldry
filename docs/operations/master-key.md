# The master key for the secrets store

The server keeps credentials for other systems in its secrets store: a webhook's signing key, a feed's password. Each one is encrypted with AES-256-GCM under one master key. The master key is never in the database. It sits in a file that you mount, and the variable `SCAFFOLDRY_MASTER_KEY_FILE` names that file.

## Make the key

Generate 32 random bytes in base64, on a machine you trust:

```sh
openssl rand -base64 32 > master.key
chmod 400 master.key
```

Mount the file read-only into every server instance, at the same path. Set `SCAFFOLDRY_MASTER_KEY_FILE` to that path. Every instance must use the same key.

## Keep it apart from the database backups

The database backup holds the encrypted secrets. The key opens them. Keep the two in different places, held by different people where you can. Someone with the backup and not the key has ciphertext only. Someone with both has every secret.

Keep a copy of the key somewhere safe and separate, such as a password manager or a sealed envelope. Lose the key and the stored secrets cannot be opened. They must then be entered again.

## What the server does at start

| Secrets stored | Key file | What happens |
| --- | --- | --- |
| None | Missing | The server starts. The secrets routes answer `503` with `master_key_missing`. |
| None | Present | The server starts. |
| Some | Missing | The server does not start. It says that secrets exist and no key file is set. |
| Some | Wrong or damaged | The server does not start. It says the key does not open the stored secrets. |
| Some | Right | The server starts. |

## Change the key

Make a new key file as above. Then run, with the server's database settings:

```sh
scaffoldry-server rotate-master-key /path/to/old.key /path/to/new.key
```

It re-encrypts every secret in one transaction. If the old key does not open every secret, nothing changes. Then point `SCAFFOLDRY_MASTER_KEY_FILE` at the new file on every instance, and restart them. Destroy the old file once the restart is done.

## Where the server may connect

The server connects out only to the hosts and networks in the setting `egress.allow`. A Platform Admin sets it. It is empty at first, so nothing leaves. Each entry is `{ "host": "hooks.example.edu", "ports": [443] }` or `{ "cidr": "10.20.0.0/16", "ports": [5432] }`. Private, loopback and link-local addresses, the cloud metadata address among them, are refused unless a `cidr` entry names that network on purpose.
