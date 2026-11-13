# Shared links

A shared link opens one view. It is off until a Platform Admin turns on a scope.

## Turn it on

A Platform Admin sets `sharing.allowed_scopes` in the settings. It is a list of `institution`, `public`, or both. The default is empty, so no link can be made.

## Two kinds of link

| Scope | Who can open it | Rows it shows |
| --- | --- | --- |
| `institution` | Anyone signed in who may already read the app | The viewer's own rows. Their row rules apply, and the workspace's guards can still forbid |
| `public` | Anyone with the link | Only rows that do not depend on who is looking |

A link adds no one to an app. An institution link opens for a person who could read the app anyway. A guard that forbids a unit blocks that unit's people from the link too.

A public link has no person behind it. A row rule that depends on a person shows no rows. The person who makes the link is told when that is so.

A public link needs a workspace classified `Public`. Otherwise it is refused with `share_not_allowed`.

## Rules for every link

- The token is 128 random bits, shown once. Only its hash is stored. The link is `/s/{token}`.
- It expires. The default is 30 days and the most is 90.
- A view holds at most 20 active links.
- A shared view is read only.
- A shared view never carries a sensitive field, and never a field the viewer's own rules hide. This holds for every link and every viewer.
- A download is off unless the link was made with `allow_download`. A public link cannot download a table that has a sensitive field.
- A personal view and a dashboard cannot be shared. Make a personal view collaborative first.
- A revoked, expired or unknown token is `404`, whatever the reason.

## Limits

Each request counts for the token: 60 a minute. A public link also counts 20 a minute for each address. Past the limit the answer is `429` with `rate_limited`. The address is the first value of `X-Forwarded-For`, so the proxy in front of the server must set it.

## Who can share

The owner or an admin of the app. They make links from the Share dialog of a view, which shows the link once and lists the active ones with `Revoke`.

## In the console

`Shares` lists every link of every app: its view, scope, who made it, when it expires, how often it was opened, and its state. `Revoke all for this app` closes every active link of one app. It asks for a reason, and the reason goes on the ledger. Making and revoking a link are on the ledger too.
