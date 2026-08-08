# MNAB — Multi Needs A Budget

MNAB is an account-first, portable desktop budgeting application. Each installation owns one
Budget and opens its fixed local database automatically; there is no budget catalog or cloud
account to choose at startup.

## Product boundaries

- One local user; there is no authentication.
- No network access, telemetry, cloud synchronization, or bank API.
- Windows-only and USD-first.
- Money is represented as integer minor units (cents), never floating point.
- One Budget exists per installation in the fixed `mnab-data/mnab.sqlite3` database.
- Imported transactions always require review before entering the budget.
- Every application-owned file remains below `mnab-data` beside the executable.

## Portable storage

MNAB never redirects data to `%AppData%`, the working directory, or a temporary folder. On
first launch it creates this tree beside `MNAB.exe`:

```text
mnab-data/
  mnab.sqlite3
  backups/
  imports/
  logs/
  settings.json (created when settings are first saved)
```

The installation directory must therefore be writable by the user. Do not install under
`Program Files`; extract the application to a user-writable folder instead.

## Build on Windows

Install the stable Rust toolchain and the Microsoft C++ Build Tools, then use PowerShell:

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo build --release
```

The executable is `target\release\mnab.exe`. Copy it to a writable folder before running it.
Debug builds retain a console for diagnostics; release builds use the Windows GUI subsystem.

## Network-free and operating-system behavior

MNAB has no HTTP client, update checker, telemetry SDK, or remotely loaded asset. Native Windows
file dialogs may ask the operating system to enumerate shell locations, mounted/network drives,
or recent locations according to the user's Windows configuration. That behavior belongs to the
operating system; MNAB itself reads or writes only paths explicitly selected by the user and its
portable `mnab-data` tree.

## Account-first workflow

Accounts and their registers are the primary navigation. Categories continuously describe the
purpose of money rather than receiving monthly assignments. Category goals optionally connect a
category to an account, target amount, and target date. Goal progress, remaining amount, and the
account goal summary are calculated from transactions; the calculated uncategorized balance is
never stored as a separate allocation. When positive goal-category balances exceed their account
balance, MNAB shows the account as overcommitted.

Categories with transaction, split, schedule, rule, goal, import, or reconciliation history are
archived or merged, never physically deleted. Archived names remain available to historical
registers and reports.

## Transfers, cards, and imports

A normal paired transfer moves money between accounts and is excluded from income and spending.
If a transfer has an intentional category effect, that effect appears only in goal/activity views
that request it. Credit-card purchases are spending in the card account; payments are ordinary
transfers to that account and are not spending a second time.

Imports are staged for review. Merchant rules can normalize a merchant/payee and choose a
category, but imported transactions remain unapproved until accepted. Broken rules and import
failures appear in Inbox rather than silently changing financial data.

## Backup, restore, and legacy databases

Use MNAB's backup command before moving or repairing an installation. Restore validates the chosen
backup before replacing the fixed database and retains a safety copy of the current database.
MNAB's account-centric schema is a clean break: legacy multi-budget/monthly-assignment databases
are not registered as active migrations. Keep the old application to export them, then import the
transactions into a fresh MNAB installation.

## Manual release checklist

- [ ] Review the complete dependency tree (`cargo tree`) for networking, telemetry, remote assets,
      and newly introduced native/runtime requirements.
- [ ] Launch the ZIP on a clean Windows x64 machine without Rust, developer tools, or SQLite.
- [ ] Confirm the archive contains only `mnab.exe` and `README.txt`.
- [ ] Complete first-run setup, exit cleanly, and reopen the fixed Budget.
- [ ] Confirm legacy databases are rejected with clean-break guidance and are not modified.
- [ ] Import representative QFX, QBO, and CSV files and review deduplication results.
- [ ] Reconcile an account and verify its history.
- [ ] Create, validate, and restore a manual backup.
- [ ] Interrupt an import/write and verify that its transaction is atomic and the budget reopens.
- [ ] Exercise a clean shutdown during queued work and confirm the WAL is checkpointed.
- [ ] Confirm every MNAB-owned file remains below `mnab-data` in the extracted portable directory.
- [ ] Confirm removing or making the portable directory unwritable produces a clear warning.

## Architecture

`domain` and `calculation` are platform-independent. `service` coordinates typed commands
against repository traits, `storage` privately owns SQLite connections, and `ui` only emits
typed application commands. The source architecture test enforces the key dependency rules.
