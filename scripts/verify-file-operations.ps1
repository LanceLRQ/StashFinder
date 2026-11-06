param(
    [Parameter(Mandatory=$true)][string]$Binary,
    [Parameter(Mandatory=$true)][string]$Directory,
    [ValidateSet('Prepare','Resume','Cleanup')][string]$Phase='Prepare',
    [ValidateRange(0,3600)][int]$HoldSeconds=0
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$Directory=[IO.Path]::GetFullPath($Directory).TrimEnd('\')
if(-not $Directory.StartsWith('D:\tmp\StashFinder-M6-',[StringComparison]::OrdinalIgnoreCase) -or $Directory.Contains('"')){throw 'Use a dedicated StashFinder-M6 directory under D:\tmp.'}
$image=Join-Path $Directory 'file-operations.vhdx'
$manifest=Join-Path $Directory 'verification.json'
function Save-Report { $script:report | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $manifest -Encoding UTF8 }
function Request($Action) {
    $pipe=[IO.Pipes.NamedPipeClientStream]::new('.','StashFinder.M0',[IO.Pipes.PipeDirection]::InOut,[IO.Pipes.PipeOptions]::None,[Security.Principal.TokenImpersonationLevel]::Identification)
    try {
        $pipe.Connect(3000)
        $bytes=[Text.Encoding]::UTF8.GetBytes((@{version=8;id=1;action=$Action}|ConvertTo-Json -Depth 10 -Compress))
        $writer=[IO.BinaryWriter]::new($pipe);$writer.Write([int]$bytes.Length);$writer.Write($bytes);$writer.Flush()
        $reader=[IO.BinaryReader]::new($pipe);$length=$reader.ReadInt32()
        if($length -lt 1 -or $length -gt 16384){throw 'Invalid reply length'}
        $body=$reader.ReadBytes($length);if($body.Length -ne $length){throw 'Truncated reply'}
        $reply=[Text.Encoding]::UTF8.GetString($body)|ConvertFrom-Json
        if($reply.body.kind -eq 'error'){throw $reply.body.data}
        if($reply.body.kind -ne 'accepted'){return $reply.body.data}
    } finally {$pipe.Dispose()}
}
function Wait-For([scriptblock]$Condition,[string]$Label){
    $deadline=[DateTime]::UtcNow.AddSeconds(120)
    do {if(&$Condition){return};Start-Sleep -Milliseconds 300}while([DateTime]::UtcNow -lt $deadline)
    throw "Timed out: $Label"
}
function Snapshot { $raw=&$Binary --snapshot 2>&1;if($LASTEXITCODE -ne 0){throw ($raw -join "`n")};return ($raw|ConvertFrom-Json) }
function Test-Disk {
    $disk=Get-DiskImage -ImagePath $image|Get-Disk
    if($disk.IsBoot -or $disk.IsSystem -or $disk.Size -ne 1GB){throw 'Unexpected disk; refusing operation'}
    return $disk
}
if($Phase -eq 'Cleanup'){
    $saved=Get-Content -LiteralPath $manifest -Raw|ConvertFrom-Json
    if($saved.image -ne $image){throw 'Manifest target mismatch'}
    if((Get-DiskImage -ImagePath $image).Attached){$null=Test-Disk;Dismount-DiskImage -ImagePath $image|Out-Null}
    $saved.cleaned=$true;$saved|ConvertTo-Json -Depth 12|Set-Content -LiteralPath $manifest -Encoding UTF8
    return
}
if($Phase -eq 'Prepare' -and (Test-Path -LiteralPath $Directory)){throw 'Prepare requires a new directory'}
if(-not (Test-Path -LiteralPath $Binary -PathType Leaf)){throw 'Build the service binary first'}
$probe=&$Binary --probe|ConvertFrom-Json
if($LASTEXITCODE -ne 0 -or $probe.mode -ne 'windows_service'){throw 'Run through verify-service.ps1 with an isolated LocalSystem backend'}
if($Phase -eq 'Prepare'){
    New-Item -ItemType Directory -Path $Directory|Out-Null
    $script:report=[ordered]@{image=$image;backendPid=$null;ready=$false;cleaned=$false;error=$null;fixtures=@()}
}else{
    $script:report=Get-Content -LiteralPath $manifest -Raw|ConvertFrom-Json
    if($report.image -ne $image -or $report.cleaned){throw 'Invalid resume manifest'}
    $report.error=$null
}
try {
    if($Phase -eq 'Prepare'){
    $create=Join-Path $Directory 'create.txt'
    @("create vdisk file=`"$image`" maximum=1024 type=expandable","select vdisk file=`"$image`"",'attach vdisk','exit')|Set-Content -LiteralPath $create -Encoding ASCII
    & diskpart.exe /s $create|Set-Content -LiteralPath (Join-Path $Directory 'diskpart.log')
    if(-not (Get-DiskImage -ImagePath $image).Attached){throw 'New VHD did not attach'}
    $disk=Test-Disk;if($disk.PartitionStyle -ne 'RAW'){throw 'Expected a new RAW VHD'}
    $disk|Initialize-Disk -PartitionStyle GPT|Out-Null
    foreach($fs in @('NTFS','exFAT','FAT32')){
        $disk=Test-Disk
        $partition=New-Partition -DiskNumber $disk.Number -Size 256MB -AssignDriveLetter
        $partition|Format-Volume -FileSystem $fs -NewFileSystemLabel ('SF-M6-'+$fs) -Confirm:$false|Out-Null
        $root=$partition.DriveLetter+':\'
        foreach($folder in @('F1\source\folder','F1\source\empty','F1\copy','F1\drag','F1\paste','F1\move','F1\conflict')){[IO.Directory]::CreateDirectory($root+$folder)|Out-Null}
        foreach($name in @('01-first.txt','02-second.txt',('03-'+[char]0x4e2d+[char]0x6587+'.txt'),'rename.txt','recycle.txt','permanent.txt','move.txt','locked.txt')){[IO.File]::WriteAllText($root+'F1\source\'+$name,'StashFinder M6 / '+$fs+' / '+$name)}
        [IO.File]::WriteAllText($root+'F1\source\folder\nested.txt','nested '+$fs)
        [IO.File]::WriteAllText($root+'F1\conflict\01-first.txt','keep conflict '+$fs)
        $deep=$root+'F1\source\long-path';while($deep.Length -lt 285){$deep+='\segment-0123456789'}
        [IO.Directory]::CreateDirectory('\\?\'+$deep)|Out-Null
        [IO.File]::WriteAllText('\\?\'+$deep+'\long.txt','long '+$fs)
        $report.fixtures+=@{filesystem=$fs;root=$root;partition=$partition.PartitionNumber;id=$null}
    }
    }else{
        $disk=Test-Disk
        foreach($fixture in $report.fixtures){
            $partition=Get-Partition -DiskNumber $disk.Number -PartitionNumber $fixture.partition
            $volume=$partition|Get-Volume
            if($volume.FileSystemLabel -ne ('SF-M6-'+$fixture.filesystem)){throw 'Unexpected resume partition'}
            $fixture.root=$partition.DriveLetter+':\'
        }
    }
    $report.backendPid=$null;Save-Report
    Wait-For {$s=Snapshot;-not $s.busy -and @($s.candidates|Where-Object {$_.observation.disk_number -eq $disk.Number -and -not $_.blocked}).Count -eq 3} 'three isolated volumes'
    if(@((Snapshot).assets).Count){throw 'The test catalog is not empty'}
    foreach($fixture in $report.fixtures){
        $s=Snapshot;$candidate=@($s.candidates|Where-Object {$_.observation.mounts -contains $fixture.root})
        if($candidate.Count -ne 1){throw 'Test volume not uniquely discovered'}
        Request @{register=@{version=$s.version;candidate_id=$candidate[0].observation.candidate_id;name=('M6 '+$fixture.filesystem);allow_weak=$false}}
        Wait-For {-not (Snapshot).busy} 'registration'
        $fixture.id=@((Snapshot).assets|Where-Object {$_.registration.name -eq ('M6 '+$fixture.filesystem)})[0].registration.partition_id
        if($fixture.filesystem -eq 'NTFS'){Request @{index_start=@{partition_id=$fixture.id;mode='scan_only'}}}
        Wait-For {(Request @{index_state=@{partition_id=$fixture.id}}).phase -eq 'snapshot'} ('index '+$fixture.filesystem)
    }
    $report.ready=$true;Save-Report
    Write-Output "Ready for ordinary-user GUI verification: $manifest"
    $deadline=[DateTime]::UtcNow.AddSeconds($HoldSeconds)
    while([DateTime]::UtcNow -lt $deadline -and -not (Test-Path -LiteralPath (Join-Path $Directory 'stop-gui'))){Start-Sleep -Seconds 1}
    if($HoldSeconds -gt 0){$null=Test-Disk;Dismount-DiskImage -ImagePath $image|Out-Null;$report.cleaned=$true;Save-Report}
} catch {$report.error=$_.Exception.Message;Save-Report;throw}
# The wrapper owns service cleanup; this script only detaches its exact VHD and retains all evidence.
