param(
    [Parameter(Mandatory = $true)][string]$Binary,
    [Parameter(Mandatory = $true)][string]$Directory,
    [ValidateRange(0, 600)][int]$HoldSeconds = 0
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
# Only the VHD created in this new directory may be formatted or have its journal changed.
$Directory = [IO.Path]::GetFullPath($Directory)
if (-not $Directory.StartsWith('D:\tmp\', [StringComparison]::OrdinalIgnoreCase) -or (Test-Path -LiteralPath $Directory)) { throw 'Use a new isolated directory under D:\tmp.' }
$projectRoot = Split-Path $PSScriptRoot -Parent
$reportPath = Join-Path $projectRoot '.local\sync-verification.json'
$stopFile = Join-Path $Directory 'stop-gui'
$report = [ordered]@{ directory = $Directory; checks = @(); readyForGui = $false; error = $null; detached = $false }
New-Item -ItemType Directory -Path $Directory | Out-Null
$image = Join-Path $Directory 'sync-test.vhdx'
$attached = $false

function Send-Action($Action) {
    $pipe = [IO.Pipes.NamedPipeClientStream]::new('.', 'StashFinder.M0', [IO.Pipes.PipeDirection]::InOut, [IO.Pipes.PipeOptions]::None, [Security.Principal.TokenImpersonationLevel]::Identification)
    try {
        $pipe.Connect(3000)
        $bytes = [Text.Encoding]::UTF8.GetBytes((@{ version=12; id = 1; action = $Action } | ConvertTo-Json -Depth 8 -Compress))
        $writer = [IO.BinaryWriter]::new($pipe)
        $writer.Write([int]$bytes.Length); $writer.Write($bytes); $writer.Flush()
        $reader = [IO.BinaryReader]::new($pipe)
        $length = $reader.ReadInt32()
        if ($length -lt 1 -or $length -gt 16384) { throw 'Invalid reply size.' }
        $body = $reader.ReadBytes($length)
        if ($body.Length -ne $length) { throw 'Truncated reply.' }
        $reply = [Text.Encoding]::UTF8.GetString($body) | ConvertFrom-Json
        if ($reply.body.kind -eq 'error') { throw $reply.body.data }
        if ($reply.body.kind -ne 'accepted') { return $reply.body.data }
    } finally { $pipe.Dispose() }
}
function Snapshot {
    $json = & $Binary --snapshot 2>&1
    if ($LASTEXITCODE -ne 0) { throw ($json -join "`n") }
    return ($json | ConvertFrom-Json)
}
function Wait-For([scriptblock]$Condition, [string]$Description) {
    $deadline = [DateTime]::UtcNow.AddSeconds(120)
    do {
        if (& $Condition) { return }
        Start-Sleep -Milliseconds 500
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out: $Description"
}
function Search([string]$Text, [string]$Id, [bool]$IncludeHistory = $false) {
    $queryId = Send-Action @{ search_start = @{ text = $Text; names_only = $false; partitions = @($Id); options = @{ include_history = $IncludeHistory } } }
    $rows = @(); $offset = 0
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        $page = Send-Action @{ search_page = @{ id = $queryId; offset = $offset } }
        if ($page.error) { throw $page.error }
        if ($page.done) {
            if ($page.row) { $rows += $page.row }
            if ($null -eq $page.next) { return ,$rows }
            $offset = $page.next
        } else { Start-Sleep -Milliseconds 25 }
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'Search timed out.'
}
function Assert-TestDisk {
    $current = Get-DiskImage -ImagePath $image | Get-Disk
    if ($current.IsBoot -or $current.IsSystem -or $current.Size -ne 512MB) { throw 'Refusing to modify an unexpected disk.' }
    return $current
}
function Save-Report { $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $reportPath -Encoding UTF8 }
try {
    $createScript = Join-Path $Directory 'create.txt'
    @("create vdisk file=`"$image`" maximum=512 type=expandable", "select vdisk file=`"$image`"", 'attach vdisk', 'exit') | Set-Content -LiteralPath $createScript -Encoding ASCII
    & diskpart.exe /s $createScript | Set-Content -LiteralPath (Join-Path $Directory 'diskpart.log')
    $attached = (Get-DiskImage -ImagePath $image).Attached
    if (-not $attached) { throw 'New VHD not attached.' }
    $disk = Assert-TestDisk
    if ($disk.PartitionStyle -ne 'RAW') { throw 'New test disk is not RAW.' }
    $disk | Initialize-Disk -PartitionStyle GPT
    $partition = Get-DiskImage -ImagePath $image | Get-Disk | New-Partition -Size 384MB -AssignDriveLetter
    $partition | Format-Volume -FileSystem NTFS -NewFileSystemLabel 'SF-M3-Test' -Confirm:$false | Out-Null
    $root = $partition.DriveLetter + ':\'
    [IO.Directory]::CreateDirectory($root + 'fixture\aaa') | Out-Null
    [IO.File]::WriteAllText($root + 'fixture\aaa\bbb.txt', 'initial')
    [IO.File]::WriteAllText($root + 'fixture\remove.txt', 'remove')
    & fsutil.exe hardlink create ($root + 'fixture\alias.txt') ($root + 'fixture\aaa\bbb.txt') | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Hard link creation failed.' }
    & fsutil.exe usn createjournal m=33554432 a=8388608 ($root.TrimEnd('\'))
    if ($LASTEXITCODE -ne 0) { throw 'Test journal creation failed.' }
    Wait-For { $s = Snapshot; -not $s.busy -and @($s.candidates | Where-Object {$_.observation.disk_number -eq $disk.Number -and -not $_.blocked}).Count -eq 1 } 'automatic device discovery'
    $s = Snapshot
    if (@($s.assets).Count -ne 0) { throw 'Verification catalog must be empty.' }
    $candidate = @($s.candidates | Where-Object {$_.observation.disk_number -eq $disk.Number -and -not $_.blocked})[0]
    Send-Action @{register=@{version=$s.version;candidate_id=$candidate.observation.candidate_id;name='M3 Test';allow_weak=$false}}
    Wait-For { $s = Snapshot; -not $s.busy -and @($s.assets).Count -eq 1 } 'registration'
    $assetId = (Snapshot).assets[0].registration.partition_id
    $report.partitionId = $assetId
    Wait-For { (Send-Action @{index_state=@{partition_id=$assetId}}).phase -eq 'ready' } 'automatic initial index'
    $generation = (Send-Action @{index_state=@{partition_id=$assetId}}).active_generation
    $report.checks += 'Device discovery and initial indexing run without GUI requests to refresh or start indexing'
    [IO.Directory]::Move($root + 'fixture\aaa', $root + 'fixture\renamed')
    [IO.File]::WriteAllText($root + 'fixture\renamed\bbb.txt', 'updated automatically')
    [IO.File]::Delete($root + 'fixture\remove.txt')
    [IO.File]::WriteAllText($root + 'fixture\added.txt', 'added')
    Wait-For { $r = Search 'renamed bbb' $assetId; $r.Count -eq 1 -and $r[0].file.size -eq '21' -and (Search 'remove.txt' $assetId).Count -eq 0 -and (Search 'added.txt' $assetId).Count -eq 1 } 'incremental rename, metadata, create and delete'
    if ((Send-Action @{index_state=@{partition_id=$assetId}}).active_generation -ne $generation) { throw 'Ordinary changes triggered a full rebuild.' }
    if ((Search 'alias.txt' $assetId)[0].file.size -ne '21') { throw 'Hard link metadata is stale.' }
    $report.checks += 'USN incremental directory rename, hard links, sizes, create and delete preserve the active generation'
    Wait-For { $s=Send-Action @{index_state=@{partition_id=$assetId}}; $s.phase -eq 'ready' -and $null -eq $s.task_id } 'idle before policy change'
    Send-Action @{auto_update=@{enabled=$false}}
    [IO.File]::WriteAllText($root + 'fixture\manual.txt', 'manual')
    Start-Sleep -Seconds 4
    if ((Search 'manual.txt' $assetId).Count) { throw 'Disabled automatic updates still consumed changes.' }
    Send-Action @{index_start=@{partition_id=$assetId;mode='journal'}}
    Wait-For { (Search 'manual.txt' $assetId).Count -eq 1 } 'manual indexing while automatic updates are disabled'
    Send-Action @{auto_update=@{enabled=$true}}
    Send-Action @{partition_pause=@{partition_id=$assetId;paused=$true}}
    [IO.File]::WriteAllText($root + 'fixture\paused.txt', 'paused')
    Start-Sleep -Seconds 4
    if ((Search 'paused.txt' $assetId).Count) { throw 'Paused partition was updated.' }
    Send-Action @{partition_pause=@{partition_id=$assetId;paused=$false}}
    Wait-For { (Search 'paused.txt' $assetId).Count -eq 1 } 'resume from persisted cursor'
    $report.checks += 'Automatic policy, manual override and partition pause/resume affect actual indexing'
    Stop-Service -Name StashFinderM0
    (Get-Service StashFinderM0).WaitForStatus('Stopped',[TimeSpan]::FromSeconds(20))
    [IO.File]::WriteAllText($root + 'fixture\restart.txt', 'restart')
    Start-Service -Name StashFinderM0
    (Get-Service StashFinderM0).WaitForStatus('Running',[TimeSpan]::FromSeconds(20))
    Wait-For { $s=Snapshot; -not $s.busy -and @($s.assets).Count -eq 1 } 'service restart discovery'
    Wait-For { (Search 'restart.txt' $assetId).Count -eq 1 } 'service restart catch-up'
    $report.checks += 'SCM service restart replays changes from the saved cursor'
    Dismount-DiskImage -ImagePath $image | Out-Null; $attached = $false
    Wait-For { (Snapshot).assets[0].connection -eq 'offline' } 'automatic detach detection'
    if ((Search 'restart.txt' $assetId).Count -ne 1) { throw 'Offline snapshot lost.' }
    Mount-DiskImage -ImagePath $image | Out-Null; $attached = $true
    $disk = Assert-TestDisk
    $partition = Get-Partition -DiskNumber $disk.Number | Where-Object {$_.Size -eq 384MB}
    $newLetter = @('X','Y','Z','W' | Where-Object {-not (Get-Volume -DriveLetter $_ -ErrorAction SilentlyContinue)})[0]
    $partition | Set-Partition -NewDriveLetter $newLetter
    $root = $newLetter + ':\'
    [IO.File]::WriteAllText($root + 'fixture\remount.txt', 'remount')
    Wait-For { $s=Snapshot; $s.assets[0].connection -eq 'online' -and $s.assets[0].mounts -contains $root } 'new mount point'
    Wait-For { (Search 'remount.txt' $assetId).Count -eq 1 } 'reconnect catch-up'
    if ((Snapshot).assets[0].registration.partition_id -ne $assetId) { throw 'Drive letter change duplicated identity.' }
    $report.checks += 'Unmount/remount and drive letter change keep the registration and offline search'
    Send-Action @{partition_pause=@{partition_id=$assetId;paused=$true}}
    $generation = (Send-Action @{index_state=@{partition_id=$assetId}}).active_generation
    $null = Assert-TestDisk
    & fsutil.exe usn deletejournal /n ($root.TrimEnd('\'))
    if ($LASTEXITCODE -ne 0) { throw 'Test journal deletion failed.' }
    & fsutil.exe usn createjournal m=33554432 a=8388608 ($root.TrimEnd('\'))
    if ($LASTEXITCODE -ne 0) { throw 'Test journal recreation failed.' }
    [IO.File]::WriteAllText($root + 'fixture\gap.txt', 'gap')
    if ((Search 'restart.txt' $assetId).Count -ne 1) { throw 'Journal reset destroyed old search.' }
    Send-Action @{partition_pause=@{partition_id=$assetId;paused=$false}}
    Wait-For { $s=Send-Action @{index_state=@{partition_id=$assetId}}; $s.phase -eq 'needs_rebuild' } 'journal gap confirmation'
    if ((Send-Action @{index_state=@{partition_id=$assetId}}).active_generation -ne $generation) { throw 'Journal gap replaced the active index before confirmation.' }
    if ((Search 'restart.txt' $assetId).Count -ne 1) { throw 'Waiting for confirmation hid old search results.' }
    Send-Action @{index_start=@{partition_id=$assetId;mode='journal'}}
    Wait-For { $s=Send-Action @{index_state=@{partition_id=$assetId}}; $s.phase -eq 'ready' -and $s.active_generation -gt $generation } 'journal gap rebuild'
    if ((Search 'gap.txt' $assetId).Count -ne 1) { throw 'Rebuilt index omitted new file.' }
    $report.checks += 'Journal replacement waits for confirmation before rebuilding and keeps old search available'
    Send-Action @{partition_pause=@{partition_id=$assetId;paused=$true}}
    $disk = Assert-TestDisk
    $partition = Get-Partition -DiskNumber $disk.Number | Where-Object {$_.Size -eq 384MB}
    $partition | Format-Volume -FileSystem NTFS -NewFileSystemLabel 'SF-M3-New' -Confirm:$false -Force | Out-Null
    Wait-For { (Snapshot).assets[0].connection -eq 'historical' } 'filesystem history'
    if ((Search 'gap.txt' $assetId $true).Count -ne 1) { throw 'Historical filesystem records lost.' }
    $s = Snapshot
    if (@($s.assets).Count -ne 1) { throw 'Formatted filesystem was registered implicitly.' }
    $candidate = @($s.candidates | Where-Object {$_.observation.disk_number -eq $disk.Number -and -not $_.blocked})[0]
    Send-Action @{register=@{version=$s.version;candidate_id=$candidate.observation.candidate_id;name='M3 New';allow_weak=$false}}
    Wait-For { @((Snapshot).assets).Count -eq 2 } 'explicit new filesystem registration'
    $report.checks += 'Formatting preserves historical records and requires explicit registration for the new filesystem'
    $report.readyForGui = $true
    Save-Report
    $deadline = [DateTime]::UtcNow.AddSeconds($HoldSeconds)
    while ([DateTime]::UtcNow -lt $deadline -and -not (Test-Path -LiteralPath $stopFile)) { Start-Sleep -Seconds 1 }
} catch {
    $report.error = $_.Exception.Message
    throw
} finally {
    if ($attached) { Dismount-DiskImage -ImagePath $image | Out-Null }
    $report.detached = -not (Get-DiskImage -ImagePath $image).Attached
    Save-Report
}
