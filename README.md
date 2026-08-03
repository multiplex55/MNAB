# MNAB — Multi Needs A Budget

MNAB is an early-stage, portable desktop budgeting application.

## Product boundaries

- One local user; there is no authentication.
- No network access, telemetry, cloud synchronization, or bank API.
- Windows-only and USD-first.
- Money is represented as integer minor units (cents), never floating point.
- Only one budget database is open at a time.
- Imported transactions always require review before entering the budget.
- Every application-owned file remains below `mnab-data` beside the executable.

## Portable storage

MNAB never redirects data to `%AppData%`, the working directory, or a temporary folder. On
first launch it creates this tree beside `MNAB.exe`:

```text
mnab-data/
  budgets/
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

The release archive is produced with `scripts/package-windows.ps1`. It contains exactly
`mnab.exe` and `README.txt`; the bundled SQLite feature means an installed SQLite runtime is not
required.

## Network-free and operating-system behavior

MNAB has no HTTP client, update checker, telemetry SDK, or remotely loaded asset. Native Windows
file dialogs may ask the operating system to enumerate shell locations, mounted/network drives,
or recent locations according to the user's Windows configuration. That behavior belongs to the
operating system; MNAB itself reads or writes only paths explicitly selected by the user and its
portable `mnab-data` tree.

## Manual release checklist

- [ ] Review the complete dependency tree (`cargo tree`) for networking, telemetry, remote assets,
      and newly introduced native/runtime requirements.
- [ ] Launch the ZIP on a clean Windows x64 machine without Rust, developer tools, or SQLite.
- [ ] Confirm the archive contains only `mnab.exe` and `README.txt`.
- [ ] Create a budget, exit cleanly, and reopen it.
- [ ] Upgrade a copy of every supported older schema and confirm the pre-migration backup.
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
