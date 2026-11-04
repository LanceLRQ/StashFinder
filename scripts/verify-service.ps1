param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^S-1-[0-9-]+$')]
    [string]$OwnerSid,
    [ValidateRange(0, 600)]
    [int]$HoldSeconds = 0,
    [string]$VhdDirectory
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run this verification script as administrator, with the ordinary GUI user SID.'
}
$projectRoot = Split-Path $PSScriptRoot -Parent
$source = Join-Path $projectRoot 'target\debug\stashfinder-service.exe'
$serviceName = 'StashFinderM0'
$testDirectory = Join-Path $env:ProgramFiles 'StashFinder-M0-Verification'
$binary = Join-Path $testDirectory 'stashfinder-service.exe'
$dataDirectory = Join-Path $env:ProgramData ('StashFinder-Verification-' + [Guid]::NewGuid().ToString('N'))
if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw 'Build the service first.' }
if (Get-Service -Name $serviceName -ErrorAction SilentlyContinue) { throw 'Verification service already exists; refusing to replace it.' }
if (Test-Path -LiteralPath $testDirectory) { throw 'Verification directory already exists; refusing to replace it.' }
$createdService = $false
$createdDirectory = $false
$result = [ordered]@{ started = $false; probe = $null; stopped = $false; removed = $false; dataDirectory = $dataDirectory }
try {
    New-Item -ItemType Directory -Path $testDirectory | Out-Null
    $createdDirectory = $true
    Copy-Item -LiteralPath $source -Destination $binary
    New-Item -ItemType Directory -Path $dataDirectory | Out-Null
    $acl = [Security.AccessControl.DirectorySecurity]::new()
    $acl.SetSecurityDescriptorSddlForm('O:BAG:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)')
    Set-Acl -LiteralPath $dataDirectory -AclObject $acl
    # Program Files inheritance prevents a normal user from replacing the elevated binary.
    New-Service -Name $serviceName -BinaryPathName ('"{0}" --service {1} --data-dir "{2}"' -f $binary, $OwnerSid, $dataDirectory) -StartupType Manual -DisplayName 'StashFinder verification' | Out-Null
    $createdService = $true
    Start-Service -Name $serviceName
    (Get-Service -Name $serviceName).WaitForStatus('Running', [TimeSpan]::FromSeconds(15))
    $result.started = $true
    $probe = & $binary --probe
    if ($LASTEXITCODE -ne 0) { throw 'SCM service probe failed' }
    $result.probe = $probe | ConvertFrom-Json
    if ($result.probe.mode -ne 'windows_service') { throw 'Probe did not reach the SCM service' }
    if ($VhdDirectory) {
        & (Join-Path $PSScriptRoot 'verify-assets.ps1') -Binary $binary -Directory $VhdDirectory -HoldSeconds $HoldSeconds
    } else {
        $deadline = [DateTime]::UtcNow.AddSeconds($HoldSeconds)
        while ([DateTime]::UtcNow -lt $deadline) { Start-Sleep -Seconds 1 }
    }
} finally {
    if ($createdService) {
        if ((Get-Service -Name $serviceName).Status -ne 'Stopped') {
            Stop-Service -Name $serviceName
            (Get-Service -Name $serviceName).WaitForStatus('Stopped', [TimeSpan]::FromSeconds(15))
        }
        $result.stopped = $true
        & sc.exe delete $serviceName | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'Failed to remove verification service' }
        $result.removed = $true
    }
    if ($createdDirectory) {
        # Only the exact file and empty directory created by this run are removed.
        if (Test-Path -LiteralPath $binary) { Remove-Item -LiteralPath $binary }
        Remove-Item -LiteralPath $testDirectory
    }
    $reportDirectory = Join-Path $projectRoot '.local'
    New-Item -ItemType Directory -Force -Path $reportDirectory | Out-Null
    $result | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $reportDirectory 'service-verification.json') -Encoding UTF8
}
$result | ConvertTo-Json -Depth 5
