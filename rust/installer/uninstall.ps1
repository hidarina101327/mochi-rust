$ErrorActionPreference = 'Stop'

$installRoot = [IO.Path]::GetDirectoryName($PSCommandPath)
$startMenuRoot = Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::StartMenu)) 'Programs'

if (Get-Process -Name 'mochi-app' -ErrorAction SilentlyContinue) {
    throw 'Mochi Native is running. Close it before uninstalling.'
}

Remove-Item -LiteralPath (Join-Path $startMenuRoot 'Mochi Native.lnk') -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath (Join-Path $startMenuRoot 'Uninstall Mochi Native.lnk') -Force -ErrorAction SilentlyContinue

$cleanup = Join-Path ([IO.Path]::GetTempPath()) ('mochi-native-uninstall-' + [Guid]::NewGuid().ToString('N') + '.ps1')
$escapedRoot = $installRoot.Replace("'", "''")
$escapedCleanup = $cleanup.Replace("'", "''")
@"
Start-Sleep -Milliseconds 500
Remove-Item -LiteralPath '$escapedRoot' -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath '$escapedCleanup' -Force -ErrorAction SilentlyContinue
"@ | Set-Content -LiteralPath $cleanup -Encoding UTF8
Start-Process -FilePath (Get-Command powershell.exe).Source -WindowStyle Hidden -ArgumentList @('-NoLogo', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $cleanup)
Write-Host 'Uninstall started.'
