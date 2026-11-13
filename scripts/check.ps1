$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$projectRoot = Split-Path $PSScriptRoot -Parent
Push-Location $projectRoot
try {
    & cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw 'Rust formatting failed' }
    & pnpm --dir ui run format:check
    if ($LASTEXITCODE -ne 0) { throw 'Frontend formatting failed' }
    & pnpm --dir ui test
    if ($LASTEXITCODE -ne 0) { throw 'Frontend tests failed' }
    & pnpm --dir ui run build
    if ($LASTEXITCODE -ne 0) { throw 'Frontend type check/build failed' }
    & cargo clippy --workspace --all-targets --locked -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Rust static checks failed' }
    & cargo test --workspace --locked
    if ($LASTEXITCODE -ne 0) { throw 'Rust tests failed' }
    & cargo build --workspace --locked
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed' }
    Write-Output 'All automated checks passed.'
} finally {
    Pop-Location
}
