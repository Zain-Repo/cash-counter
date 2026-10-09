# Native cash-register workspace

The app uses GPUI Component through `gpui-component-uo = 1.19.0-pre` and its
matching embedded icon assets. GPUI and the platform crate are pinned to
`1.19.0-pre`. The previously resolved GPUI 1.23.2 changed `wrap_line` to require
an additional argument, breaking the component mirror's published base package.
Keep these pins together; validate upgrades as one compatible set.

Research sources: [the published component package](https://docs.rs/crate/gpui-component-uo/1.19.0-pre),
[upstream component documentation](https://longbridge.github.io/gpui-component/docs/),
and [the GPUI publisher](https://github.com/iamnbutler/gpui-unofficial).
Exact input, button, theme, root, task, and focus signatures were checked against
the installed packages. The current upstream documentation has moved to GPUI
Kit; examples from that newer API should not be copied without checking versions.

The layout uses a quiet green and warm neutral palette, a flat balance band,
an aligned movement ledger, and a focused form. It uses Candara for interface
text, Georgia for the date heading, and Consolas for amounts. These fonts are
available on the validation Windows host; other platforms may use GPUI font
fallbacks. Controls reuse the library's interaction and focus behavior.

## Workflow

1. Create a register by name. New registers use CAD.
2. Open a day. The date defaults to the computer's local date and remains
   editable in `YYYY-MM-DD` format. Enter an explicit opening count.
3. Choose Cash in or Cash out, enter a positive amount and reason, and save.
4. Read saved days in the sidebar. Movements and sessions use 25-row cursor
   pages with Older/Newer navigation. Movements are newest-first by entry ID;
   sessions are newest-first by session ID. Timestamps display in local time.
5. Close the day by entering the physical cash count. Review expected cash,
   counted cash, and the difference before selecting Confirm & close day.
   Closed sessions are read-only.

An active session is selected when reopening the app. Data changes from another
instance become visible through Refresh. Closing checks that the expected
balance still matches the reviewed snapshot; if it changed, refresh and review
again. The UI currently assumes CAD and two decimal places; multi-currency
register creation and denomination counting are not included.

Amounts are parsed as integers to the cent. Invalid decimals, negative values,
and overflow are rejected. A successful write clears the relevant draft. A
failed write preserves the form. Retriable cash-entry errors retain the exact
payload and retry key to avoid double counting. Constraint errors allow the
input to be corrected. A successful save followed by a failed refresh is
reported as saved, so it is not silently retried as a new entry.

## Keyboard and state handling

Tab/Shift+Tab navigate controls. Buttons support keyboard activation; inputs
support the library's selection, clipboard, and IME behavior. Ctrl+Enter submits
the current form; Escape cancels an auxiliary form. A closing count requires
two submissions, including the explicit review stage. Inputs have visible
labels and accessible names. Errors remain visible and use an alert role;
save feedback uses a status role. Busy operations disable mutation controls.
The form stacks above the ledger in narrower windows (minimum 860 × 640).
The app adds no custom animations.

## Persistence and background work

The app creates `register.sqlite3` in the user-local application-data directory
resolved by `directories::ProjectDirs` for `ca / Local / Cash Register`.
There is no sample data or automatic opening count. The development override
`CASH_REGISTER_DB` accepts an explicit database path, useful for isolated smoke
tests. It must point to a database owned by this application. The parent
directory is created if necessary.

The foreground view submits one operation at a time to a background-executor
task. A mutex serializes access to the retained SQLite connection. There is no
unbounded queue or persistent helper thread. Tasks are retained by the view;
database locking waits are bounded by the connection's five-second timeout.
If a window closes during a write, the database operation may still complete
atomically; its result cannot update a released view. Disk work never occurs
in render functions.

## Validation

```sh
cargo check --locked --all-targets
cargo test --locked
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
```

Use an alternate `--target-dir` if an existing running Windows executable locks
the normal build output. Database/service tests use temporary files. GPUI tests
exercise setup, cash saving, invalid input, duplicate-submit blocking, closing
review, and rendering at both layout widths using the test platform. Native
smoke testing should use an isolated `CASH_REGISTER_DB` rather than real records.

On Windows, the final native accessibility inspection exposed an upstream
`accesskit_windows 0.33.1` invalid-window-handle panic during shutdown after
Alt+F4. Saving, closing a day, and reopening persisted records succeeded before
that failure. Treat native shutdown with active accessibility clients as an
unresolved dependency limitation; do not claim full platform stability.
