param(
    [Parameter(Mandatory = $true)][string]$Binary,
    [Parameter(Mandatory = $true)][string]$Volume,
    [Parameter(Mandatory = $true)][string]$Output,
    [ValidateRange(1, 3)][int]$Pairs = 3,
    [switch]$Preflight
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($Volume -notmatch '^\\\\\?\\Volume\{[a-fA-F0-9-]{36}\}\\$') { throw 'Supply the verified volume GUID path.' }
$Output = [IO.Path]::GetFullPath($Output)
$root = [IO.Path]::GetPathRoot($Output)
$target = Get-Volume -UniqueId $Volume
$outputVolume = Get-Volume -DriveLetter $root.Substring(0, 1)
if ($target.UniqueId -eq $outputVolume.UniqueId) { throw 'Place outputs on a different volume.' }
if (Test-Path -LiteralPath $Output) { throw 'Output directory must not exist.' }
$source = (Get-Item -LiteralPath $Binary).FullName
New-Item -ItemType Directory -Path $Output | Out-Null
$program = Join-Path $Output 'index-benchmark.exe'
Copy-Item -LiteralPath $source -Destination $program
$scratch = Join-Path $Output 'temp'
New-Item -ItemType Directory -Path $scratch | Out-Null
@{ volume = $Volume; executableSha256 = (Get-FileHash -LiteralPath $program).Hash; started = (Get-Date).ToString('o'); os = [Environment]::OSVersion.VersionString; processors = [Environment]::ProcessorCount; preflight = [bool]$Preflight } |
    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Output 'manifest.json')

Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class IndexBenchmarkCounters {
    [StructLayout(LayoutKind.Sequential)]
    public struct Counters { public ulong Reads, Writes, Other, ReadBytes, WrittenBytes, OtherBytes; }
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool GetProcessIoCounters(IntPtr process, out Counters counters);
}
'@

function Invoke-IndexRun([string]$Name, [string]$Strategy) {
    $db = Join-Path $Output "$Name.sqlite"
    $stdout = Join-Path $Output "$Name.jsonl"
    $stderr = Join-Path $Output "$Name.err"
    $process = Start-Process -FilePath $program -ArgumentList @($Volume, ('"{0}"' -f $db), '600', $Strategy) `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr -WindowStyle Hidden -PassThru
    $handle = $process.Handle
    $peakPrivate = 0L
    $peakWorking = 0L
    $peakTemporary = 0L
    while (-not $process.WaitForExit(200)) {
        $process.Refresh()
        if (-not $process.HasExited) {
            $peakPrivate = [Math]::Max($peakPrivate, $process.PrivateMemorySize64)
            $peakWorking = [Math]::Max($peakWorking, $process.PeakWorkingSet64)
        }
        $bytes = 0L
        Get-ChildItem -LiteralPath $scratch -File -Force | ForEach-Object { $bytes += $_.Length }
        $wal = Get-Item -LiteralPath "$db-wal" -ErrorAction SilentlyContinue
        if ($wal) { $bytes += $wal.Length }
        $peakTemporary = [Math]::Max($peakTemporary, $bytes)
    }
    $counters = New-Object IndexBenchmarkCounters+Counters
    $hasCounters = [IndexBenchmarkCounters]::GetProcessIoCounters($handle, [ref]$counters)
    $dbSize = Get-Item -LiteralPath $db -ErrorAction SilentlyContinue
    @{ exit = $process.ExitCode; strategy = $Strategy; cpuSeconds = $process.TotalProcessorTime.TotalSeconds;
        peakWorkingSetBytes = [Math]::Max($peakWorking, $process.PeakWorkingSet64); sampledPeakPrivateBytes = $peakPrivate;
        sampledTempAndWalBytes = $peakTemporary; ioReadBytes = $(if ($hasCounters) { $counters.ReadBytes });
        ioWrittenBytes = $(if ($hasCounters) { $counters.WrittenBytes }); dbBytes = $(if ($dbSize) { $dbSize.Length }) } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Output "$Name.resources.json")
    if ($process.ExitCode -ne 0) { throw "$Name failed: $(Get-Content -LiteralPath $stderr -Raw)" }
}

$oldTemp = $env:TEMP
$oldTmp = $env:TMP
try {
    $env:TEMP = $scratch
    $env:TMP = $scratch
    if ($Preflight) { Invoke-IndexRun 'preflight' 'sequential' }
    else {
        for ($pair = 1; $pair -le $Pairs; $pair++) {
            $order = if ($pair % 2 -eq 0) { @('sequential', 'legacy') } else { @('legacy', 'sequential') }
            foreach ($strategy in $order) { Invoke-IndexRun "$pair-$strategy" $strategy }
        }
    }
    'ok' | Set-Content -LiteralPath (Join-Path $Output 'finished.txt')
} catch {
    $_.ToString() | Set-Content -LiteralPath (Join-Path $Output 'failed.txt')
    throw
} finally {
    $env:TEMP = $oldTemp
    $env:TMP = $oldTmp
}
