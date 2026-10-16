# Reviewing the UI without a person at the screen

A browser pass by hand, or by an agent reading screenshots, costs a lot. These checks cost a few hundred tokens a run. They read the page, not a picture of it.

## Run them

Needs the server binary (`cargo build -p scaffoldry-server`), `psql`, a running Postgres, and a Chromium. Each run makes a new throwaway database, seeds it, starts the API on 8080 and the web server on 5173, and removes all of it at the end. Free those two ports first.

| Command | What it does |
| --- | --- |
| `npm run ui:audit` | Visits every screen at desktop and phone width and reports faults. Clicks every button and re-checks the page after each click |
| `npm run ui:smoke` | Sign in, reload and stay the same person, My apps, Datasets, the admin console. One PASS or FAIL line each |
| `npm run ui:mcp` | Talks to `/api/v1/mcp` the way a client does and prints one line per check, with the size of the tool list |
| `npm run ui:all` | All three |
| `npm run ui:test` | Tests the audit itself against pages with known faults |

The audit prints rules grouped, three examples each, and the path of a JSON file with every finding.

## What the audit finds

- `doubled-symbol`: a button with an icon and a label that starts with `+`, so the plus is drawn twice.
- `unnamed-control`: a button, link, or field with no text, label, title, or placeholder.
- `placeholder-text`: `undefined`, `NaN`, `[object Object]`, `lorem ipsum`, `TODO`, `coming soon` on screen.
- `overlay-overlap`, `overlay-offscreen`, `overlay-hidden`, `too-many-overlays`: popups and toasts that cover each other, sit outside the window, hide under something, or pile up. A small toast on top of a full-screen modal is fine and is not reported. A notice in the page flow is not an overlay, whatever its role. These are checked after each click, because that is when popups appear.
- `horizontal-overflow`: the page scrolls sideways.
- `clipped-text`: text cut off with an ellipsis and no tooltip.
- `duplicate-control`: the same two-word button twice on one screen.
- `dead-control`: a button whose click changes nothing: no new address, no request, no change to the page.
- `unclickable-control`: a button that cannot be clicked because something covers it.
- `request-or-console-error`: a failed request or a console error while the screen loaded.

## What it cannot do

- It checks what it has a rule for. It will not say a layout looks wrong, or that a flow is confusing. Take one screenshot of a changed screen when that matters.
- `dead-control` has false alarms. A tab or filter that is already selected changes nothing when clicked. A logo that does nothing is a finding, but a mild one. Read each before acting.
- Controls that need typed input or a chosen row are clicked empty, so their effect shows only if empty is enough.
- Delete, remove, revoke, reset, clear, discard, and sign out buttons are not clicked.

## Rules for adding a check

Write the page with the fault first, in `scripts/ui/audit.test.mjs`, and see the rule miss it. Then add the rule. A rule that cannot be shown to catch a fault is not a rule.

## MCP

See `docs/mcp-client.md`. `npm run ui:mcp` checks the endpoint over the wire, and `npm run mcp:client` runs a real Claude Code client against it.
