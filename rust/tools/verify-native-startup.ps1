param(
    [string]$BinaryPath = 'rust/target/release/mochi-app.exe',
    [string]$OutputDir = 'rust/target/startup-smoke'
)

$ErrorActionPreference = 'Stop'
$taskRepo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$taskBinary = [IO.Path]::GetFullPath((Join-Path $taskRepo $BinaryPath))
$taskOutput = [IO.Path]::GetFullPath((Join-Path $taskRepo $OutputDir))
if (-not (Test-Path -LiteralPath $taskBinary -PathType Leaf)) { throw "Binary missing: $taskBinary" }
$taskRun = Join-Path $taskOutput ([Guid]::NewGuid().ToString('N'))
$taskWorkspace = Join-Path $taskRun 'workspace'
$null = New-Item -ItemType Directory -Path $taskWorkspace -Force
$taskSettings = Join-Path $taskRun 'settings.json'
@{ 'workspace.lastPath' = $taskWorkspace.Replace('\', '/') } | ConvertTo-Json | Set-Content -LiteralPath $taskSettings -Encoding UTF8
$taskPreviousSettings = $env:MOCHI_SETTINGS_FILE
$taskPreviousOffscreen = $env:MOCHI_VERIFY_OFFSCREEN
$taskProcess = $null
try {
    $env:MOCHI_SETTINGS_FILE = $taskSettings
    $env:MOCHI_VERIFY_OFFSCREEN = '1'
    $taskProcess = Start-Process -FilePath $taskBinary -WorkingDirectory $taskRepo -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $taskRun 'stdout.log') -RedirectStandardError (Join-Path $taskRun 'stderr.log')
    $null = $taskProcess.Handle
    # Debug 构建使用控制台子系统，因此调用 WaitForInputIdle 会抛出异常，
    # 即使进程马上就要创建原生窗口也是如此。
    $taskDeadline = [DateTime]::UtcNow.AddSeconds(20)
    do {
        $taskProcess.Refresh()
        if ($taskProcess.HasExited -or $taskProcess.MainWindowHandle -ne 0) { break }
        $null = $taskProcess.WaitForExit(100)
    } while ([DateTime]::UtcNow -lt $taskDeadline)
    if ($taskProcess.HasExited -or $taskProcess.MainWindowHandle -eq 0) { throw 'Native main window was not created.' }
    $taskWindow = $taskProcess.MainWindowHandle.ToInt64()
    if (-not $taskProcess.CloseMainWindow()) { throw 'Native main window did not accept close.' }
    if (-not $taskProcess.WaitForExit(10000)) { throw 'Native UI did not shut down within 10 seconds.' }
    $taskProcess.WaitForExit()
    if ($taskProcess.ExitCode -ne 0) { throw "Native UI exited with code $($taskProcess.ExitCode)." }
    if (-not (Test-Path -LiteralPath (Join-Path $taskWorkspace '.mochi/config.json'))) { throw 'Native UI did not restore the isolated workspace.' }
    $taskReport = [ordered]@{ passed = $true; binary = $taskBinary; realWindow = $true; windowHandle = $taskWindow; workspaceRestored = $true; cleanExit = $true; exitCode = $taskProcess.ExitCode; isolatedWorkspace = $taskWorkspace }
    $taskReport | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $taskOutput 'report.json') -Encoding UTF8
    $taskReport | ConvertTo-Json
}
finally {
    $env:MOCHI_SETTINGS_FILE = $taskPreviousSettings
    $env:MOCHI_VERIFY_OFFSCREEN = $taskPreviousOffscreen
    # 这里只能停止此脚本创建的临时测试进程。
    if ($null -ne $taskProcess) {
        $taskProcess.Refresh()
        if (-not $taskProcess.HasExited) { Stop-Process -Id $taskProcess.Id -Force }
    }
}
