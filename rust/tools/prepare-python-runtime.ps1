param(
    [string]$Destination = (Join-Path $PSScriptRoot '../target/native-dev/runtime/python'),
    [string]$ArchivePath
)
$ErrorActionPreference = 'Stop'
$manifest = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'python-runtime.json') -Raw | ConvertFrom-Json
$Destination = [IO.Path]::GetFullPath($Destination)
if ((Split-Path -Leaf $Destination) -ne 'python') { throw 'Destination must end with runtime/python (a dedicated Python directory).' }
if ((Split-Path -Leaf (Split-Path -Parent $Destination)) -ne 'runtime') { throw 'Destination parent must be runtime.' }
if (Test-Path -LiteralPath $Destination) {
    $installedManifest = Join-Path $Destination 'mochi-runtime.json'
    if (-not (Test-Path -LiteralPath $installedManifest)) { throw "Refusing to overwrite existing directory: $Destination" }
    $installed = Get-Content -LiteralPath $installedManifest -Raw | ConvertFrom-Json
    if ($installed.version -ne $manifest.version -or $installed.sha256 -ne $manifest.sha256) { throw 'Existing runtime differs; use a new staging directory to build the updated package.' }
    if (-not $installed.files) { throw 'Runtime verification manifest is missing.' }
    foreach ($file in $installed.files.PSObject.Properties) {
        if ($file.Name -match '[/\\:]' -or $file.Name -in @('.', '..')) { throw 'Invalid runtime manifest path.' }
        $item = Join-Path $Destination $file.Name
        if (-not (Test-Path -LiteralPath $item -PathType Leaf) -or (Get-FileHash -LiteralPath $item -Algorithm SHA256).Hash -ne $file.Value) { throw "Runtime file verification failed: $($file.Name)" }
    }
    Write-Host "Verified bundled Python $($manifest.version): $Destination"
    return
}
$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$stage = Join-Path $tempRoot ('mochi-python-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $stage | Out-Null
try {
    if ([string]::IsNullOrWhiteSpace($ArchivePath)) {
        $ArchivePath = Join-Path $stage 'python.zip'
        Invoke-WebRequest -UseBasicParsing -Uri $manifest.url -OutFile $ArchivePath -TimeoutSec 120
    } else {
        $ArchivePath = (Resolve-Path -LiteralPath $ArchivePath).Path
    }
    $hash = (Get-FileHash -LiteralPath $ArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($hash -ne $manifest.sha256) { throw 'Python archive SHA-256 mismatch. Nothing installed.' }
    $unpack = Join-Path $stage 'python'
    Expand-Archive -LiteralPath $ArchivePath -DestinationPath $unpack
    foreach ($name in @('python.exe', 'python314.dll', 'python314.zip', 'python314._pth', 'LICENSE.txt')) {
        if (-not (Test-Path -LiteralPath (Join-Path $unpack $name) -PathType Leaf)) { throw "Incomplete Python archive: $name" }
    }
    # 保留官方的 _pth 隔离设置。不使用 pip、PATH、注册表或系统级安装。
    $version = & (Join-Path $unpack 'python.exe') -I -X utf8 -c 'import sys; print(sys.version.split()[0])'
    if ($LASTEXITCODE -ne 0 -or $version.Trim() -ne $manifest.version) { throw 'Bundled Python smoke test failed.' }
    $files = [ordered]@{}
    Get-ChildItem -LiteralPath $unpack -File | ForEach-Object { $files[$_.Name] = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
    $record = @{ version = $manifest.version; sha256 = $hash; source = $manifest.source; files = $files } | ConvertTo-Json -Depth 5
    [IO.File]::WriteAllText((Join-Path $unpack 'mochi-runtime.json'), $record, [Text.UTF8Encoding]::new($false))
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Destination) | Out-Null
    # 源文件只取自本次调用的暂存子目录。绝不替换已存在的目标文件。
    $resolvedSource = (Resolve-Path -LiteralPath $unpack).Path
    if (-not $resolvedSource.StartsWith($stage + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid staging source.' }
    if (Test-Path -LiteralPath $Destination) { throw 'Destination appeared during preparation; refusing to overwrite.' }
    Move-Item -LiteralPath $resolvedSource -Destination $Destination
    Write-Host "Prepared bundled Python $version`: $Destination"
} finally {
    $resolvedStage = [IO.Path]::GetFullPath($stage)
    if ((Split-Path -Parent $resolvedStage).TrimEnd('\') -eq $tempRoot.TrimEnd('\') -and (Split-Path -Leaf $resolvedStage) -match '^mochi-python-[0-9a-f]{32}$') {
        if (Test-Path -LiteralPath $resolvedStage) { Remove-Item -LiteralPath $resolvedStage -Recurse -Force }
    } else { throw 'Refusing cleanup outside the verified temporary directory.' }
}
