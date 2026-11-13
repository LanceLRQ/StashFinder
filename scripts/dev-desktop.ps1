param([switch]$CleanupOnly)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$binary = Join-Path $projectRoot 'target\debug\stashfinder-service.exe'
$data = Join-Path $projectRoot '.local\dev-assets'
$backendArguments = '--foreground --watch-stdin --data-dir "{0}"' -f $data
$backendCommand = '"{0}" {1}' -f $binary, $backendArguments
$backend = $null
$exitCode = 1
Push-Location $projectRoot
try {
    $processes = Get-CimInstance Win32_Process -Filter "Name='stashfinder-service.exe'"
    foreach ($candidate in $processes) {
        if ($candidate.ExecutablePath -ne $binary -or $candidate.CommandLine -cne $backendCommand) { continue }
        $parent = Get-CimInstance Win32_Process -Filter "ProcessId=$($candidate.ParentProcessId)"
        if ($parent -and $parent.CreationDate -le $candidate.CreationDate) { continue }
        $orphan = Get-Process -Id $candidate.ProcessId -ErrorAction SilentlyContinue
        if (-not $orphan) { continue }
        try {
            # Pin the process handle before checking its identity and stopping it.
            $null = $orphan.Handle
            if ($orphan.HasExited -or $orphan.Path -ne $binary -or
                [Math]::Abs(($orphan.StartTime - $candidate.CreationDate).TotalMilliseconds) -ge 1) { continue }
            $orphan.Kill()
            if (-not $orphan.WaitForExit(5000)) { throw "Orphaned development backend PID $($candidate.ProcessId) did not exit." }
            Write-Output "Cleaned up orphaned development backend (PID $($candidate.ProcessId))."
        } finally { $orphan.Dispose() }
    }
    if ($CleanupOnly) { exit 0 }

    $existing = Get-ChildItem '\\.\pipe\' | Where-Object Name -eq 'StashFinder.M0'
    if (-not $existing -or -not (Test-Path -LiteralPath $binary)) {
        & cargo build -p stashfinder-service --locked
        if ($LASTEXITCODE -ne 0) { throw 'Development backend build failed.' }
    }
    if (-not $existing) {
        $start = [Diagnostics.ProcessStartInfo]::new($binary, $backendArguments)
        $start.UseShellExecute = $false
        $start.CreateNoWindow = $true
        $start.RedirectStandardInput = $true
        $backend = [Diagnostics.Process]::Start($start)
    }
    if ($backend) { Write-Output 'Waiting for the development backend to open the index (up to 60 seconds)...' }
    $startup = [Diagnostics.Stopwatch]::StartNew()
    do {
        # Windows PowerShell can turn native stderr into terminating errors.
        $ErrorActionPreference = 'Continue'
        $reply = & $binary --probe 2>&1
        $probeExit = $LASTEXITCODE
        $ErrorActionPreference = 'Stop'
        if ($probeExit -eq 0) { break }
        if ($backend -and $backend.HasExited) {
            throw "Development backend exited before it was ready (exit code $($backend.ExitCode)). Check the backend output above."
        }
        if (-not $backend) {
            throw "The existing development pipe did not respond to a compatible probe. Close its launcher or check the service using StashFinder.M0. $($reply -join ' ')"
        }
        Start-Sleep -Milliseconds 250
    } while ($startup.Elapsed.TotalSeconds -lt 57)
    if ($probeExit -ne 0) {
        throw "Development backend failed its readiness probe after $([int]$startup.Elapsed.TotalSeconds) seconds. $($reply -join ' ')"
    }
    $status = $reply | ConvertFrom-Json
    if ($backend -and ($status.process_id -ne $backend.Id -or $status.mode -ne 'foreground')) {
        throw 'The readiness probe reached a different backend.'
    }
    Write-Output ('StashFinder backend ready ({0}, PID {1}).' -f $status.mode, $status.process_id)
    & pnpm --dir ui run tauri dev
    $exitCode = $LASTEXITCODE
} finally {
    if ($backend) {
        try {
            $backend.StandardInput.Close()
            if (-not $backend.WaitForExit(15000)) {
                $backend.Kill()
                $backend.WaitForExit()
            }
        } finally { $backend.Dispose() }
    }
    Pop-Location
}
exit $exitCode
