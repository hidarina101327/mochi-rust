param([Parameter(Mandatory=$true)][string]$BinaryPath)
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$source = Join-Path $repoRoot 'rust/crates/mochi-core/assets/release-guides'
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('mochi-release-guides-' + [guid]::NewGuid().ToString('N'))
$workspace = Join-Path $testRoot 'workspace'
$profile = Join-Path $testRoot 'settings.json'
$previousSettings = $env:MOCHI_SETTINGS_FILE
try {
    New-Item -ItemType Directory -Path $workspace -Force | Out-Null
    $values = @{ 'workspace.lastPath' = $workspace } | ConvertTo-Json
    [IO.File]::WriteAllText($profile, $values, [Text.UTF8Encoding]::new($false))
    $env:MOCHI_SETTINGS_FILE = $profile
    for ($pass = 1; $pass -le 2; $pass++) {
        if ($pass -eq 2) {
            [IO.File]::WriteAllText((Join-Path $workspace '知识库/墨池/墨池-快捷键.md'), 'outdated')
        }
        $process = Start-Process -FilePath $BinaryPath -ArgumentList '--install-bundled-guides' -WindowStyle Hidden -Wait -PassThru
        if ($process.ExitCode -ne 0) { throw "Guide installation failed: $($process.ExitCode)" }
        $files = @(Get-ChildItem -LiteralPath $source -File -Recurse)
        foreach ($file in $files) {
            $relative = [IO.Path]::GetRelativePath($source, $file.FullName)
            $installed = Join-Path (Join-Path $workspace '知识库/墨池') $relative
            if ((Get-FileHash -LiteralPath $file.FullName).Hash -ne (Get-FileHash -LiteralPath $installed).Hash) {
                throw "Guide mismatch: $relative"
            }
        }
        Write-Host "Pass $pass verified: $($files.Count) documents and images"
    }
} finally {
    $env:MOCHI_SETTINGS_FILE = $previousSettings
    $resolved = [IO.Path]::GetFullPath($testRoot)
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')
    if ((Split-Path -Parent $resolved).TrimEnd('\') -eq $tempRoot -and (Split-Path -Leaf $resolved) -match '^mochi-release-guides-[0-9a-f]{32}$') {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
