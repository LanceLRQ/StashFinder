param(
    [Parameter(Mandatory = $true)][string]$Binary,
    [Parameter(Mandatory = $true)][string]$Directory,
    [ValidateRange(0, 600)][int]$HoldSeconds = 0
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
# Run only after explicit authorization to create a journal and mutate test files.
# The newly created image is the only disk this script initializes or formats.
$Directory = [IO.Path]::GetFullPath($Directory)
if (-not $Directory.StartsWith('D:\tmp\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Use a new directory under D:\tmp.' }
if (Test-Path -LiteralPath $Directory) { throw 'Test directory must not exist.' }
$reportPath = Join-Path (Split-Path $PSScriptRoot -Parent) '.local\index-verification.json'
$report = [ordered]@{ directory = $Directory; checks = @(); readyForGui = $false; detached = $false; error = $null }
New-Item -ItemType Directory -Path $Directory | Out-Null
$image = Join-Path $Directory 'verification.vhdx'
$attached = $false

function Send-Action($Action) {
    $pipe = [IO.Pipes.NamedPipeClientStream]::new('.', 'StashFinder.M0', [IO.Pipes.PipeDirection]::InOut, [IO.Pipes.PipeOptions]::None, [Security.Principal.TokenImpersonationLevel]::Identification)
    try {
        $pipe.Connect(3000)
        $bytes = [Text.Encoding]::UTF8.GetBytes((@{ version=11; id = 1; action = $Action } | ConvertTo-Json -Depth 8 -Compress))
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
    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    do {
        $json = & $Binary --snapshot 2>&1
        if ($LASTEXITCODE -ne 0) { throw ('Snapshot failed: ' + ($json -join "`n")) }
        $value = $json | ConvertFrom-Json
        if (-not $value.busy) {
            if ($value.error) { throw $value.error }
            return $value
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'Snapshot timed out.'
}
function Wait-Index([string]$Id) {
    $deadline = [DateTime]::UtcNow.AddMinutes(5)
    do {
        $state = Send-Action @{ index_state = @{ partition_id = $Id } }
        if ($state.phase -notin @('loading', 'queued', 'enumerating', 'scanning', 'replaying', 'building')) { return $state }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'Index timed out.'
}
function Search([string]$Text, [string]$Id) {
    $queryId = Send-Action @{ search_start = @{ text = $Text; names_only = $false; partitions = @($Id) } }
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
function Read-Fixture([string]$Root) {
    $result = @{}
    $queue = [Collections.Generic.Queue[string]]::new()
    $queue.Enqueue($Root + 'fixture')
    while ($queue.Count) {
        $path = $queue.Dequeue()
        $attributes = [IO.File]::GetAttributes($path)
        $isDirectory = ($attributes -band [IO.FileAttributes]::Directory) -ne 0
        $relative = $path.Substring($Root.Length)
        $result[$relative] = @{ size = $(if ($isDirectory) { $null } else { ([IO.FileInfo]::new($path)).Length.ToString() }); modified = [IO.File]::GetLastWriteTimeUtc($path).ToFileTimeUtc().ToString() }
        if ($isDirectory -and ($attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0) {
            foreach ($child in [IO.Directory]::EnumerateFileSystemEntries($path)) { $queue.Enqueue($child) }
        }
    }
    return $result
}
function Compare-Fixture($Expected, $Actual) {
    if ($Expected.Count -ne $Actual.Count) { throw "Fixture count mismatch: expected $($Expected.Count), actual $($Actual.Count)." }
    foreach ($row in $Actual) {
        if (-not $Expected.ContainsKey($row.file.path)) { throw ('Unexpected path: ' + $row.file.path) }
        $item = $Expected[$row.file.path]
        if ($item.size -ne $row.file.size -or $item.modified -ne $row.file.modified) { throw ('Metadata mismatch: ' + $row.file.path) }
    }
}

try {
    Send-Action @{auto_update=@{enabled=$false}}
    $createScript = Join-Path $Directory 'create.txt'
    @("create vdisk file=`"$image`" maximum=512 type=expandable", "select vdisk file=`"$image`"", 'attach vdisk', 'exit') | Set-Content -LiteralPath $createScript -Encoding ASCII
    & diskpart.exe /s $createScript | Set-Content -LiteralPath (Join-Path $Directory 'diskpart.log')
    $diskImage = Get-DiskImage -ImagePath $image
    $attached = $diskImage.Attached
    if (-not $attached) { throw 'New VHD was not attached.' }
    $disk = $diskImage | Get-Disk
    if ($disk.IsBoot -or $disk.IsSystem -or $disk.PartitionStyle -ne 'RAW' -or $disk.Size -ne 512MB) { throw 'Unexpected test disk; refusing initialization.' }
    $disk | Initialize-Disk -PartitionStyle GPT
    $partition = Get-DiskImage -ImagePath $image | Get-Disk | New-Partition -Size 384MB -AssignDriveLetter
    $partition | Format-Volume -FileSystem NTFS -NewFileSystemLabel 'SF-M2-Test' -Confirm:$false | Out-Null
    $root = $partition.DriveLetter + ':\'
    $wideRoot = '\\?\' + $root
    [IO.Directory]::CreateDirectory($wideRoot + 'fixture\aaa') | Out-Null
    [IO.File]::WriteAllText($wideRoot + 'fixture\aaa\bbb.txt', 'initial')
    [IO.File]::WriteAllText($wideRoot + 'fixture\zero.txt', '')
    $unicodeName = ([string][char]0x4e2d) + [char]0x6587 + '.txt'
    [IO.File]::WriteAllText($wideRoot + 'fixture\' + $unicodeName, 'Chinese filename')
    $sourceHash = (Get-FileHash -LiteralPath ($root + 'fixture\' + $unicodeName)).Hash
    [IO.File]::WriteAllText($wideRoot + 'fixture\remove.txt', 'delete during scan')
    $long = $wideRoot + 'fixture\deep'
    while ($long.Length -lt 280) { $long += '\segment-0123456789' }
    [IO.Directory]::CreateDirectory($long) | Out-Null
    [IO.File]::WriteAllText($long + '\long.txt', 'long path')
    & fsutil.exe hardlink create ($root + 'fixture\alias.txt') ($root + 'fixture\aaa\bbb.txt') | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Hard link creation failed.' }
    New-Item -ItemType Junction -Path ($root + 'fixture\junction') -Target ($root + 'fixture\aaa') | Out-Null
    for ($group = 0; $group -lt 300; $group++) {
        $folder = $wideRoot + ('bulk\{0:d3}' -f $group)
        [IO.Directory]::CreateDirectory($folder) | Out-Null
        for ($n = 0; $n -lt 40; $n++) { [IO.File]::WriteAllText(($folder + ('\item-{0:d3}.txt' -f $n)), 'bulk') }
    }
    $expected = Read-Fixture $wideRoot
    Send-Action 'refresh'
    $snapshot = Snapshot
    if (@($snapshot.assets).Count -ne 0) { throw 'Verification catalog must be empty.' }
    $candidate = @($snapshot.candidates | Where-Object { $_.observation.disk_number -eq $disk.Number -and -not $_.blocked })
    if ($candidate.Count -ne 1) { throw 'Expected exactly one eligible VHD partition.' }
    Send-Action @{ register = @{ version = $snapshot.version; candidate_id = $candidate[0].observation.candidate_id; name = 'M2 Test'; allow_weak = $false } }
    $snapshot = Snapshot
    $assetId = $snapshot.assets[0].registration.partition_id
    $report.partitionId = $assetId
    $report.mount = $root
    Send-Action @{ index_start = @{ partition_id = $assetId; mode = 'journal' } }
    $state = Wait-Index $assetId
    if ($state.phase -ne 'failed' -or $state.active_generation) { throw 'Expected absent journal to require a choice.' }
    $report.checks += 'Missing journal does not silently create or publish an index'
    Send-Action @{ index_start = @{ partition_id = $assetId; mode = 'scan_only' } }
    $state = Wait-Index $assetId
    if ($state.phase -ne 'snapshot') { throw ($state | ConvertTo-Json -Compress) }
    $rows = Search 'fixture' $assetId
    Compare-Fixture $expected $rows
    if (@($rows | Where-Object { -not $_.needs_validation }).Count) { throw 'Scan-only results must be marked for validation.' }
    $report.checks += 'Scan-only metadata matches independent enumeration including Unicode, zero size, hard links, long paths and non-traversed junction'
    $watch = [Diagnostics.Stopwatch]::StartNew()
    Send-Action @{ index_start = @{ partition_id = $assetId; mode = 'create_journal' } }
    $state = Wait-Index $assetId
    $watch.Stop()
    if ($state.phase -ne 'ready') { throw ($state | ConvertTo-Json -Compress) }
    $report.buildMilliseconds = $watch.ElapsedMilliseconds
    $report.records = $state.records
    Compare-Fixture $expected (Search 'fixture' $assetId)
    $report.checks += 'Explicit journal creation, MFT enumeration and publication match source metadata'
    Send-Action @{ index_start = @{ partition_id = $assetId; mode = 'journal' } }
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        $state = Send-Action @{ index_state = @{ partition_id = $assetId } }
        if ($state.phase -in @('enumerating', 'scanning')) { break }
        if ([DateTime]::UtcNow -gt $deadline -or $state.phase -eq 'failed') { throw 'Could not observe active scan.' }
        Start-Sleep -Milliseconds 10
    } while ($true)
    [IO.Directory]::Move($wideRoot + 'fixture\aaa', $wideRoot + 'fixture\renamed')
    [IO.File]::WriteAllText($wideRoot + 'fixture\renamed\bbb.txt', 'updated during scan')
    [IO.File]::Delete($wideRoot + 'fixture\remove.txt')
    [IO.File]::WriteAllText($wideRoot + 'fixture\added.txt', 'created during scan')
    $expected = Read-Fixture $wideRoot
    $state = Wait-Index $assetId
    if ($state.phase -ne 'ready') { throw ($state | ConvertTo-Json -Compress) }
    Compare-Fixture $expected (Search 'fixture' $assetId)
    if ((Search 'aaa bbb' $assetId).Count) { throw 'Old path survived rename.' }
    if ((Search 'renamed bbb' $assetId).Count -ne 1) { throw 'Path AND search failed.' }
    $report.checks += 'Changes during scan replay directory rename, hard-link size, create and delete'
    $active = $state.active_generation
    Send-Action @{ index_start = @{ partition_id = $assetId; mode = 'journal' } }
    $running = Send-Action @{ index_state = @{ partition_id = $assetId } }
    Send-Action @{ task_cancel = @{ id = $running.task_id } }
    $state = Wait-Index $assetId
    if ($state.phase -notin @('failed','ready') -or $state.active_generation -ne $active) { throw 'Cancellation lost the old generation.' }
    Compare-Fixture $expected (Search 'fixture' $assetId)
    $report.checks += 'Cancellation preserves the published searchable generation'
    $watch.Restart()
    $null = Search 'definitely-no-match' $assetId
    $watch.Stop()
    $report.noMatchMilliseconds = $watch.ElapsedMilliseconds
    $serviceProcess = Get-CimInstance Win32_Service -Filter "Name='StashFinderM0'"
    $report.workingSetBytes = (Get-Process -Id $serviceProcess.ProcessId).WorkingSet64
    if ((Get-FileHash -LiteralPath ($root + 'fixture\' + $unicodeName)).Hash -ne $sourceHash) { throw 'Indexing changed ordinary file content.' }
    $report.checks += 'Unmodified sample content hash remains identical'
    Dismount-DiskImage -ImagePath $image | Out-Null
    $attached = $false
    $report.detachStates = @()
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        Send-Action 'refresh'
        $snapshot = Snapshot
        $report.detachStates += $snapshot.assets[0].connection
        if ($snapshot.assets[0].connection -eq 'offline') { break }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    $report.snapshot = $snapshot
    if ($snapshot.assets[0].connection -ne 'offline') { throw 'Offline status missing after device-removal settling period.' }
    $rows = Search 'fixture' $assetId
    Compare-Fixture $expected $rows
    if (@($rows | Where-Object { $_.connection -ne 'offline' -or $_.mounts.Count }).Count) { throw 'Offline rows advertised an online mount.' }
    Restart-Service -Name StashFinderM0
    (Get-Service StashFinderM0).WaitForStatus('Running', [TimeSpan]::FromSeconds(15))
    $snapshot = Snapshot
    Compare-Fixture $expected (Search 'fixture' $assetId)
    $report.checks += 'Detached VHD remains searchable with identical metadata after SCM restart'
    $report.snapshot = $snapshot
    $report.readyForGui = $true
    $report | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $reportPath -Encoding UTF8
    $deadline = [DateTime]::UtcNow.AddSeconds($HoldSeconds)
    while ([DateTime]::UtcNow -lt $deadline) { Start-Sleep -Seconds 1 }
} catch {
    $report.error = $_.ToString() + "`n" + $_.ScriptStackTrace
    $report | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $reportPath -Encoding UTF8
    throw
} finally {
    if ($attached) { Dismount-DiskImage -ImagePath $image | Out-Null }
    $remainingImage = Get-DiskImage -ImagePath $image -ErrorAction SilentlyContinue
    $report.detached = -not ($remainingImage -and $remainingImage.Attached)
    $report | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $reportPath -Encoding UTF8
}
