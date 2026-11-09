param([Parameter(Mandatory=$true)][string]$Archive)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::OpenRead([IO.Path]::GetFullPath($Archive))
try {
    $entries = @($zip.Entries | Where-Object { $_.Name })
    $manifestEntry = @($entries | Where-Object { $_.FullName.Replace('\','/').EndsWith('/manifest.json') })
    if ($manifestEntry.Count -ne 1) { throw 'Expected one package manifest.' }
    $prefix = $manifestEntry[0].FullName.Replace('\','/').Substring(0, $manifestEntry[0].FullName.Replace('\','/').Length - 'manifest.json'.Length)
    $reader = [IO.StreamReader]::new($manifestEntry[0].Open())
    try { $manifest = $reader.ReadToEnd() | ConvertFrom-Json } finally { $reader.Dispose() }
    $required = @('StashFinder.exe','stashfinder-service.exe','portable.json','README.md','README.en.md','LICENSE','third-party.md','third-party.en.md')
    if (@($manifest.files).Count -ne $required.Count -or @($manifest.files.name | Sort-Object -Unique).Count -ne $required.Count -or $entries.Count -ne ($required.Count + 1) -or $manifest.architecture -ne 'windows-x64') { throw 'Unexpected package content or architecture.' }
    foreach ($expected in $manifest.files) {
        if ($expected.name -notin $required) { throw 'Unexpected package file.' }
        $entry = @($entries | Where-Object { $_.FullName.Replace('\','/') -ceq ($prefix + $expected.name) })
        if ($entry.Count -ne 1 -or $entry[0].Length -ne $expected.bytes) { throw ('Missing, duplicate or truncated file: ' + $expected.name) }
        $stream = $entry[0].Open(); $algorithm = [Security.Cryptography.SHA256]::Create()
        try { $hash = ([BitConverter]::ToString($algorithm.ComputeHash($stream))).Replace('-','').ToLowerInvariant() }
        finally { $algorithm.Dispose(); $stream.Dispose() }
        if ($hash -cne $expected.sha256) { throw ('SHA256 mismatch: ' + $expected.name) }
    }
    $markerEntry = @($entries | Where-Object { $_.Name -ceq 'portable.json' })[0]
    $reader = [IO.StreamReader]::new($markerEntry.Open())
    try { $marker = $reader.ReadToEnd() | ConvertFrom-Json } finally { $reader.Dispose() }
    if ($marker.version -ne 1) { throw 'Invalid portable marker.' }
    Write-Output 'Portable archive: content and SHA256 checks passed.'
} finally { $zip.Dispose() }
