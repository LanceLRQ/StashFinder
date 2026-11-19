$protocolMatch = [regex]::Match((Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\crates\core\src\lib.rs') -Raw), 'PROTOCOL_VERSION: u32 = (\d+)')
if (-not $protocolMatch.Success) { throw 'Cannot determine protocol version.' }
[int]$protocolMatch.Groups[1].Value
