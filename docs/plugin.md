# Cash Ledger plugin

The local Codex plugin exposes the desktop application's Rust services over MCP
standard input and output. It uses the same SQLite ledger, company credentials,
financial validation, reporting limits, and backup/restore services as the app.
It does not run a network listener or upload the ledger to a separate service.
Tool results appear in the requesting Codex conversation.

The normal database is `%LOCALAPPDATA%/Cash Register/data/register.sqlite3`.
`CASH_REGISTER_DB` selects an isolated ledger for development, as it does in the
desktop application. Each plugin process starts signed out. Use `status`, then
`login` with the existing administrator credentials, or `setup_company` for a
new ledger. Signing into the desktop does not sign the plugin in.

`CASH_LEDGER_UPDATE_PREFERENCES` can point to an isolated update-preference file
for tests. Without that override, the plugin and desktop use the same established
`updates.json` location.

Amounts are integer CAD cents. Financial submissions use retry keys. Closing and
restore require a preview followed by explicit confirmation; a changed preview
must be reviewed again. Restore creates the app's safety backup and signs out.
Printing submits a native Windows print job, which still depends on the printer
driver and device. Update installation uses the app's existing verified Windows
installer. Close the desktop app after saving its drafts before requesting an
installation; the plugin does not force it to close.

See [feature coverage](plugin-feature-coverage.md) for the tool inventory and
verification limits. Automatic backup schedules run while a signed-in desktop or
plugin process is active. The desktop retains responsibility for its own startup
update automation and draft-safe restart policy. Plugin update requests are
explicit actions.

## Build, verify, and install

The plugin needs Windows x64, PowerShell 7, and the repository's Rust build environment to build.
The installed package includes the server executable and x64 Visual C++ runtime;
it does not need Cargo or Python to run.

```powershell
rtk cargo fmt --all -- --check
rtk cargo check --locked --all-targets
rtk cargo clippy --locked --all-targets -- -D warnings
rtk cargo test --locked
rtk proxy powershell -NoProfile -File packaging/build-plugin.ps1
rtk proxy python packaging/verify-plugin.py
rtk proxy powershell -NoProfile -File packaging/build-plugin.ps1 -SkipBuild -Install
rtk proxy python packaging/verify-plugin-host.py
```

The build creates `plugins/cash-ledger/server/`, which is excluded from Git.
Source-controlled manifests include portable Agent Plugins 1.0 metadata plus the
Codex compatibility manifest and MCP configuration. The build keeps both copies
synchronized. `-VcRuntimePath` can select the x64 redistributable SDK DLL when
Visual Studio discovery is unavailable.

Installation copies the completed package to `~/plugins/cash-ledger`, appends an
entry to the personal marketplace while preserving existing entries, and installs
it with `codex plugin add cash-ledger@local-plugins`. The installer refuses to
overwrite an existing personal source package accidentally. To update this managed
installation, use `-Install -UpdateExisting`. The installer resolves the executable
path in the personal source before installing because Windows Codex does not
expand plugin-root placeholders in executable names. Do not edit the installed cache.
The host verifier uses an ephemeral session and temporary database, disables unrelated
configured MCP servers only for that subprocess, and calls status without a model turn.

Start a new Codex chat after installation to load its tools. Invoke Cash Ledger
from the plugin picker or ask to use Cash Ledger to show register balances,
prepare a report, or review backup settings. Local stdio plugins run on this
Windows machine; they are not hosted plugins for web or mobile.
