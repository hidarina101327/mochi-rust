param(
    [string]$BinaryPath,
    [string]$TargetDir = (Join-Path $PSScriptRoot '../target'),
    [string]$OutputDir = (Join-Path $PSScriptRoot '../../release'),
    [string]$Version,
    [string]$InnoSetupCompiler,
    [string]$PythonArchivePath
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$TargetDir = [IO.Path]::GetFullPath($TargetDir)

if ([string]::IsNullOrWhiteSpace($Version)) {
    $metadata = cargo metadata --manifest-path (Join-Path $repoRoot 'rust/Cargo.toml') --no-deps --format-version 1 | ConvertFrom-Json
    $Version = $metadata.packages | Where-Object { $_.name -eq 'mochi-app' } | Select-Object -ExpandProperty version -First 1
}

if ($Version -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$') {
    throw "Version '$Version' is not a valid release version."
}

Push-Location $repoRoot
try {
    & node scripts/generate-icons.mjs
    if ($LASTEXITCODE -ne 0) { throw 'Icon generation failed.' }

    & cargo build --manifest-path rust/Cargo.toml --target-dir $TargetDir -p mochi-core --bin mochi-workflow --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Workflow worker build failed.' }
    & cargo build --manifest-path rust/Cargo.toml --target-dir $TargetDir -p mochi-core --bin mochi-clipper-host --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Web clipper host build failed.' }
    & cargo build --manifest-path rust/Cargo.toml --target-dir $TargetDir -p mochi-core --bin mochi-community --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Community CLI build failed.' }

    if ([string]::IsNullOrWhiteSpace($BinaryPath)) {
        & cargo build --manifest-path rust/Cargo.toml --target-dir $TargetDir -p mochi-app --release --locked
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
        $BinaryPath = Join-Path $TargetDir 'release/mochi-app.exe'
    }
}
finally {
    Pop-Location
}

if (-not [System.IO.Path]::IsPathRooted($BinaryPath)) {
    $BinaryPath = Join-Path $repoRoot $BinaryPath
}
$BinaryPath = (Resolve-Path -LiteralPath $BinaryPath).Path
if (-not (Test-Path -LiteralPath $BinaryPath -PathType Leaf)) {
    throw "Application binary not found: $BinaryPath"
}

if ([string]::IsNullOrWhiteSpace($InnoSetupCompiler)) {
    $programFilesX86 = [Environment]::GetFolderPath('ProgramFilesX86')
    $programFiles = [Environment]::GetFolderPath('ProgramFiles')
    $candidates = @(
        (Get-Command 'ISCC.exe' -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source -First 1),
        (Join-Path $programFilesX86 'Inno Setup 6\ISCC.exe'),
        (Join-Path $programFilesX86 'Inno Setup 7\ISCC.exe'),
        (Join-Path $programFiles 'Inno Setup 6\ISCC.exe'),
        (Join-Path $programFiles 'Inno Setup 7\ISCC.exe')
    ) | Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Leaf) }
    $InnoSetupCompiler = $candidates | Select-Object -First 1
}

if ([string]::IsNullOrWhiteSpace($InnoSetupCompiler) -or -not (Test-Path -LiteralPath $InnoSetupCompiler -PathType Leaf)) {
    throw 'Inno Setup 6 or newer is required. Install it from https://jrsoftware.org/isinfo.php and run this command again.'
}
$InnoSetupCompiler = (Resolve-Path -LiteralPath $InnoSetupCompiler).Path

$OutputDir = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $OutputDir))
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
$stage = Join-Path ([System.IO.Path]::GetTempPath()) ("mochi-package-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $stage | Out-Null

try {
    Copy-Item -LiteralPath $BinaryPath -Destination (Join-Path $stage 'Mochi.exe')
    Copy-Item -LiteralPath (Join-Path $TargetDir 'release/mochi-workflow.exe') -Destination (Join-Path $stage 'mochi-workflow.exe')
    Copy-Item -LiteralPath (Join-Path $TargetDir 'release/mochi-clipper-host.exe') -Destination (Join-Path $stage 'mochi-clipper-host.exe')
    Copy-Item -LiteralPath (Join-Path $TargetDir 'release/mochi-community.exe') -Destination (Join-Path $stage 'mochi-community.exe')
    Copy-Item -LiteralPath (Join-Path $repoRoot 'build/icon.ico') -Destination (Join-Path $stage 'Mochi.ico')
    $licensesSource = Join-Path (Split-Path -Parent $BinaryPath) 'licenses'
    if (-not (Test-Path -LiteralPath $licensesSource -PathType Container)) {
        throw "Bundled license directory not found: $licensesSource"
    }
    Copy-Item -LiteralPath $licensesSource -Destination (Join-Path $stage 'licenses') -Recurse
    & (Join-Path $PSScriptRoot 'prepare-python-runtime.ps1') -Destination (Join-Path $stage 'runtime/python') -ArchivePath $PythonArchivePath
    if (-not (Test-Path -LiteralPath (Join-Path $stage 'runtime/python/python.exe'))) { throw 'Bundled Python missing.' }
    Copy-Item -LiteralPath (Join-Path $stage 'runtime/python/LICENSE.txt') -Destination (Join-Path $stage 'licenses/Python-LICENSE.txt')

    $env:MOCHI_PACKAGE_VERSION = $Version
    $env:MOCHI_PACKAGE_STAGE = $stage
    $env:MOCHI_PACKAGE_OUTPUT = $OutputDir
    & $InnoSetupCompiler (Join-Path $repoRoot 'rust/installer/Mochi.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Inno Setup compilation failed.' }
}
finally {
    Remove-Item Env:MOCHI_PACKAGE_VERSION -ErrorAction SilentlyContinue
    Remove-Item Env:MOCHI_PACKAGE_STAGE -ErrorAction SilentlyContinue
    Remove-Item Env:MOCHI_PACKAGE_OUTPUT -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $stage) {
        $verifiedStage = [IO.Path]::GetFullPath($stage)
        if ((Split-Path -Parent $verifiedStage).TrimEnd('\') -ne ([IO.Path]::GetFullPath([IO.Path]::GetTempPath())).TrimEnd('\') -or (Split-Path -Leaf $verifiedStage) -notmatch '^mochi-package-[0-9a-f]{32}$') { throw 'Refusing cleanup outside verified package staging directory.' }
        Remove-Item -LiteralPath $stage -Recurse -Force
    }
}

$installer = Join-Path $OutputDir "Mochi-$Version-x64-Setup.exe"
if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) {
    throw "Installer was not produced: $installer"
}

$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $installer).Hash.ToLowerInvariant()
[System.IO.File]::WriteAllText("$installer.sha256", "$hash  $(Split-Path -Leaf $installer)`n", [System.Text.Encoding]::ASCII)

Write-Host "Installer: $installer"
Write-Host "SHA-256:  $hash"

# 不运行 UPX 等可执行文件压缩工具：它们几乎不能保护 Rust 代码，
# 却更容易触发 Windows Defender 或 SmartScreen。
# 发布配置会去除符号、启用 LTO，并省略调试产物。
