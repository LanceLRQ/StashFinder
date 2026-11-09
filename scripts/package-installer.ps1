$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
Push-Location $projectRoot
try {
    $originalRustFlags = [Environment]::GetEnvironmentVariable('RUSTFLAGS', 'Process')
    try {
        [Environment]::SetEnvironmentVariable('RUSTFLAGS', ($originalRustFlags + ' -C target-feature=+crt-static').Trim(), 'Process')
        & cargo build -p stashfinder-service --release --locked
        if ($LASTEXITCODE -ne 0) { throw 'Backend release build failed.' }
    } finally {
        [Environment]::SetEnvironmentVariable('RUSTFLAGS', $originalRustFlags, 'Process')
    }
    & npm.cmd --prefix ui run tauri -- build --config src-tauri/tauri.installer.conf.json --bundles nsis -- --locked
    if ($LASTEXITCODE -ne 0) { throw 'Windows installer build failed.' }
} finally { Pop-Location }
