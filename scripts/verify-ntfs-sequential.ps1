param(
    [Parameter(Mandatory = $true)][string]$Binary,
    [Parameter(Mandatory = $true)][string]$Directory,
    [Parameter(Mandatory = $true)][string]$Output
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$Directory = [IO.Path]::GetFullPath($Directory)
$Output = [IO.Path]::GetFullPath($Output)
$testRoot = Get-Item -LiteralPath 'D:\tmp'
if ((Split-Path -Parent $Directory) -ine $testRoot.FullName -or ($testRoot.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
    throw 'Use a new direct child of D:\tmp.'
}
if (Test-Path -LiteralPath $Directory) { throw 'The test directory must be new.' }
if ((Test-Path -LiteralPath $Output) -or [IO.Path]::GetPathRoot($Output) -ine 'C:\') { throw 'Use a new output directory on C:.' }
$binaryPath = (Get-Item -LiteralPath $Binary).FullName
New-Item -ItemType Directory -Path $Directory | Out-Null
$image = Join-Path $Directory 'test.vhdx'
$attachScript = Join-Path $Directory 'attach.txt'
$attached = $false
try {
    @(('create vdisk file="{0}" maximum=1024 type=expandable' -f $image), ('select vdisk file="{0}"' -f $image), 'attach vdisk', 'exit') |
        Set-Content -LiteralPath $attachScript -Encoding ASCII
    & diskpart.exe /s $attachScript | Set-Content -LiteralPath (Join-Path $Directory 'diskpart.log')
    $mounted = Get-DiskImage -ImagePath $image
    $attached = $mounted.Attached
    if (-not $attached) { throw 'The new VHD is not attached.' }
    $disk = $mounted | Get-Disk
    if ($disk.IsBoot -or $disk.IsSystem -or $disk.PartitionStyle -ne 'RAW' -or $disk.Size -ne 1GB) { throw 'Refusing to initialize an unexpected disk.' }
    $disk | Initialize-Disk -PartitionStyle GPT
    $partition = Get-DiskImage -ImagePath $image | Get-Disk | New-Partition -UseMaximumSize -AssignDriveLetter
    $partition | Format-Volume -FileSystem NTFS -NewFileSystemLabel 'SF-Sequential-Test' -Confirm:$false | Out-Null
    $volume = $partition | Get-Volume
    $root = $volume.DriveLetter + ':\'
    if ($volume.DriveLetter -in @('C','D') -or $volume.FileSystemLabel -ne 'SF-Sequential-Test') { throw 'Unexpected fixture volume.' }
    $tree = Join-Path $root 'fixture'
    $folder = Join-Path $tree 'folder'
    New-Item -ItemType Directory -Path $folder | Out-Null
    $regular = Join-Path $folder 'source.txt'
    [IO.File]::WriteAllText($regular, 'known content')
    $unicode = ([string][char]0x4e2d) + [char]0x6587 + '.txt'
    [IO.File]::WriteAllText((Join-Path $tree $unicode), 'unicode')
    [IO.File]::WriteAllBytes((Join-Path $tree 'empty.txt'), [byte[]]@())
    $sparse = Join-Path $tree 'sparse.bin'
    [IO.File]::WriteAllBytes($sparse, [byte[]]@())
    & fsutil.exe sparse setflag $sparse | Out-Null
    if ($LASTEXITCODE) { throw 'Sparse flag setup failed.' }
    $stream = [IO.File]::OpenWrite($sparse)
    try { $stream.SetLength(8MB) } finally { $stream.Dispose() }
    $compressed = Join-Path $tree 'compressed.txt'
    [IO.File]::WriteAllText($compressed, ('A' * 65536))
    & compact.exe /C /I $compressed | Out-Null
    if ($LASTEXITCODE) { throw 'Compression setup failed.' }
    Set-Content -LiteralPath $regular -Stream 'test-metadata' -Value 'test stream'
    & fsutil.exe hardlink create (Join-Path $tree 'alias.txt') $regular | Out-Null
    if ($LASTEXITCODE) { throw 'Hard link creation failed.' }
    New-Item -ItemType Junction -Path (Join-Path $tree 'junction') -Target $folder | Out-Null
    $long = '\\?\' + (Join-Path $folder 'deep')
    while ($long.Length -lt 280) { $long += '\segment-0123456789' }
    [IO.Directory]::CreateDirectory($long) | Out-Null
    [IO.File]::WriteAllText(($long + '\long.txt'), 'long path')
    & fsutil.exe usn createjournal m=33554432 a=8388608 ($volume.DriveLetter + ':') | Out-Null
    if ($LASTEXITCODE) { throw 'Fixture journal creation failed.' }
    $before = (Get-FileHash -LiteralPath $regular).Hash
    & (Join-Path $PSScriptRoot 'benchmark-ntfs-index.ps1') -Binary $binaryPath -Volume $volume.UniqueId -Output $Output -Pairs 1
    & node.exe (Join-Path $PSScriptRoot 'compare-ntfs-index.mjs') (Join-Path $Output '1-legacy.sqlite') (Join-Path $Output '1-sequential.sqlite') (Join-Path $Output 'differences.jsonl') |
        Set-Content -LiteralPath (Join-Path $Output 'comparison.json')
    if ($LASTEXITCODE) { throw 'Fixture comparison failed.' }
    $comparison = Get-Content -LiteralPath (Join-Path $Output 'comparison.json') -Raw | ConvertFrom-Json
    if ($comparison.legacyOnly -ne 0 -or $comparison.sequentialOnly -ne 0) { throw 'Fixture path sets differ.' }
    if ((Get-FileHash -LiteralPath $regular).Hash -ne $before) { throw 'Fixture source changed.' }
    Dismount-DiskImage -ImagePath $image | Out-Null
    $attached = $false
    # The same persisted catalogs must remain readable without a mounted source.
    & node.exe (Join-Path $PSScriptRoot 'compare-ntfs-index.mjs') (Join-Path $Output '1-legacy.sqlite') (Join-Path $Output '1-sequential.sqlite') (Join-Path $Output 'offline-differences.jsonl') |
        Set-Content -LiteralPath (Join-Path $Output 'offline-comparison.json')
    if ($LASTEXITCODE) { throw 'Offline fixture comparison failed.' }
} finally {
    if ($attached) { Dismount-DiskImage -ImagePath $image | Out-Null }
}
