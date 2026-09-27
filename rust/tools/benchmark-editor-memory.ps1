param([Parameter(Mandatory=$true)][string]$Executable,[Parameter(Mandatory=$true)][string]$InputDirectory,[Parameter(Mandatory=$true)][string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
$taskExecutable = (Resolve-Path -LiteralPath $Executable).Path
$taskOutput = [IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Path $taskOutput -Force | Out-Null
$taskInputs = (Resolve-Path -LiteralPath $InputDirectory).Path
foreach ($taskCase in @(@('editor',20),@('editor',100),@('editor',500),@('memory',500))) {
    $taskKind = $taskCase[0]
    $taskSize = $taskCase[1]
    $taskStem = '{0}-{1}kb' -f $taskKind,$taskSize
    $taskInfo = [Diagnostics.ProcessStartInfo]::new()
    $taskInfo.FileName = $taskExecutable
    $taskInfo.UseShellExecute = $false
    $taskInfo.CreateNoWindow = $true
    $taskInfo.RedirectStandardOutput = $true
    $taskInfo.RedirectStandardError = $true
    $taskInfo.Arguments = '--snapshot "' + $taskKind + '-benchmark:' + (Join-Path $taskInputs ('document-{0}kb.md' -f $taskSize)) + '" "' + (Join-Path $taskOutput ($taskStem + '.png')) + '"'
    $taskInfo.EnvironmentVariables['MOCHI_SNAPSHOT_DPI'] = '120'
    $taskProcess = [Diagnostics.Process]::Start($taskInfo)
    $taskStdout = $taskProcess.StandardOutput.ReadToEndAsync()
    $taskStderr = $taskProcess.StandardError.ReadToEndAsync()
    if (-not $taskProcess.WaitForExit(240000)) { $taskProcess.Kill(); throw 'Benchmark timed out' }
    if ($taskProcess.ExitCode -ne 0) { throw $taskStderr.GetAwaiter().GetResult() }
    $taskReport = $taskStdout.GetAwaiter().GetResult() | ConvertFrom-Json
    $taskStages = [ordered]@{}
    foreach ($taskStage in $taskReport.resourceMemory.stages.PSObject.Properties) {
        $taskStages[$taskStage.Name] = [ordered]@{WorkingSetMB=[math]::Round($taskStage.Value.workingSetBytes/1MB,2);PrivateMB=[math]::Round($taskStage.Value.privateBytes/1MB,2)}
    }
    [ordered]@{Case=$taskStem;TypingMedianMs=$taskReport.typingAndPaint.medianMs;TypingMaxMs=$taskReport.typingAndPaint.maxMs;ArrowMedianMs=$taskReport.arrowAndPaint.medianMs;Memory=$taskStages;Checks=$taskReport.generated;InputUnchanged=$taskReport.inputFileUnchanged} | ConvertTo-Json -Depth 6
    $taskProcess.Dispose()
}
