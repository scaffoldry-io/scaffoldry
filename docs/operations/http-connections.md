# Connecting to a web service

A connection of kind `http` reads a JSON or JSON Lines web service and copies its rows into a read-only dataset, on the schedule you give it. This note is for the person who sets it up.

## What you need

1. **The host on the outbound allowlist.** Add the host and port under Outbound access. A service on your own network needs a `cidr` entry that names that network on purpose.
2. **A token, if the service needs one.** Add it under Secrets, and name it in the connection. The server sends it as `Authorization: Bearer ...`. It is never shown or logged.

## The connection

```json
{
  "kind": "http",
  "secret": "registrar_api",
  "config": {
    "url": "https://api.registrar.example.edu/v1/programs",
    "format": "json",
    "path": "data",
    "next": "meta.next"
  }
}
```

- `url` must be an https address, with no user name or password in it.
- `format` is `json` or `jsonl`. A `jsonl` source is one page, with one object on each line.
- `path` says where the list of rows is in a `json` answer, such as `data` or `result.items`. Leave it out when the answer is the list.
- `next` says where the answer names the next page, as a full https address. The next address goes through the same allowlist check as the first. A page with no `next`, or an empty one, is the last.

## The limits

- A redirect is a failure. It is not followed.
- A page may be 5 MB. A run reads at most 100 pages and takes at most 10 minutes.
- A page that is not JSON, or that has no list where `path` says, fails the run with a message that names the path.

## What a run does

- It reads every page first and keeps the rows in a private temporary file, so memory stays flat and the service is asked once.
- It then checks every row and counts them. A bad value, a repeated key, or a result under half the size of the last one stops the run before anything is written. It writes only what changed.
- If the rows gain or lose a key that the connection maps, the run stops and makes a proposal for the dataset, as a changed file does. A key the connection does not map and the rows no longer have is not noticed until you add it to the map.
