# Cash Ledger

[![CI](https://github.com/Zain-Repo/cash-counter/actions/workflows/ci.yml/badge.svg)](https://github.com/Zain-Repo/cash-counter/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Zain-Repo/cash-counter)](https://github.com/Zain-Repo/cash-counter/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)

Cash Ledger is a native desktop cash-register app built with Rust, GPUI and
SQLite. Open a register day, record cash
movements, and reconcile the physical count before closing.

<img src="assets/till.png" alt="Cash Ledger application icon" width="96" height="96">

## What it does

- Create named CAD registers and open a day with an explicit starting count.
- Record cash in and cash out with a reason and exact integer-cent arithmetic.
- Review balances and paginated movement/session history.
- Compare the physical count with the expected balance before closing a day.
- Persist records locally in SQLite, with transactional writes and duplicate
  submission protection.
- Set up a company profile and local administrator account, then sign in before
  accessing the register workspace.
- Configure company rules and printing, and generate date-range ledger reports.

## Install on Windows

Download `CashLedger-<version>-windows-x64-setup.exe` from
[Releases](https://github.com/Zain-Repo/cash-counter/releases/latest).
The installer supports Windows 10 version 1703 or later and Windows 11 x64,
requires no administrator privileges, and includes the required Visual C++
runtime. It creates a Start Menu shortcut and offers an optional desktop shortcut.

The release includes `SHA256SUMS.txt`. To check a download in PowerShell:

```powershell
Get-FileHash ./CashLedger-0.0.1-windows-x64-setup.exe -Algorithm SHA256
```

Compare the result with the corresponding line in `SHA256SUMS.txt`.
Installers are currently unsigned. Uninstall removes the application and its
shortcuts while preserving the register database.

## Use Cash Ledger

1. Complete company setup and create your administrator username and password.
   Subsequent launches start at the login page.
2. Create a register.
3. Open a day with the business date and opening cash count.
4. Add cash-in or cash-out movements with an amount and reason.
5. Review saved days and movement history; Refresh reloads changes from another instance.
6. Enter the physical closing count, review the difference, then confirm closure.

Enter signs in from either login field. Ctrl+Enter submits the current form;
Escape cancels an auxiliary form.
Closing requires the explicit review stage. Closed days are read-only.

## Data and limitations

Data stays on the local machine. On Windows, the default database is
`%LOCALAPPDATA%/Cash Register/data/register.sqlite3`; see the
[UI guide](docs/ui.md) for persistence and concurrency behavior.
Back up the database when the application is closed.
For development smoke tests, `CASH_REGISTER_DB` can point to an isolated database.

The initial version supports CAD with two decimal places. Multi-currency setup,
denomination counting and cloud sync are not implemented. Accounts are local to
the database; signing in does not encrypt the database or restrict access by
other software running under your Windows account.
An upstream Windows accessibility issue can cause a shutdown panic with active
accessibility clients; details are in the [UI guide](docs/ui.md).
Windows is the supported release target; Linux and macOS releases are not validated.

## Build and test

Install [Rust](https://www.rust-lang.org/tools/install), Visual Studio C++ Build
Tools and a Windows SDK. The repository pins the validated Rust 1.96.0 toolchain;
rustup installs it when running Cargo. Follow the [setup guide](docs/gpui/setup.md).

```powershell
cargo run --locked
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

The GPUI community packages are pinned as one compatible set. Preserve
`Cargo.lock` and review framework upgrades together.

To build the Windows installer, install Inno Setup 6.5 or later and run:

```powershell
./packaging/windows/build-installer.ps1
```

See [Windows packaging](docs/windows-packaging.md) for compiler discovery,
runtime redistribution, icon generation and verification.

## CI and releases

GitHub Actions checks formatting, all targets, Clippy and the complete test
suite, then builds the Windows installer for pushes and pull requests. On main,
the same CI run then reads the version from `Cargo.toml`, creates its `vX.Y.Z`
tag on the tested commit, and publishes the installer with license notices and
SHA-256 checksums. Pull requests never publish releases. Each new release requires
a version bump and matching release notes before pushing.
Actions are pinned to reviewed commit SHAs; Dependabot checks for updates weekly.
See [the release guide](docs/releases.md) for the versioning procedure.

## Documentation

- [Storage API and database invariants](docs/database.md)
- [Workspace behavior and known limitations](docs/ui.md)
- [Company setup, sign-in, settings, reports and printing](docs/company-and-reports.md)
- [Automatic and manual application updates](docs/updating.md)
- [GPUI development guide](docs/gpui/README.md)
- [Windows packaging](docs/windows-packaging.md)
- [Release process](docs/releases.md)

## License

Cash Ledger's source code and authored icon are available under the
[MIT License](LICENSE). Third-party libraries, assets and the Microsoft runtime
retain their respective licenses. Release packages include dependency notices;
see [third-party attribution](THIRD_PARTY_NOTICES.md).
