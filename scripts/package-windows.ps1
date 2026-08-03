$ErrorActionPreference = "Stop"
$version = (Select-String '^version = "(.+)"' Cargo.toml).Matches.Groups[1].Value
$stage = Join-Path $env:TEMP "mnab-package-$([guid]::NewGuid())"
New-Item -ItemType Directory $stage | Out-Null
Copy-Item target/x86_64-pc-windows-msvc/release/mnab.exe (Join-Path $stage 'mnab.exe')
@"
MNAB v$version

Extract both files to a user-writable directory and run mnab.exe. Do not run from inside
the ZIP. MNAB creates mnab-data beside the executable on first launch. No installed SQLite
or other runtime is required.
"@ | Set-Content -Encoding utf8 (Join-Path $stage 'README.txt')
$archive = "MNAB-v$version-windows-x64.zip"
Remove-Item -ErrorAction Ignore $archive
Compress-Archive -Path (Join-Path $stage 'mnab.exe'),(Join-Path $stage 'README.txt') -DestinationPath $archive
Remove-Item -Recurse $stage
