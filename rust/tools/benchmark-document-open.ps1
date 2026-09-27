param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [string]$OutputDirectory = (Join-Path ([IO.Path]::GetTempPath()) ('mochi-document-benchmark-' + [Guid]::NewGuid().ToString('N'))),
    [int[]]$SizesKb = @(20, 100, 500)
)

$ErrorActionPreference = 'Stop'
$taskExecutable = (Resolve-Path -LiteralPath $Executable).Path
$taskOutput = [IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Path $taskOutput -Force | Out-Null
$taskChinese = -join (@(0x4e2d, 0x6587, 0x6b63, 0x6587, 0x6d4b, 0x8bd5, 0x6392, 0x7248) | ForEach-Object { [char]$_ })
$taskResults = foreach ($taskKilobytes in $SizesKb) {
    $taskBody = [Text.StringBuilder]::new()
    $taskRow = 0
    $taskBytes = 0
    while ($taskBytes -lt $taskKilobytes * 1024) {
        $taskParagraph = ('## Section {0}' -f $taskRow) + "`r`n`r`n" + ('{0} {1}. {0} {0}. The quick brown fox jumps over the lazy dog. **{0}** and `inline code`.' -f $taskChinese, $taskRow) + "`r`n`r`n"
        [void]$taskBody.Append($taskParagraph)
        $taskBytes += [Text.Encoding]::UTF8.GetByteCount($taskParagraph)
        $taskRow++
    }
    $taskInput = Join-Path $taskOutput ('document-{0}kb.md' -f $taskKilobytes)
    $taskImage = Join-Path $taskOutput ('document-{0}kb.png' -f $taskKilobytes)
    [IO.File]::WriteAllText($taskInput, $taskBody.ToString(), [Text.UTF8Encoding]::new($false))
    $taskStart = [Diagnostics.ProcessStartInfo]::new()
    $taskStart.FileName = $taskExecutable
    $taskStart.UseShellExecute = $false
    $taskStart.CreateNoWindow = $true
    $taskStart.RedirectStandardError = $true
    $taskStart.RedirectStandardOutput = $true
    $taskStart.Arguments = '--snapshot "file:' + $taskInput + '" "' + $taskImage + '"'
    $taskStart.EnvironmentVariables['MOCHI_SNAPSHOT_DPI'] = '120'
    $taskWatch = [Diagnostics.Stopwatch]::StartNew()
    $taskProcess = [Diagnostics.Process]::Start($taskStart)
    $taskError = $taskProcess.StandardError.ReadToEndAsync()
    $taskStdout = $taskProcess.StandardOutput.ReadToEndAsync()
    $taskPeakWorkingSet = 0L
    while (-not $taskProcess.WaitForExit(25)) {
        $taskProcess.Refresh()
        if (-not $taskProcess.HasExited) {
            $taskPeakWorkingSet = [math]::Max($taskPeakWorkingSet, $taskProcess.PeakWorkingSet64)
        }
        if ($taskWatch.ElapsedMilliseconds -ge 60000) {
            $taskProcess.Kill()
            throw ('Document benchmark timed out at {0} KB' -f $taskKilobytes)
        }
    }
    $taskWatch.Stop()
    if ($taskProcess.ExitCode -ne 0) { throw $taskError.GetAwaiter().GetResult() }
    if (-not (Test-Path -LiteralPath $taskImage)) { throw 'Snapshot was not produced' }
    [pscustomobject]@{
        bytes = (Get-Item -LiteralPath $taskInput).Length
        paragraphs = $taskRow
        elapsedMs = $taskWatch.ElapsedMilliseconds
        sampledPeakWorkingSetMB = if ($taskPeakWorkingSet -gt 0) { [math]::Round($taskPeakWorkingSet / 1MB, 2) } else { $null }
        image = $taskImage
    }
    $taskProcess.Dispose()
}
$taskReport = [ordered]@{
    executable = $taskExecutable
    measuredAt = [DateTime]::UtcNow.ToString('o')
    scenario = 'Snapshot: isolated workspace initialization, document read/layout, real DirectWrite and software Direct2D rendering, PNG encoding, process exit. End-to-end timings, not per-keystroke latency; build profile is determined by the executable.'
    results = @($taskResults)
}
$taskJson = $taskReport | ConvertTo-Json -Depth 5
[IO.File]::WriteAllText((Join-Path $taskOutput 'report.json'), $taskJson, [Text.UTF8Encoding]::new($false))
$taskJson
