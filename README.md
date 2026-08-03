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

## Architecture

`domain` and `calculation` are platform-independent. `service` coordinates typed commands
against repository traits, `storage` privately owns SQLite connections, and `ui` only emits
typed application commands. The source architecture test enforces the key dependency rules.

