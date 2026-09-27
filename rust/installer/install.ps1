$ErrorActionPreference = 'Stop'

try {
    $sourceRoot = [IO.Path]::GetDirectoryName($PSCommandPath)
    $localAppData = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
    $installRoot = Join-Path $localAppData 'Mochi Native'
    $installedExe = Join-Path $installRoot 'mochi-app.exe'
    $startMenuRoot = Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::StartMenu)) 'Programs'

    if (Get-Process -Name 'mochi-app' -ErrorAction SilentlyContinue) {
        throw 'Mochi Native 正在运行。请先退出应用后再安装更新。'
    }

    New-Item -ItemType Directory -Force -Path $installRoot | Out-Null
    New-Item -ItemType Directory -Force -Path $startMenuRoot | Out-Null
    Copy-Item -LiteralPath (Join-Path $sourceRoot 'mochi-app.exe') -Destination $installedExe -Force

    $sourceLicenses = Join-Path $sourceRoot 'licenses'
    if (Test-Path -LiteralPath $sourceLicenses) {
        Copy-Item -LiteralPath $sourceLicenses -Destination (Join-Path $installRoot 'licenses') -Recurse -Force
    }
    Copy-Item -LiteralPath (Join-Path $sourceRoot 'uninstall.ps1') -Destination (Join-Path $installRoot 'uninstall.ps1') -Force

    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut((Join-Path $startMenuRoot 'Mochi Native.lnk'))
    $shortcut.TargetPath = $installedExe
    $shortcut.WorkingDirectory = $installRoot
    $shortcut.IconLocation = "$installedExe,0"
    $shortcut.Description = 'Mochi Native'
    $shortcut.Save()

    $uninstall = $shell.CreateShortcut((Join-Path $startMenuRoot 'Uninstall Mochi Native.lnk'))
    $uninstall.TargetPath = (Get-Command powershell.exe).Source
    $uninstall.Arguments = '-NoLogo -NoProfile -ExecutionPolicy Bypass -File "' + (Join-Path $installRoot 'uninstall.ps1') + '"'
    $uninstall.WorkingDirectory = $installRoot
    $uninstall.Description = 'Uninstall Mochi Native'
    $uninstall.Save()

    # 双击安装包和应用内更新均复用这里：安装成功后启动新版，避免用户只看到
    # IExpress 的短暂进程就误以为“闪退”。
    Start-Process -FilePath $installedExe -WorkingDirectory $installRoot
}
catch {
    Add-Type -AssemblyName PresentationFramework
    [System.Windows.MessageBox]::Show(
        "Mochi Native 安装失败：`n$($_.Exception.Message)",
        'Mochi Native',
        'OK',
        'Error'
    ) | Out-Null
    exit 1
}
