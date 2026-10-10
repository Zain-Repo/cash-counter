# MCP plugin feature coverage

The `cash-ledger-mcp` binary exposes Cash Ledger services over newline-delimited
JSON-RPC on standard input and output. `initialize`, `ping`, and `tools/list` are
available before sign-in. Each server process has its own authenticated state;
company setup signs in that process, while a later process must call `login`.
The integration suite runs the real binary with a temporary SQLite database,
so it does not read or modify the desktop user's ledger.

| Feature | MCP tools | Integration coverage |
| --- | --- | --- |
| Protocol and discovery | `initialize`, `ping`, `tools/list`, `status` | Protocol version, server identity, advertised schemas, setup state, notification suppression, and a valid request after tool errors |
| Company access | `setup_company`, `login`, `logout`, `company_profile`, `save_company_settings`, `change_password` | First setup, duplicate setup rejection, settings and printer preferences, wrong and correct current passwords, logout authorization, and sign-in with the changed password |
| Registers and movements | `registers`, `register_sessions`, `session_balance`, `create_register`, `open_register`, `record_movement`, `register_history` | Register and session listing, opening, balance read, invalid date rejection, idempotent retry, invalid direction, and history results |
| Closing | `close_preview`, `close_register` | Count and balance preview, stale preview rejection after a movement, and token-confirmed close with a signed difference |
| Reports, printing, and analytics | `report`, `print_report`, `export_report_csv`, `analytics_dashboard` | Date-filtered session report, CSV creation in the temporary directory, aggregate movement totals, and print rejection without explicit confirmation |
| Backups and restore | `backup_status`, `save_backup_settings`, `run_backup`, `restore_preview`, `restore_backup` | Manual destination settings, verified backup creation, validated preview, mismatch rejection, and restoring the previewed backup after an intervening register change |
| Updater | `update_preferences`, `set_update_preferences`, `update_status`, `check_updates`, `update_prepare`, `update_install` | Preference read and write in an isolated preference file, read-only local updater status, and rejection of installation without explicit confirmation |

The suite intentionally does not invoke `check_updates` or `update_prepare`,
because both contact the release service and `update_prepare` downloads an
installer. It does not invoke `update_install`, which hands off to the desktop
installer. Printing is not invoked; the printer name and paper size are tested
only as saved company preferences, and `print_report` is checked only for its
confirmation gate. Updater preferences use a temporary file in the test
process, separate from the desktop user's preference file. A timed integration
case also waits for an automatic backup while signed in and checks that a
separate due backup does not run after sign-out.

Restore uses two calls. `restore_preview` validates the chosen file or folder
and returns the selected backup path, size, digest, and a process-local preview
token. `restore_backup` requires that token and explicit `confirm: true`; it
revalidates the backup before replacing the ledger. A successful restore signs
the process out.
