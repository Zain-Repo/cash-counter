# Company setup, sign-in, settings, and reports

Cash Ledger keeps company configuration, local credentials, and the cash ledger in the
same SQLite file. Existing installations retain their registers and movements;
company setup adds its own tables and does not change historical cash records.

## First use and sign-in

Complete company setup once, including the company name and an administrator
username and password. Passwords require at least 6 characters. Credentials
are stored as salted Argon2id password hashes, rather than plaintext.
Later launches require sign-in before displaying the workspace. Sign out when
leaving the register unattended.

Setup and sign-in forms scroll vertically when they exceed the window height.
Use the mouse wheel, trackpad, or vertical scrollbar to reach the bottom controls.

There is one local administrator account. This is application access control,
not database encryption: protect the Windows account and database backups.
There is no cloud identity service or email password recovery. Preserve your
administrator credentials; do not delete the ledger to reset a password.

## Company settings

Company settings include the name, address, written cash-handling rules, closing
variance warning tolerance, printer name, and A4 or Letter paper size. Written
rules are guidance for staff. The numerical tolerance warns during closing
review; the operator can still explicitly confirm the reconciled count.
Amounts and registers continue to use CAD with two decimal places.

## Live dashboard and backups

The Dashboard shows CAD activity across registers for the last 7, 30, 90, or 365
inclusive business dates. It includes incoming and outgoing totals, net movement,
movement counts, open days, daily net-flow and register charts, and the twenty
largest closing count differences. Opening cash is not revenue; net movement is
cash received minus cash paid out. Closing differences are signed and can offset
each other in the summary.

Analytics use an independent read-only connection and a consistent snapshot.
Only aggregate chart points and bounded detail lists return to the UI. Refresh
runs in the background every fifteen seconds after the previous read completes
while the dashboard is active and signed in. Query timing describes that read,
not a guarantee for all database sizes. Queries have a ten-second execution
deadline and a two-second lock timeout. Errors preserve the previous snapshot.

### Backup location and schedule

Open **Settings > Backups** and choose an existing folder with **Browse folder**,
or enter its full path. The folder must be writable. Save the backup settings
before running a backup; selecting a folder alone does not change the saved policy.

Manual mode creates a snapshot when you select **Back up now**. Enable
**Automatic backups** and choose Hourly, Every 6 hours, Daily, or Weekly for a
schedule. Saving a new automatic policy makes its first backup due immediately.
Subsequent intervals run from the last successful completion. Manual backups
remain available in automatic mode and also advance that schedule.

Scheduling runs while Cash Ledger is open and signed in, across all workspace
pages. Due work is checked at sign-in and every thirty seconds; busy operations
can delay it. Missed intervals produce one catch-up backup rather than one file
for every missed interval. Backups do not run after sign-out or while the app is
closed. Failed automatic backups retry after five minutes. Settings shows the
last successful backup, next scheduled attempt, and any error, with a manual
retry action. Preferences and recent outcomes persist in the ledger database.

Each run creates a uniquely named, consistent online SQLite snapshot, including
committed records in the WAL. A temporary snapshot is checked for integrity
before publication. Existing backups and live SQLite sidecars are never replaced.
A database lease prevents overlapping backup runs from multiple application
instances and expires after an interrupted run. Backup work runs in the
background, and application updates wait for it to finish.

The dashboard's **Backup settings** shortcut opens the Backups settings page.
Backup files include company credentials and financial data. Keep the folder
private and use a separate drive for protection against local disk failure.
Files are retained until you remove them yourself; automatic pruning and encryption
are not included.

### Restore data

In **Settings > Backups > Restore data**, select **Use last backup**, choose a
backup file, or choose a folder. A folder selects its latest generated backup;
the confirmation shows the exact file before any data is replaced. Folder selection
uses the most recently modified generated snapshot and excludes pre-restore safety copies.

Save or discard unsaved work first. **Restore data** replaces the ledger, company
details and administrator accounts after confirmation. The backup is validated
before restoring, and a verified `cash-counter-pre-restore-*` safety snapshot of
the current ledger is saved beside the live database. Current backup preferences
are preserved. Close other app instances before restoring so SQLite can lock the
ledger exclusively. A successful restore signs you out; use the credentials from
the restored backup to sign in again.

### Settings navigation

Settings uses the GPUI component library's searchable sidebar with pages for
Company, Cash handling, Printing, Backups, and Updates. Company, cash handling,
and printing pages each have a save action for the shared company preferences. Backup edits have their own
save and discard actions; update preferences apply immediately. Unsaved backup
preferences and a pending restore selection block an automatic application restart.

The isolated synthetic workload example is described in [Performance testing](performance.md).

## Reports and printing

Reports select an inclusive range of business dates and optionally a register.
They include session opening cash, cash in, cash out, expected cash, physical
closing count and difference, plus movement details from both directions.
Open sessions have no closing count or difference yet. Report queries read a
consistent SQLite snapshot. Requests exceeding 10,000 sessions or 100,000
movements must use a narrower range, rather than silently truncating results.

CSV exports label financial columns in integer currency minor units (100 means
CAD 1.00). Text fields are quoted and escaped, and potentially executable
spreadsheet formulas are prefixed with an apostrophe. Treat exported files as
company financial data. Exports require a `.csv` filename, reject database and
SQLite destinations, and replace the selected CSV through a temporary file.
The on-screen preview shows up to 100 sessions; CSV and printed reports include
the complete matching result within the report size limits.

Native printing is supported on Windows. Leave the printer name blank to use
the Windows default, or supply the installed printer's exact name. Configure
the paper size in Settings. Reports render through the Windows printer driver;
they do not send raw text to a printer or depend on an external document viewer.
The driver may display additional UI, including a destination prompt for a PDF
printer. A successful print submission does not prove paper was produced;
check the Windows print queue for driver or device failures.

Printing and database work run on the background executor. Other platforms can
export CSV but do not support this Windows printing implementation.

## Implementation references

- [RustCrypto Argon2](https://docs.rs/argon2/0.5.3/argon2/) documents PHC password
  hashing and verification with random salts.
- [Windows GDI print lifecycle](https://learn.microsoft.com/en-us/windows/win32/printdocs/gdi-print-api-functions)
  documents document/page start, completion, and abort operations.
- [StartDocW](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/nf-wingdi-startdocw)
  starts a driver-rendered print job.
- [windows-sys printing bindings](https://docs.rs/windows-sys/0.61.2/windows_sys/Win32/Graphics/Printing/index.html)
  expose the Windows printer APIs to Rust.
