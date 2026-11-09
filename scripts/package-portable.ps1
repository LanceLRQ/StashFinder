param(
    [string]$OutputDirectory,
    [string]$BinaryDirectory,
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$projectRoot = Split-Path $PSScriptRoot -Parent
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $projectRoot ('.local\packages\' + [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')) }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $OutputDirectory) { throw 'OutputDirectory must be a new directory; existing packages are never overwritten.' }
if (-not $BinaryDirectory) { $BinaryDirectory = Join-Path $projectRoot 'target\release' }
$BinaryDirectory = [IO.Path]::GetFullPath($BinaryDirectory)
$version = (Get-Content -LiteralPath (Join-Path $projectRoot 'ui\src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).version
$protocolMatch = [regex]::Match((Get-Content -LiteralPath (Join-Path $projectRoot 'crates\core\src\lib.rs') -Raw), 'PROTOCOL_VERSION: u32 = (\d+)')
if (-not $protocolMatch.Success) { throw 'Cannot determine protocol version.' }
$protocol = [int]$protocolMatch.Groups[1].Value
Push-Location $projectRoot
try {
    if (-not $SkipBuild) {
        if ($BinaryDirectory -ne (Join-Path $projectRoot 'target\release')) { throw 'Custom BinaryDirectory requires -SkipBuild.' }
        & npm.cmd --prefix ui run tauri -- build --no-bundle -- --locked
        if ($LASTEXITCODE -ne 0) { throw 'Desktop release build failed.' }
        $originalRustFlags = [Environment]::GetEnvironmentVariable('RUSTFLAGS', 'Process')
        try {
            [Environment]::SetEnvironmentVariable('RUSTFLAGS', ($originalRustFlags + ' -C target-feature=+crt-static').Trim(), 'Process')
            & cargo build -p stashfinder-service --release --locked
            if ($LASTEXITCODE -ne 0) { throw 'Backend release build failed.' }
        } finally {
            [Environment]::SetEnvironmentVariable('RUSTFLAGS', $originalRustFlags, 'Process')
        }
    }
    $revision = (& git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Cannot determine source revision.' }
    $modified = [bool](@(& git status --porcelain).Count)
    $name = 'StashFinder-' + $version + '-windows-x64-portable'
    $folder = Join-Path $OutputDirectory $name
    $sources = [ordered]@{
        'StashFinder.exe' = (Join-Path $BinaryDirectory 'stashfinder-desktop.exe')
        'stashfinder-service.exe' = (Join-Path $BinaryDirectory 'stashfinder-service.exe')
        'README.md' = (Join-Path $projectRoot 'docs\portable.md')
        'README.en.md' = (Join-Path $projectRoot 'docs\portable.en.md')
        'LICENSE' = (Join-Path $projectRoot 'LICENSE')
        'third-party.md' = (Join-Path $projectRoot 'docs\third-party.md')
        'third-party.en.md' = (Join-Path $projectRoot 'docs\third-party.en.md')
    }
    foreach ($source in $sources.Values) {
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw ('Missing package input: ' + $source) }
    }
    New-Item -ItemType Directory -Path $folder | Out-Null
    foreach ($entry in $sources.GetEnumerator()) { Copy-Item -LiteralPath $entry.Value -Destination (Join-Path $folder $entry.Key) }
    foreach ($readme in @(@('README.md','(portable.en.md)','(README.en.md)'), @('README.en.md','(portable.md)','(README.md)'))) {
        $path = Join-Path $folder $readme[0]
        [IO.File]::WriteAllText($path, ([IO.File]::ReadAllText($path).Replace($readme[1], $readme[2])), [Text.UTF8Encoding]::new($false))
    }
    [IO.File]::WriteAllText((Join-Path $folder 'portable.json'), '{"version":1}', [Text.UTF8Encoding]::new($false))
    $files = @(Get-ChildItem -LiteralPath $folder -File | Sort-Object Name | ForEach-Object {
        [ordered]@{ name = $_.Name; bytes = $_.Length; sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
    })
    $manifest = [ordered]@{
        version = $version; architecture = 'windows-x64'; protocol = $protocol
        source_revision = $revision; source_modified = $modified; built_from_source = (-not $SkipBuild.IsPresent)
        packaged_at_utc = [DateTime]::UtcNow.ToString('o'); files = $files
    }
    [IO.File]::WriteAllText((Join-Path $folder 'manifest.json'), ($manifest | ConvertTo-Json -Depth 5), [Text.UTF8Encoding]::new($false))
    $archive = Join-Path $OutputDirectory ($name + '.zip')
    Compress-Archive -LiteralPath $folder -DestinationPath $archive -CompressionLevel Optimal
    $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText(($archive + '.sha256'), ($hash + '  ' + [IO.Path]::GetFileName($archive) + "`n"), [Text.UTF8Encoding]::new($false))
    & (Join-Path $PSScriptRoot 'verify-portable-package.ps1') -Archive $archive
    [pscustomobject]@{ Archive = $archive; Folder = $folder; SHA256 = $hash; SourceModified = $modified }
} finally { Pop-Location }
