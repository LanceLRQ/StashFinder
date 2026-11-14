param(
    [Parameter(Mandatory=$true)][string]$Binary,
    [Parameter(Mandatory=$true)][string]$Directory,
    [ValidateRange(0,600)][int]$HoldSeconds=0
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$Directory=[IO.Path]::GetFullPath($Directory)
if(-not $Directory.StartsWith('D:\tmp\',[StringComparison]::OrdinalIgnoreCase) -or (Test-Path -LiteralPath $Directory)){throw 'Use a new isolated directory under D:\tmp.'}
New-Item -ItemType Directory -Path $Directory | Out-Null
$image=Join-Path $Directory 'scan-test.vhdx'
$reportPath=Join-Path (Split-Path $PSScriptRoot -Parent) '.local\scans-verification.json'
$report=[ordered]@{directory=$Directory;checks=@();error=$null;readyForGui=$false;detached=$false}
$attached=$false
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
function Assert-TestDisk {
    $disk=Get-DiskImage -ImagePath $image | Get-Disk
    if($disk.IsBoot -or $disk.IsSystem -or $disk.Size -ne 1GB){throw 'Unexpected test disk'}
    return $disk
}
function Save-Report { $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $reportPath -Encoding UTF8 }
function Check-Fixture($Root,$Id) {
    $actual=@([IO.Directory]::EnumerateFiles('\\?\'+$Root+'m4fixture','*',[IO.SearchOption]::AllDirectories) | ForEach-Object {[IO.FileInfo]::new($_)})
    $rows=@((Search 'm4fixture' $Id) | Where-Object {$_.file.kind -eq 'file'})
    if($rows.Count -ne $actual.Count){return $false}
    foreach($file in $actual){
        $relative=$file.FullName.Substring(4+$Root.Length)
        $found=@($rows | Where-Object {$_.file.path -eq $relative})
        if($found.Count -ne 1 -or $found[0].file.size -ne [string]$file.Length){return $false}
        if($found[0].file.created -ne [string]$file.CreationTimeUtc.ToFileTimeUtc() -or $found[0].file.modified -ne [string]$file.LastWriteTimeUtc.ToFileTimeUtc()){return $false}
    }
    return $true
}
try {
    $create=Join-Path $Directory 'create.txt'
    @("create vdisk file=`"$image`" maximum=1024 type=expandable","select vdisk file=`"$image`"",'attach vdisk','exit') | Set-Content -LiteralPath $create -Encoding ASCII
    & diskpart.exe /s $create | Set-Content -LiteralPath (Join-Path $Directory 'diskpart.log')
    $attached=(Get-DiskImage -ImagePath $image).Attached
    if(-not $attached){throw 'Test disk not attached'}
    $disk=Assert-TestDisk
    if($disk.PartitionStyle -ne 'RAW'){throw 'Test disk is not new'}
    $disk | Initialize-Disk -PartitionStyle GPT
    $fixtures=@()
    foreach($fs in @('NTFS','exFAT','FAT32')) {
        $partition=New-Partition -DiskNumber $disk.Number -Size 256MB -AssignDriveLetter
        $partition | Format-Volume -FileSystem $fs -NewFileSystemLabel ('SF-M4-'+$fs) -Confirm:$false | Out-Null
        $root=$partition.DriveLetter+':\'
        [IO.Directory]::CreateDirectory($root+'m4fixture\中文') | Out-Null
        [IO.File]::WriteAllText($root+'m4fixture\中文\zero.txt','')
        [IO.File]::WriteAllText($root+'m4fixture\before.txt','before')
        $deep=$root+'m4fixture\'+(('long-directory-' * 12))+'\'+('deep-' * 25)
        [IO.Directory]::CreateDirectory('\\?\'+$deep) | Out-Null
        [IO.File]::WriteAllText('\\?\'+$deep+'\long.txt','long')
        $fixtures+=@{filesystem=$fs;root=$root;partition=$partition.PartitionNumber;id=$null}
    }
    Wait-For { $s=Snapshot; -not $s.busy -and @($s.candidates | Where-Object {$_.observation.disk_number -eq $disk.Number -and -not $_.blocked}).Count -eq 3 } 'test partitions discovered'
    if(@((Snapshot).assets).Count){throw 'Test catalog must be empty'}
    foreach($fixture in $fixtures){
        $s=Snapshot;$c=@($s.candidates | Where-Object {$_.observation.mounts -contains $fixture.root})[0]
        Send-Action @{register=@{version=$s.version;candidate_id=$c.observation.candidate_id;name=('M4 '+$fixture.filesystem);allow_weak=$false}}
        Wait-For {-not (Snapshot).busy} 'registration'
        $fixture.id=@((Snapshot).assets | Where-Object {$_.registration.name -eq ('M4 '+$fixture.filesystem)})[0].registration.partition_id
        if($fixture.filesystem -eq 'NTFS'){
            $journal=& fsutil.exe usn queryjournal ($fixture.root.TrimEnd('\')) 2>&1
            if($LASTEXITCODE -eq 0){throw 'Fresh NTFS unexpectedly has a journal'}
            Send-Action @{index_start=@{partition_id=$fixture.id;mode='scan_only'}}
        }
        Wait-For { (Send-Action @{index_state=@{partition_id=$fixture.id}}).phase -eq 'snapshot' } ('initial '+$fixture.filesystem+' scan')
        Wait-For {Check-Fixture $fixture.root $fixture.id} ('initial independent '+$fixture.filesystem+' metadata comparison')
    }
    $report.checks+='NTFS without USN, exFAT and FAT32 scan Chinese, zero-sized and long-path files; results match independent file enumeration'
    Save-Report
    foreach($fixture in $fixtures){
        [IO.Directory]::Move($fixture.root+'m4fixture\中文',$fixture.root+'m4fixture\renamed')
        [IO.File]::WriteAllText($fixture.root+'m4fixture\renamed\zero.txt','changed')
        [IO.File]::Delete($fixture.root+'m4fixture\before.txt')
        [IO.File]::WriteAllText($fixture.root+'m4fixture\new.txt','new')
    }
    foreach($fixture in $fixtures){Wait-For {Check-Fixture $fixture.root $fixture.id} ('online '+$fixture.filesystem+' changes')}
    $report.checks+='Online directory rename, metadata changes, create and delete are reconciled without GUI requests'
    Save-Report
    Stop-Service StashFinderM0
    (Get-Service StashFinderM0).WaitForStatus('Stopped',[TimeSpan]::FromSeconds(20))
    foreach($fixture in $fixtures){[IO.File]::WriteAllText($fixture.root+'m4fixture\restart.txt','while stopped')}
    Start-Service StashFinderM0
    (Get-Service StashFinderM0).WaitForStatus('Running',[TimeSpan]::FromSeconds(20))
    Wait-For {$s=Snapshot; -not $s.busy -and @($s.assets).Count -eq 3} 'restart discovery'
    foreach($fixture in $fixtures){Wait-For {Check-Fixture $fixture.root $fixture.id} ('restart '+$fixture.filesystem+' full validation')}
    $report.checks+='Service restart keeps the NTFS scan choice and validates all three filesystems'
    Save-Report
    $before=Snapshot
    Dismount-DiskImage -ImagePath $image | Out-Null;$attached=$false
    Wait-For {@((Snapshot).assets | Where-Object {$_.connection -eq 'offline'}).Count -eq 3} 'offline detection'
    $offline=Snapshot
    foreach($fixture in $fixtures){
        if((Search 'restart.txt' $fixture.id).Count -ne 1){throw 'Offline search lost'}
        $old=@($before.assets | Where-Object {$_.registration.partition_id -eq $fixture.id})[0].registration.observation.capacity
        $current=@($offline.assets | Where-Object {$_.registration.partition_id -eq $fixture.id})[0].registration.observation.capacity
        if($old.total -ne $current.total -or $old.free -ne $current.free){throw 'Offline capacity changed'}
    }
    Mount-DiskImage -ImagePath $image | Out-Null;$attached=$true
    $disk=Assert-TestDisk
    foreach($fixture in $fixtures){
        $p=Get-Partition -DiskNumber $disk.Number -PartitionNumber $fixture.partition
        $fixture.root=$p.DriveLetter+':\'
        [IO.File]::WriteAllText($fixture.root+'m4fixture\reconnect.txt','reconnected')
    }
    Wait-For {@((Snapshot).assets | Where-Object {$_.connection -eq 'online'}).Count -eq 3} 'reconnect detection'
    foreach($fixture in $fixtures){Wait-For {Check-Fixture $fixture.root $fixture.id} ('reconnect '+$fixture.filesystem+' validation')}
    $ntfs=@($fixtures | Where-Object {$_.filesystem -eq 'NTFS'})[0]
    $journal=& fsutil.exe usn queryjournal ($ntfs.root.TrimEnd('\')) 2>&1
    if($LASTEXITCODE -eq 0){throw 'Scanning created a journal without consent'}
    $report.checks+='Offline records and capacity survive detach; reconnect validates changes; NTFS journal remains absent'
    $tasks=@();$offset=0;$version=$null
    do {
        $page=Send-Action @{task_page=@{version=$version;offset=$offset}}
        $version=$page.version
        if($page.row){$tasks+=$page.row}
        $offset=$page.next
    } while($null -ne $offset)
    $report.tasks=$tasks
    if(@($tasks | Where-Object {$_.error -like '*database is locked*'}).Count){throw 'Database lock conflict recorded during scan verification'}
    $report.fixtures=$fixtures;$report.readyForGui=$true;Save-Report
    $deadline=[DateTime]::UtcNow.AddSeconds($HoldSeconds)
    while([DateTime]::UtcNow -lt $deadline -and -not (Test-Path -LiteralPath (Join-Path $Directory 'stop-gui'))){
        if($attached -and (Test-Path -LiteralPath (Join-Path $Directory 'show-offline'))){
            Dismount-DiskImage -ImagePath $image | Out-Null;$attached=$false
            Wait-For {@((Snapshot).assets | Where-Object {$_.connection -eq 'offline'}).Count -eq 3} 'GUI offline detection'
            $report.guiOffline=$true;Save-Report
        }
        Start-Sleep -Seconds 1
    }
} catch {$report.error=$_.Exception.Message;throw}
finally {
    if($attached){Dismount-DiskImage -ImagePath $image | Out-Null}
    $report.detached=-not (Get-DiskImage -ImagePath $image).Attached
    Save-Report
}
