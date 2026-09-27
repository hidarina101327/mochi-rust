param([Parameter(Mandatory=$true)][string]$Directory)
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$needles = @([Environment]::UserName, [Environment]::GetFolderPath('UserProfile'), $repoRoot, $repoRoot.Replace('\','/')) | Where-Object { $_ }
$files = @(Get-ChildItem -LiteralPath $Directory -File -Recurse)
foreach ($file in $files) {
    if ($file.Extension -in @('.pdb','.rs','.rlib','.rmeta')) { throw "Development artifact in package: $($file.Name)" }
    $bytes = [IO.File]::ReadAllBytes($file.FullName)
    $ascii = [Text.Encoding]::ASCII.GetString($bytes)
    $unicode = [Text.Encoding]::Unicode.GetString($bytes)
    foreach ($needle in $needles) {
        if ($ascii.IndexOf($needle, [StringComparison]::OrdinalIgnoreCase) -ge 0 -or $unicode.IndexOf($needle, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
            throw "Personal build string found in $($file.Name)"
        }
    }
    if ($file.Name -notin @('Mochi.exe','mochi-workflow.exe','mochi-clipper-host.exe','mochi-community.exe')) { continue }
    if ($ascii.Contains('RSDS')) { throw "CodeView/PDB reference found in $($file.Name)" }
    $pe = [BitConverter]::ToInt32($bytes, 60)
    if ([BitConverter]::ToUInt16($bytes, $pe + 4) -ne 0x8664) { throw "Not an x64 binary: $($file.Name)" }
    if ([BitConverter]::ToUInt32($bytes, $pe + 16) -ne 0) { throw "COFF symbols remain: $($file.Name)" }
    Write-Host "$($file.Name): x64, no COFF symbols or CodeView/PDB reference"
}
Write-Host "Verified $($files.Count) package files; no known personal build strings or development artifacts"
