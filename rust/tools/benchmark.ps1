param(
  [string]$Executable = (Join-Path $PSScriptRoot '../target/release/mochi-app.exe'),
  [string]$Output = (Join-Path $PSScriptRoot '../target/benchmark-native.json'),
  [ValidateRange(0,20000)][int]$NoteCount = 0
)
$ErrorActionPreference='Stop'
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public class MochiBenchmark {
  public delegate bool EnumProc(IntPtr h,IntPtr p);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc callback,IntPtr p);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h,StringBuilder s,int n);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h,uint m,IntPtr w,IntPtr l);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h,uint m,IntPtr w,IntPtr l);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h,out Rect r);
  public struct Rect { public int left,top,right,bottom; }
  public static IntPtr Find(uint wanted) {
    IntPtr found=IntPtr.Zero;
    EnumWindows((h,p)=>{uint pid;GetWindowThreadProcessId(h,out pid);var name=new StringBuilder(128);GetClassName(h,name,128);if(pid==wanted&&name.ToString()=="MochiNativeWindow"){found=h;return false;}return true;},IntPtr.Zero);
    return found;
  }
}
'@
$taskRoot=Join-Path ([IO.Path]::GetTempPath()) ('mochi-benchmark-'+[guid]::NewGuid().ToString('N'))
$taskWorkspace=Join-Path $taskRoot 'workspace'
New-Item -ItemType Directory -Path $taskWorkspace -Force | Out-Null
if($NoteCount -gt 0){
  $taskNotes=Join-Path $taskWorkspace '知识库/规模验收'
  New-Item -ItemType Directory -Path $taskNotes -Force | Out-Null
  for($taskIndex=0;$taskIndex -lt $NoteCount;$taskIndex++){
    $taskName='笔记{0:D5}' -f $taskIndex
    $taskNext='笔记{0:D5}' -f (($taskIndex+1)%$NoteCount)
    $taskBody="# $taskName`n`n中文检索🙂 needle$taskIndex`n`n[[$taskNext]]`n`n"+('这是隔离的规模验收正文 native workspace benchmark.'+"`n")*40
    [IO.File]::WriteAllText((Join-Path $taskNotes "$taskName.md"),$taskBody,[Text.UTF8Encoding]::new($false))
  }
}
$taskSettings=Join-Path $taskRoot 'settings.json'
$taskValues=@{'workspace.lastPath'=$taskWorkspace;'app.git.autoCommitEnabled'='false';'app.appearance.themeMode'='light'}
[IO.File]::WriteAllText($taskSettings,($taskValues|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
$taskPreviousSettings=$env:MOCHI_SETTINGS_PATH
$taskPreviousOffscreen=$env:MOCHI_VERIFY_OFFSCREEN
$taskForeground=[MochiBenchmark]::GetForegroundWindow()
$taskProcess=$null
try {
  $env:MOCHI_SETTINGS_PATH=$taskSettings
  $env:MOCHI_VERIFY_OFFSCREEN='1'
  $taskExe=(Resolve-Path -LiteralPath $Executable).Path
  $taskWatch=[Diagnostics.Stopwatch]::StartNew()
  $taskProcess=Start-Process -FilePath $taskExe -WorkingDirectory (Split-Path $taskExe) -WindowStyle Hidden -PassThru
  $taskWindow=[IntPtr]::Zero
  while($taskWatch.Elapsed.TotalSeconds -lt 120 -and $taskWindow -eq [IntPtr]::Zero){
    if($taskProcess.HasExited){throw "Native process exited: $($taskProcess.ExitCode)"}
    $taskWindow=[MochiBenchmark]::Find([uint32]$taskProcess.Id)
    if($taskWindow -eq [IntPtr]::Zero){Start-Sleep -Milliseconds 50}
  }
  if($taskWindow -eq [IntPtr]::Zero){throw 'Native HWND not created in 120 seconds'}
  $taskRect=[MochiBenchmark+Rect]::new()
  [void][MochiBenchmark]::GetWindowRect($taskWindow,[ref]$taskRect)
  if($taskRect.left -ge 0){throw 'Verification window was not placed offscreen'}
  [void][MochiBenchmark]::SendMessageW($taskWindow,0x000F,[IntPtr]::Zero,[IntPtr]::Zero)
  $taskFirstPaintMs=$taskWatch.ElapsedMilliseconds
  Start-Sleep -Seconds 5
  $taskProcess.Refresh()
  $taskResult=[ordered]@{
    executable=$taskExe;workspace=$taskWorkspace;scenario='isolated synthetic workspace; real offscreen HWND paint; no global hotkey registration';noteCount=$NoteCount
    firstPaintMs=$taskFirstPaintMs;workingSetMB=[math]::Round($taskProcess.WorkingSet64/1MB,2)
    privateMemoryMB=[math]::Round($taskProcess.PrivateMemorySize64/1MB,2);threads=$taskProcess.Threads.Count
    executableMB=[math]::Round((Get-Item -LiteralPath $taskExe).Length/1MB,2)
    foregroundUnchanged=([MochiBenchmark]::GetForegroundWindow() -eq $taskForeground)
  }
  [void][MochiBenchmark]::PostMessageW($taskWindow,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)
  $taskResult.closedGracefully=$taskProcess.WaitForExit(10000)
  $taskJson=$taskResult|ConvertTo-Json
  [IO.File]::WriteAllText([IO.Path]::GetFullPath($Output),$taskJson,[Text.UTF8Encoding]::new($false))
  Write-Output $taskJson
} finally {
  $env:MOCHI_SETTINGS_PATH=$taskPreviousSettings
  $env:MOCHI_VERIFY_OFFSCREEN=$taskPreviousOffscreen
  if($null -ne $taskProcess -and -not $taskProcess.HasExited){Stop-Process -Id $taskProcess.Id}
  # 保留明确命名的临时工作区供复查，不递归删除任何用户路径。
}
