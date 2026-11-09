param(
    [Parameter(Mandatory)][ValidateSet('Prepare','Install','Uninstall')][string]$Phase,
    [Parameter(Mandatory)][string]$InstallDirectory,
    [ValidateSet('0','1')][string]$InstallService = '0'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$name = 'StashFinder'
$pipe = '\\.\pipe\StashFinder.Installed'
$admins = 'S-1-5-32-544'
$system = 'S-1-5-18'
$trustedInstaller = 'S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464'
$trusted = @($admins, $system, $trustedInstaller)
$sidType = [Security.Principal.SecurityIdentifier]
if (-not [Environment]::Is64BitProcess) { throw 'The x64 installer requires 64-bit Windows PowerShell.' }
$programFiles = [Environment]::GetFolderPath('ProgramFiles')
$directory = [IO.Path]::GetFullPath($InstallDirectory).TrimEnd('\')
if ($directory -ne (Join-Path $programFiles $name)) {
    throw "Install StashFinder in the default protected directory: $programFiles\$name"
}
$binary = Join-Path $directory 'stashfinder-service.exe'
$data = Join-Path ([Environment]::GetFolderPath('CommonApplicationData')) $name
$windows = [Environment]::GetFolderPath('Windows')
$sc = Join-Path ([Environment]::SystemDirectory) 'sc.exe'

function Assert-ProtectedPath([string]$Path) {
    $entry = Get-Item -LiteralPath $Path -Force
    $acl = Get-Acl -LiteralPath $Path
    if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -or $acl.GetOwner($sidType).Value -notin $trusted) {
        throw "Untrusted service path: $Path"
    }
    foreach ($rule in $acl.GetAccessRules($true, $true, $sidType)) {
        if ($rule.AccessControlType -eq 'Allow' -and ($rule.FileSystemRights -band 0xD0156) -and
            -not ($rule.PropagationFlags -band [Security.AccessControl.PropagationFlags]::InheritOnly) -and
            $rule.IdentityReference.Value -notin $trusted) {
            throw "An untrusted account can modify: $Path"
        }
    }
}

function Stop-InstalledService {
    $controller = Get-Service -Name $name
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(20)
        while ($controller.Status -eq 'StartPending') {
            if ([DateTime]::UtcNow -ge $deadline) { throw 'Service is still starting.' }
            Start-Sleep -Milliseconds 100
            $controller.Refresh()
        }
        if ($controller.Status -notin @('Stopped', 'StopPending')) { $controller.Stop() }
        $controller.WaitForStatus('Stopped', [TimeSpan]::FromSeconds(20))
    } finally { $controller.Dispose() }
}

function Remove-InstalledService {
    Stop-InstalledService
    & $sc delete $name
    if ($LASTEXITCODE -ne 0) { throw 'Could not remove the StashFinder service registration.' }
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        & $sc query $name | Out-Null
        if ($LASTEXITCODE -eq 1060) { return }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'Windows is still removing the StashFinder service. Close service-management windows and retry.'
}

$service = Get-CimInstance Win32_Service -Filter "Name='$name'"
$owner = $null
if ($service) {
    $expected = '^' + [regex]::Escape('"' + $binary + '" --installed-service ') + '(S-1-(?:\d+-)*\d+)$'
    if ($service.PathName -match $expected -and $service.StartName -eq 'LocalSystem') {
        $owner = $Matches[1]
    } else {
        throw 'A different Windows service already uses the StashFinder name.'
    }
}
Assert-ProtectedPath $programFiles
if (-not (Test-Path -LiteralPath $directory)) {
    if ($Phase -eq 'Uninstall' -and -not $service) { exit 0 }
    [IO.Directory]::CreateDirectory($directory) | Out-Null
}
Assert-ProtectedPath $directory
if (Test-Path -LiteralPath $binary) { Assert-ProtectedPath $binary }

if ($Phase -in @('Prepare', 'Uninstall')) {
    $desktopBackend = Get-CimInstance Win32_Process -Filter "Name='stashfinder-service.exe'" |
        Where-Object { $_.ExecutablePath -eq $binary -and $_.CommandLine -match '\s--desktop-session\s' }
    if ($desktopBackend) { throw 'Close StashFinder and wait for its indexing backend to exit before changing the installation.' }
}

if ($Phase -eq 'Prepare') {
    if ($service) { Stop-InstalledService }
    exit 0
}
if ($Phase -eq 'Uninstall') {
    if ($service) { Remove-InstalledService }
    exit 0
}

if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw 'Installed service executable is missing.' }
if (-not (Test-Path -LiteralPath $data)) {
    $acl = [Security.AccessControl.DirectorySecurity]::new()
    $acl.SetSecurityDescriptorSddlForm('O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)')
    [IO.Directory]::CreateDirectory($data, $acl) | Out-Null
}
Assert-ProtectedPath $data
$ownerFile = Join-Path $data 'owner.sid'
if (Test-Path -LiteralPath $ownerFile) {
    Assert-ProtectedPath $ownerFile
    $savedOwner = [IO.File]::ReadAllText($ownerFile).Trim()
    if ($owner -and $owner -ne $savedOwner) { throw 'Service account does not match the saved installation account.' }
    $owner = $savedOwner
}
if (-not $owner) {
    # The interactive shell remains under the original user even with alternate UAC credentials.
    $session = (Get-Process -Id $PID).SessionId
    $owners = @(Get-CimInstance Win32_Process -Filter "Name='explorer.exe' AND SessionId=$session" |
        Where-Object ExecutablePath -eq (Join-Path $windows 'explorer.exe') |
        ForEach-Object { (Invoke-CimMethod -InputObject $_ -MethodName GetOwnerSid).Sid } |
        Where-Object { $_ } | Sort-Object -Unique)
    if ($owners.Count -gt 1) { throw 'Cannot determine a unique interactive account for StashFinder.' }
    $owner = if ($owners.Count) { $owners[0] } else { [Security.Principal.WindowsIdentity]::GetCurrent().User.Value }
}
if ($owner -notmatch '^S-1-(?:\d+-)*\d+$') { throw 'Invalid installation account SID.' }
if (-not (Test-Path -LiteralPath $ownerFile)) {
    [IO.File]::WriteAllText($ownerFile, $owner)
    $ownerAcl = Get-Acl -LiteralPath $ownerFile
    $ownerAcl.SetOwner([Security.Principal.SecurityIdentifier]::new($admins))
    Set-Acl -LiteralPath $ownerFile -AclObject $ownerAcl
}

if ($InstallService -eq '0') {
    if ($service) { Remove-InstalledService }
    exit 0
}

$created = $false
try {
    if (-not $service) {
        New-Service -Name $name -DisplayName $name -StartupType Automatic `
            -BinaryPathName ('"{0}" --installed-service {1}' -f $binary, $owner) | Out-Null
        $created = $true
    }
    # The installation account can query and start this service. Only administrators can change it.
    & $sc sdset $name ('D:P(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;SY)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)(A;;LCRP;;;{0})' -f $owner)
    if ($LASTEXITCODE -ne 0) { throw 'Could not set the StashFinder service permissions.' }
    Set-Service -Name $name -StartupType Automatic
    Start-Service -Name $name
    $status = & $binary --probe $pipe
    if ($LASTEXITCODE -ne 0) { throw 'The installed backend did not respond to its readiness probe.' }
    $reported = $status | ConvertFrom-Json
    $actual = Get-CimInstance Win32_Service -Filter "Name='$name'"
    if ($reported.mode -ne 'windows_service' -or $reported.process_id -ne $actual.ProcessId) {
        throw 'The readiness probe did not reach the installed StashFinder service.'
    }
} catch {
    if ($created) {
        try {
            Remove-InstalledService
        } catch { Write-Warning $_ }
    }
    throw
}
