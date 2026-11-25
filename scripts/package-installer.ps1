$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
Push-Location $projectRoot
try {
    & pnpm --dir ui run tauri build --config src-tauri/tauri.installer.conf.json --bundles nsis -- -- --locked
    if ($LASTEXITCODE -ne 0) { throw 'Windows installer build failed.' }
} finally { Pop-Location }
