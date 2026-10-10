---
name: cash-ledger
description: Manage Cash Ledger's local cash registers, cash movements, daily reconciliation, company settings, reports, analytics, backups, restore, and updates. Use when the user asks to work with Cash Ledger or their local cash register ledger.
---

# Cash Ledger

Use the plugin's MCP tools to operate the application's existing services. Discover
the current tools and their schemas; do not issue SQL or edit database files.

Start with the status tool. Complete setup only when the user requests it on an
unconfigured ledger. Sign in with the user's local administrator credentials before
reading financial data or changing settings. Never save passwords in skills,
scripts, notes, or logs. Do not echo credentials back in a response.

Amounts use integer CAD cents: CAD 12.34 is 1234. Confirm the business date,
register, direction, amount, and reason from the user's request. Use the same
request identifier when retrying a financial submission so a retry cannot create
a second movement. Paginate history when the result has more records.

For closing, obtain a preview of the expected balance and discrepancy first.
Show the preview to the user and obtain their confirmation before committing
closure. For restore, resolve and validate the selected backup, show the exact
file, and obtain confirmation before replacing the ledger. Restore signs out;
sign in again with the restored administrator credentials. Treat a stale preview
as a reason to obtain and review a fresh preview.

Reports use inclusive business dates. Narrow oversized report ranges. CSV values
are integer cents. Export only to the user's selected CSV destination. Printing
submits a Windows print job; report submission separately from physical output.

Save backup settings before running a backup. Automatic backups depend on an
active signed-in application or plugin process. Explain any runtime limitations
returned by a tool. Never interpret an update check or download as a successful
installation. Installing an app update requires an explicit user request and
the tool's confirmation flow; preserve unfinished work in the desktop app.

Use read-only tools for reviews unless the user asks for changes. After an action,
report the saved result, any error, and any remaining step the user must take.
