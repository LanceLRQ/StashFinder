param(
    [Parameter(Mandatory=$true)][string]$Manifest,
    [string]$Binary=(Join-Path $PSScriptRoot '..\target\debug\stashfinder-service.exe')
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$Manifest=[IO.Path]::GetFullPath($Manifest)
$directory=Split-Path $Manifest -Parent
if(-not $directory.StartsWith('D:\tmp\StashFinder-M6-',[StringComparison]::OrdinalIgnoreCase) -or (Split-Path $Manifest -Leaf) -ne 'verification.json'){throw 'Use the authorized M6 verification manifest'}
$saved=Get-Content -LiteralPath $Manifest -Raw|ConvertFrom-Json
$image=Join-Path $directory 'file-operations.vhdx'
if($saved.image -ne $image -or $saved.cleaned -or -not $saved.ready -or @($saved.fixtures).Count -ne 3){throw 'The isolated M6 backend and VHD must already be ready'}
function Request($Action){
    $pipe=[IO.Pipes.NamedPipeClientStream]::new('.','StashFinder.M0',[IO.Pipes.PipeDirection]::InOut,[IO.Pipes.PipeOptions]::None,[Security.Principal.TokenImpersonationLevel]::Identification)
    try{
        $pipe.Connect(3000)
        $bytes=[Text.Encoding]::UTF8.GetBytes((@{version=11;id=1;action=$Action}|ConvertTo-Json -Depth 10 -Compress))
        $writer=[IO.BinaryWriter]::new($pipe);$writer.Write([int]$bytes.Length);$writer.Write($bytes);$writer.Flush()
        $reader=[IO.BinaryReader]::new($pipe);$length=$reader.ReadInt32()
        if($length -lt 1 -or $length -gt 16384){throw 'Invalid reply length'}
        $body=$reader.ReadBytes($length);if($body.Length -ne $length){throw 'Truncated reply'}
        $reply=[Text.Encoding]::UTF8.GetString($body)|ConvertFrom-Json
        if($reply.body.kind -eq 'error'){throw $reply.body.data}
        return $reply.body
    }finally{$pipe.Dispose()}
}
function Snapshot{
    $raw=&$Binary --snapshot
    if($LASTEXITCODE -ne 0){throw 'Snapshot request failed'}
    return ($raw|ConvertFrom-Json)
}
function Probe{
    $raw=&$Binary --probe
    if($LASTEXITCODE -ne 0){throw 'Service probe failed'}
    $probe=$raw|ConvertFrom-Json
    if($probe.mode -ne 'windows_service'){throw 'A dedicated LocalSystem verification backend is required'}
    return $probe
}
function Wait-Count([string]$Text,[int]$Count){
    $started=[DateTime]::UtcNow
    do{
        $start=Request @{search_start=@{text=$Text;names_only=$true;partitions=@($saved.fixtures|ForEach-Object {$_.id})}}
        $queryDeadline=[DateTime]::UtcNow.AddSeconds(5)
        do{
            $page=Request @{search_batch=@{id=$start.data;offset=0;limit=32}}
            if($page.data.done){break}
            Start-Sleep -Milliseconds 100
        }while([DateTime]::UtcNow -lt $queryDeadline)
        if($page.data.done -and -not $page.data.error -and $page.data.total -eq $Count){return ([DateTime]::UtcNow-$started).TotalMilliseconds}
        Start-Sleep -Milliseconds 200
    }while(([DateTime]::UtcNow-$started).TotalSeconds -lt 45)
    throw "Catalog did not reach $Count matches for $Text within 45 seconds"
}
$probe=Probe
$snapshot=Snapshot
$disk=Get-DiskImage -ImagePath $image|Get-Disk
if($disk.IsBoot -or $disk.IsSystem -or $disk.Size -ne 1GB){throw 'Unexpected test disk'}
$roots=@()
foreach($fixture in $saved.fixtures){
    $asset=@($snapshot.assets|Where-Object {$_.registration.partition_id -eq $fixture.id})
    $partition=Get-Partition -DiskNumber $disk.Number -PartitionNumber $fixture.partition
    if($asset.Count -ne 1 -or $asset[0].connection -ne 'online' -or $asset[0].registration.name -ne ('M6 '+$fixture.filesystem) -or $fixture.filesystem -notin @('NTFS','exFAT','FAT32') -or $fixture.root -ne ($partition.DriveLetter+':\') -or @($asset[0].mounts)[0] -ne $fixture.root){throw 'Fixture registration or mount mismatch'}
    $roots+=Join-Path $fixture.root 'F1'
}
$prefix='remount-'+[Guid]::NewGuid().ToString('N')
$report=[ordered]@{image=$image;servicePid=$probe.process_id;remountMs=$null;stages=@();passed=$false;error=$null}
$needsWritable=$false
try{
    foreach($root in $roots){[IO.File]::WriteAllText((Join-Path $root ($prefix+'-before.txt')),'Before remount')}
    $report.stages+=@{stage='initial';elapsedMs=(Wait-Count ($prefix+'-before.txt') 3)}
    $started=[DateTime]::UtcNow
    $needsWritable=$true
    Dismount-DiskImage -ImagePath $image|Out-Null
    Mount-DiskImage -ImagePath $image -Access ReadOnly|Out-Null
    Dismount-DiskImage -ImagePath $image|Out-Null
    Mount-DiskImage -ImagePath $image -Access ReadWrite|Out-Null
    $needsWritable=$false
    $report.remountMs=([DateTime]::UtcNow-$started).TotalMilliseconds
    foreach($root in $roots){[IO.File]::WriteAllText((Join-Path $root ($prefix+'-created.txt')),'After remount')}
    $report.stages+=@{stage='create';elapsedMs=(Wait-Count ($prefix+'-created.txt') 3)}
    foreach($root in $roots){Move-Item -LiteralPath (Join-Path $root ($prefix+'-created.txt')) -Destination (Join-Path $root ($prefix+'-renamed.txt'))}
    $report.stages+=@{stage='rename-new';elapsedMs=(Wait-Count ($prefix+'-renamed.txt') 3)}
    $report.stages+=@{stage='rename-old';elapsedMs=(Wait-Count ($prefix+'-created.txt') 0)}
    foreach($root in $roots){Remove-Item -LiteralPath (Join-Path $root ($prefix+'-renamed.txt'))}
    $report.stages+=@{stage='delete';elapsedMs=(Wait-Count ($prefix+'-renamed.txt') 0)}
    if((Probe).process_id -ne $probe.process_id){throw 'Backend restarted during verification'}
    $after=Snapshot
    foreach($fixture in $saved.fixtures){
        $old=@($snapshot.assets|Where-Object {$_.registration.partition_id -eq $fixture.id})[0]
        $now=@($after.assets|Where-Object {$_.registration.partition_id -eq $fixture.id})
        if($now.Count -ne 1 -or $now[0].registration.filesystem_id -ne $old.registration.filesystem_id){throw 'Registration changed across remount'}
    }
    $report.passed=$true
}catch{$report.error=$_.Exception.Message;throw}
finally{
    if($needsWritable){
        if((Get-DiskImage -ImagePath $image).Attached){Dismount-DiskImage -ImagePath $image|Out-Null}
        Mount-DiskImage -ImagePath $image -Access ReadWrite|Out-Null
    }
    $report|ConvertTo-Json -Depth 10|Set-Content -LiteralPath (Join-Path $directory 'remount-verification.json') -Encoding UTF8
}
$report|ConvertTo-Json -Depth 10
