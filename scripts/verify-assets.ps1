param(
    [Parameter(Mandatory = $true)][string]$Binary,
    [Parameter(Mandatory = $true)][string]$Directory,
    [ValidateRange(0,600)][int]$HoldSeconds = 0
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
# This script only creates and formats a new file-backed test disk. It never
# selects an existing disk by number or accepts an existing VHD directory.
$Directory = [IO.Path]::GetFullPath($Directory)
if (-not $Directory.StartsWith('D:\tmp\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Use a new directory under D:\tmp.' }
if (Test-Path -LiteralPath $Directory) { throw 'Test directory must not exist.' }
$reportDirectory = Join-Path (Split-Path $PSScriptRoot -Parent) '.local'
New-Item -ItemType Directory -Force -Path $reportDirectory | Out-Null
$reportPath = Join-Path $reportDirectory 'assets-verification.json'
$report = [ordered]@{ directory = $Directory; checks = @(); readyForGui = $false; detached = $false; error = $null }
New-Item -ItemType Directory -Path $Directory | Out-Null
$image = Join-Path $Directory 'verification.vhdx'
$attached = $false

function Invoke-AssetAction($Action) {
    $pipe = [IO.Pipes.NamedPipeClientStream]::new('.', 'StashFinder.M0', [IO.Pipes.PipeDirection]::InOut, [IO.Pipes.PipeOptions]::None, [Security.Principal.TokenImpersonationLevel]::Identification)
    try {
        $pipe.Connect(3000)
        $bytes = [Text.Encoding]::UTF8.GetBytes((@{version=4;id=1;action=$Action} | ConvertTo-Json -Depth 8 -Compress))
        $writer = [IO.BinaryWriter]::new($pipe)
        $writer.Write([int]$bytes.Length)
        $writer.Write($bytes)
        $writer.Flush()
        $reader = [IO.BinaryReader]::new($pipe)
        $length = $reader.ReadInt32()
        if ($length -lt 1 -or $length -gt 16384) { throw 'Invalid reply size.' }
        $body = $reader.ReadBytes($length)
        if ($body.Length -ne $length) { throw 'Truncated reply.' }
        $reply = [Text.Encoding]::UTF8.GetString($body) | ConvertFrom-Json
        if ($reply.body.kind -ne 'accepted') { throw ($reply | ConvertTo-Json -Depth 8 -Compress) }
    } finally { $pipe.Dispose() }
    Get-AssetSnapshot
}

function Get-AssetSnapshot {
    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    do {
        $json = & $Binary --snapshot 2>&1
        if ($LASTEXITCODE -ne 0) { throw ('Asset snapshot failed: ' + ($json -join "`n")) }
        $snapshot = $json | ConvertFrom-Json
        if (-not $snapshot.busy) {
            if ($snapshot.error) { throw $snapshot.error }
            return $snapshot
        }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'Device discovery timed out.'
}

try {
    $createScript = Join-Path $Directory 'create.txt'
    @("create vdisk file=`"$image`" maximum=512 type=expandable", "select vdisk file=`"$image`"", 'attach vdisk', 'exit') | Set-Content -LiteralPath $createScript -Encoding ASCII
    & diskpart.exe /s $createScript | Set-Content -LiteralPath (Join-Path $Directory 'diskpart.log')
    $diskImage = Get-DiskImage -ImagePath $image
    $attached = $diskImage.Attached
    if (-not $attached) { throw 'New VHD was not attached.' }
    $disk = $diskImage | Get-Disk
    if ($disk.IsBoot -or $disk.IsSystem -or $disk.PartitionStyle -ne 'RAW' -or $disk.Size -ne 512MB) { throw 'Unexpected test disk; refusing initialization.' }
    $disk | Initialize-Disk -PartitionStyle GPT
    $testPartitions = @()
    foreach ($fs in @('NTFS','exFAT','FAT32')) {
        $partition = Get-DiskImage -ImagePath $image | Get-Disk | New-Partition -Size 128MB -AssignDriveLetter
        $partition | Format-Volume -FileSystem $fs -NewFileSystemLabel ('SF-Test-' + $fs) -Confirm:$false | Out-Null
        $testPartitions += $partition
    }
    $snapshot = Invoke-AssetAction 'refresh'
    $report.snapshot = $snapshot
    if (@($snapshot.assets).Count -ne 0) { throw 'Verification catalog is not empty.' }
    $candidates = @($snapshot.candidates | Where-Object { $_.observation.disk_number -eq $disk.Number -and -not $_.blocked })
    if ($candidates.Count -ne 3) { throw 'Expected three supported VHD partitions.' }
    $report.checks += 'NTFS/exFAT/FAT32 discovered without registration'
    $first = $candidates | Where-Object { $_.observation.identity.filesystem -eq 'NTFS' }
    $snapshot = Invoke-AssetAction @{register=@{version=$snapshot.version;candidate_id=$first.observation.candidate_id;name='M1 NTFS';allow_weak=$false}}
    if (@($snapshot.assets).Count -ne 1) { throw 'Selection registered unexpected partitions.' }
    $original = $snapshot.assets[0].registration
    $report.checks += 'Only selected partition registered'
    $snapshot = Invoke-AssetAction @{rename=@{partition_id=$original.partition_id;name='M1 重命名'}}
    if ($snapshot.assets[0].registration.name -ne 'M1 重命名') { throw 'Rename failed.' }
    $available = @([char[]]'ZYXWVUTSRQPONMLKJIHG' | Where-Object { -not (Get-Volume -DriveLetter $_ -ErrorAction SilentlyContinue) })
    if (-not $available.Count) { throw 'No spare drive letter.' }
    $ntfsPartition = $testPartitions[0]
    $ntfsPartition | Set-Partition -NewDriveLetter $available[0]
    $snapshot = Invoke-AssetAction 'refresh'
    if ($snapshot.assets[0].registration.partition_id -ne $original.partition_id -or $snapshot.assets[0].mounts[0] -ne ($available[0] + ':\')) { throw 'Drive-letter reassociation failed.' }
    $report.checks += 'Drive-letter change preserves partition and filesystem IDs'
    $capacity = $snapshot.assets[0].registration.observation.capacity
    Dismount-DiskImage -ImagePath $image | Out-Null
    $attached = $false
    $snapshot = Invoke-AssetAction 'refresh'
    if ($snapshot.assets[0].connection -ne 'offline' -or $snapshot.assets[0].registration.observation.capacity.sampled_at -ne $capacity.sampled_at) { throw 'Offline snapshot failed.' }
    $report.checks += 'Detach preserves historical capacity'
    Mount-DiskImage -ImagePath $image | Out-Null
    $attached = $true
    $snapshot = Invoke-AssetAction 'refresh'
    if ($snapshot.assets[0].connection -ne 'online' -or $snapshot.assets[0].registration.filesystem_id -ne $original.filesystem_id) { throw 'Remount identity failed.' }
    $report.checks += 'Remount restores original registration'
    foreach ($fs in @('exFAT','FAT32')) {
        $candidate = $snapshot.candidates | Where-Object { $_.observation.disk_number -eq (Get-DiskImage -ImagePath $image | Get-Disk).Number -and $_.observation.identity.filesystem -eq $fs }
        $snapshot = Invoke-AssetAction @{register=@{version=$snapshot.version;candidate_id=$candidate.observation.candidate_id;name=('M1 '+$fs);allow_weak=$false}}
    }
    $ids = @($snapshot.assets | ForEach-Object { $_.registration.filesystem_id } | Sort-Object)
    Restart-Service -Name StashFinderM0
    (Get-Service StashFinderM0).WaitForStatus('Running', [TimeSpan]::FromSeconds(15))
    $snapshot = Get-AssetSnapshot
    $restored = @($snapshot.assets | ForEach-Object { $_.registration.filesystem_id } | Sort-Object)
    if (($ids -join ',') -ne ($restored -join ',') -or @($snapshot.assets | Where-Object { $_.registration.name -eq 'M1 重命名' }).Count -ne 1) { throw 'Service restart lost registration.' }
    if (@($snapshot.assets | Where-Object { $_.error }).Count) { throw 'Partition health failed.' }
    $report.checks += 'Three filesystem databases and name survive SCM restart'
    # Leave one additional disposable partition unregistered for the real GUI check.
    $guiPartition = Get-DiskImage -ImagePath $image | Get-Disk | New-Partition -Size 64MB -AssignDriveLetter
    $guiPartition | Format-Volume -FileSystem NTFS -NewFileSystemLabel 'SF-GUI' -Confirm:$false | Out-Null
    $report.guiMount = $guiPartition.DriveLetter + ':\'
    $snapshot = Invoke-AssetAction 'refresh'
    $report.snapshot = $snapshot
    $report.readyForGui = $true
    $report | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $reportPath -Encoding UTF8
    $deadline = [DateTime]::UtcNow.AddSeconds($HoldSeconds)
    while ([DateTime]::UtcNow -lt $deadline) { Start-Sleep -Seconds 1 }
} catch {
    $report.error = $_.ToString()
    $report | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $reportPath -Encoding UTF8
    # Keep the isolated test disk available for inspection during the requested hold.
    $deadline = [DateTime]::UtcNow.AddSeconds($HoldSeconds)
    while ([DateTime]::UtcNow -lt $deadline) { Start-Sleep -Seconds 1 }
    throw
} finally {
    if ($attached) { Dismount-DiskImage -ImagePath $image | Out-Null; $report.detached = $true }
    $report | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $reportPath -Encoding UTF8
}
