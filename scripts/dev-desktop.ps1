$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$backend = $null
$exitCode = 1
Push-Location $projectRoot
try {
    $binary = Join-Path $projectRoot 'target\debug\stashfinder-service.exe'
    $existing = Get-ChildItem '\\.\pipe\' | Where-Object Name -eq 'StashFinder.M0'
    if (-not $existing -or -not (Test-Path -LiteralPath $binary)) {
        & cargo build -p stashfinder-service --locked
        if ($LASTEXITCODE -ne 0) { throw 'Development backend build failed.' }
    }
    if (-not $existing) {
        $data = Join-Path $projectRoot '.local\dev-assets'
        $start = [Diagnostics.ProcessStartInfo]::new($binary, ('--foreground --watch-stdin --data-dir "{0}"' -f $data))
        $start.UseShellExecute = $false
        $start.CreateNoWindow = $true
        $start.RedirectStandardInput = $true
        $backend = [Diagnostics.Process]::Start($start)
    }
    $reply = & $binary --probe
    if ($LASTEXITCODE -ne 0) { throw 'Development backend failed its readiness probe. Check the backend error above.' }
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
