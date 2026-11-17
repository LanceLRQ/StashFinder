param(
    [string]$BenchmarkPath,
    [string]$CapturePath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run this benchmark from an administrator PowerShell.'
}
if (-not $BenchmarkPath) { $BenchmarkPath = Join-Path $repo 'target/release/examples/index-benchmark.exe' }
if (-not $CapturePath) { $CapturePath = Join-Path $repo 'target/release/examples/mft-compare.exe' }
$benchmark = (Resolve-Path -LiteralPath $BenchmarkPath).Path
$capture = (Resolve-Path -LiteralPath $CapturePath).Path
$id = (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0,6)
$root = Join-Path $repo '.local/deferred-mft-runs'
$result = Join-Path $root $id
$other = Join-Path $env:LOCALAPPDATA "StashFinder-deferred-mft-$id"
$volumes = @{}
$disks = @{}
foreach ($drive in 'C','D') {
    $volume = Get-Volume -DriveLetter $drive
    if ($volume.FileSystem -ne 'NTFS') { throw "$drive is not NTFS" }
    $volumes[$drive] = $volume.UniqueId
    $disks[$drive] = (Get-Partition -DriveLetter $drive).DiskNumber
}
if ($disks.C -eq $disks.D -or [IO.Path]::GetPathRoot($result) -ne 'D:\' -or [IO.Path]::GetPathRoot($other) -ne 'C:\') {
    throw 'Expected isolated output roots on two physical disks, C and D.'
}
New-Item -ItemType Directory -Force -Path $result,$other | Out-Null
Set-Content -LiteralPath (Join-Path $root 'current.txt') -Value $result
@{ volumes=$volumes; disks=$disks; rawC=(Join-Path $result 'raw-C'); rawD=(Join-Path $other 'raw-D');
   benchmark=$benchmark; capture=$capture; benchmarkSha=(Get-FileHash $benchmark).Hash; captureSha=(Get-FileHash $capture).Hash;
   cache='operating system cache uncontrolled'; rawReplay='identical captured bytes; capture overhead and loading excluded from replay timing' } |
    ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $result 'manifest.json')

function Write-Step([string]$Message) {
    $line = "$(Get-Date -Format o) $Message"
    Add-Content -LiteralPath (Join-Path $result 'progress.txt') -Value $line
    Write-Host $line
}

function Invoke-Measured([string]$Program, [string[]]$Arguments, [string]$Name) {
    $stdout = Join-Path $result "$Name.jsonl"
    $stderr = Join-Path $result "$Name.err"
    $process = Start-Process -FilePath $Program -ArgumentList $Arguments -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    $process.Handle | Out-Null
    $peak = 0L
    $deadline = [DateTime]::UtcNow.AddMinutes(10)
    try {
        while (-not $process.WaitForExit(100)) {
            if ([DateTime]::UtcNow -gt $deadline) { throw "$Name timed out" }
            $process.Refresh()
            $peak = [Math]::Max($peak, $process.PrivateMemorySize64)
        }
        if ($process.ExitCode -ne 0) { throw "$Name failed. See $stderr and $stdout" }
    } finally {
        if (-not $process.HasExited) { Stop-Process -Id $process.Id }
        $process.WaitForExit()
    }
    @{ name=$Name; privateMiB=[Math]::Round($peak/1MB,1) } | ConvertTo-Json -Compress |
        Add-Content -LiteralPath (Join-Path $result 'memory.jsonl')
    return @(Get-Content -LiteralPath $stdout -Encoding UTF8 | ConvertFrom-Json)
}

foreach ($drive in 'C','D') {
    & fsutil.exe usn queryjournal "${drive}:" *> (Join-Path $result "$drive-usn.txt")
    if ($LASTEXITCODE -ne 0) { throw "$drive requires an existing USN journal" }
    $output = if ($drive -eq 'C') { $result } else { $other }
    $raw = Join-Path $output "raw-$drive"
    Write-Step "$drive capture and identical-byte replay start"
    $events = Invoke-Measured $capture @('capture', $volumes[$drive], ('"{0}"' -f $raw)) "$drive-raw"
    $replay = @($events | Where-Object { $_.event -eq 'raw_replay' })
    if ($replay.Count -ne 6 -or @($replay | Where-Object { -not $_.equal }).Count -ne 0) { throw "$drive raw replay incomplete" }
    foreach ($mode in 'immediate','deferred') {
        $samples = @($replay | Where-Object { $_.mode -eq $mode } | Sort-Object elapsed_ms)
        $median = $samples[1]
        Write-Step "$drive raw replay $mode median $([Math]::Round($median.elapsed_ms,1)) ms, $($median.stats.objects) objects, $($median.stats.links) links, $($median.stats.extension_io.calls) extension reads, equal"
    }
    $modes = if ($drive -eq 'C') { @('direct','direct-deferred') } else { @('direct-deferred','direct') }
    foreach ($mode in $modes) {
        $database = Join-Path $output "$drive-$mode.sqlite"
        Write-Step "StashFinder Release $drive $mode start"
        $events = Invoke-Measured $benchmark @($volumes[$drive], ('"{0}"' -f $database), '360', $mode) "$drive-$mode"
        $ready = @($events | Where-Object { $_.event -eq 'searchable' })
        $saved = @($events | Where-Object { $_.event -eq 'persisted' })
        $build = @($events | Where-Object { $_.event -eq 'direct_build' })
        if ($ready.Count -ne 1 -or $saved.Count -ne 1 -or $build.Count -ne 1) { throw "$drive $mode incomplete" }
        if ($ready[0].sample_rows -le 0 -or -not $saved[0].search_during_backup) { throw "$drive $mode search unavailable" }
        Write-Step "StashFinder Release $drive $mode searchable $($ready[0].from_open_ms) ms, persisted $($saved[0].from_open_ms) ms"
        $io = $build[0].read_classes
        Write-Step "$drive $mode reads: sequential $([Math]::Round($io.sequential.ms,1)) ms; extensions $([Math]::Round($io.extensions.ms,1)) ms; attribute lists $([Math]::Round($io.attribute_lists.ms,1)) ms; mapping $([Math]::Round($io.mapping.ms,1)) ms; bootstrap $([Math]::Round($io.bootstrap.ms,1)) ms; reread $($build[0].reread_mft_bytes) bytes"
    }
}
@{ passed=$true; finishedUtc=[DateTime]::UtcNow.ToString('o') } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $result 'done.json')
Write-Step "Complete. Results: $result"
