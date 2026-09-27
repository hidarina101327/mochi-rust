param([string]$Out = (Join-Path $PSScriptRoot '../target/shot.png'), [int]$ClickX = 0, [int]$ClickY = 0, [int]$DoubleX = 0, [int]$DoubleY = 0, [int]$RightX = 0, [int]$RightY = 0, [string]$Type = "", [string]$Keys = "", [string]$Hotkey = "", [Parameter(Mandatory=$true)][ValidateRange(1,2147483647)][int]$TargetProcessId)

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class Shot {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, System.Text.StringBuilder s, int n);
  public static IntPtr FindProcessWindow(int pid) {
    IntPtr result = IntPtr.Zero;
    EnumWindows((h, l) => {
      uint owner; GetWindowThreadProcessId(h, out owner);
      var cls = new System.Text.StringBuilder(256); GetClassNameW(h, cls, 256);
      if (owner == pid && cls.ToString() == "MochiNativeWindow") { result = h; return false; }
      return true;
    }, IntPtr.Zero);
    return result;
  }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out R r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  // title 用 IntPtr：PowerShell 会把 $null 字符串传成 ""，那就变成「标题为空的窗口」，永远找不到
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls, IntPtr title);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref P p);
  [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  // 不设这个，本进程拿到的是被系统虚拟化过的逻辑坐标，
  // 而 CopyFromScreen 用物理坐标——两者对不上，截出来的图整体偏移。
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  // PrintWindow 让窗口把自己画进我们给的 DC，与 z 序无关——
  // CopyFromScreen 抓的是屏幕，被别的窗口挡住就拍到别人。
  // PW_RENDERFULLCONTENT (0x2) 是 D2D/D3D 窗口能被抓到的关键。
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
  // 真实按键：应用靠 GetKeyState 判 Ctrl/Shift，SendMessage 的 WM_KEYDOWN 骗不过它
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  public struct R { public int L, T, Rr, B; }
  public struct P { public int X, Y; }
}
'@
[Shot]::SetProcessDPIAware() | Out-Null

$p = Get-Process -Id $TargetProcessId -ErrorAction SilentlyContinue
if (-not $p) { Write-Output "not running"; exit 1 }
if ($p.ProcessName -ne 'mochi-app') { Write-Output "target is not mochi-app"; exit 1 }
# 按类名找，别用 MainWindowHandle——窗口被最小化时它会指到控制台或输入法窗口
$h = [Shot]::FindProcessWindow($p.Id)
if ($h -eq [IntPtr]::Zero) { Write-Output "window not found"; exit 1 }
# 用 -WindowStyle Hidden 启动（为了不带出控制台）时，首个 ShowWindow 会被 STARTUPINFO 压成隐藏
if (-not [Shot]::IsWindowVisible($h)) { [Shot]::ShowWindow($h, 5) | Out-Null; Start-Sleep -Milliseconds 400 }  # SW_SHOW
if ([Shot]::IsIconic($h)) { [Shot]::ShowWindow($h, 9) | Out-Null; Start-Sleep -Milliseconds 400 }  # SW_RESTORE
[Shot]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 500

# 客户区坐标是 DIP；消息要物理像素
$dpi = [Shot]::GetDpiForWindow($h)
$scale = $dpi / 96.0
function Send-Click([int]$dx, [int]$dy, [bool]$double) {
  # 参数按 DIP 给（和应用的布局坐标系一致）；WM_LBUTTON* 的 lParam 要**物理**像素，
  # 应用收到后自己除以缩放系数换回 DIP。所以这里乘一次。
  $px = [int]($dx * $scale); $py = [int]($dy * $scale)
  $lp = [IntPtr](($py -shl 16) -bor ($px -band 0xFFFF))
  [Shot]::SendMessageW($h, 0x0201, [IntPtr]1, $lp) | Out-Null   # WM_LBUTTONDOWN
  [Shot]::SendMessageW($h, 0x0202, [IntPtr]0, $lp) | Out-Null   # WM_LBUTTONUP
  if ($double) {
    [Shot]::SendMessageW($h, 0x0203, [IntPtr]1, $lp) | Out-Null # WM_LBUTTONDBLCLK
    [Shot]::SendMessageW($h, 0x0202, [IntPtr]0, $lp) | Out-Null
  }
  Start-Sleep -Milliseconds 350
}

function Send-RightClick([int]$dx, [int]$dy) {
  $px = [int]($dx * $scale); $py = [int]($dy * $scale)
  $lp = [IntPtr](($py -shl 16) -bor ($px -band 0xFFFF))
  [Shot]::SendMessageW($h, 0x0204, [IntPtr]2, $lp) | Out-Null   # WM_RBUTTONDOWN
  [Shot]::SendMessageW($h, 0x0205, [IntPtr]0, $lp) | Out-Null   # WM_RBUTTONUP
  Start-Sleep -Milliseconds 350
}

# 逐字符发 WM_CHAR（走 IME 之外的普通输入路径）
function Send-Text([string]$text) {
  foreach ($ch in $text.ToCharArray()) {
    [Shot]::SendMessageW($h, 0x0102, [IntPtr][int]$ch, [IntPtr]0) | Out-Null
  }
  Start-Sleep -Milliseconds 200
}

# 发虚拟键（WM_KEYDOWN + WM_KEYUP），逗号分隔的十进制键码，如 "13" 回车、"27" Esc
function Send-Keys([string]$codes) {
  foreach ($c in $codes.Split(',')) {
    if ($c.Trim() -eq '') { continue }
    $vk = [int]$c.Trim()
    [Shot]::SendMessageW($h, 0x0100, [IntPtr]$vk, [IntPtr]0) | Out-Null
    [Shot]::SendMessageW($h, 0x0101, [IntPtr]$vk, [IntPtr]0) | Out-Null
  }
  Start-Sleep -Milliseconds 250
}

# 组合键，如 "Ctrl+P" / "Ctrl+Shift+F"：按下修饰键 → 主键 → 松开（窗口已在前台）
function Send-Hotkey([string]$combo) {
  $parts = $combo.Split('+') | % { $_.Trim() }
  $mods = @()
  $main = $null
  foreach ($p in $parts) {
    switch ($p.ToLower()) {
      'ctrl'  { $mods += 0x11 }
      'shift' { $mods += 0x10 }
      'alt'   { $mods += 0x12 }
      default { $main = [int][char]$p.ToUpper()[0] }
    }
  }
  # 不是前台就不发：真实按键会落到别的程序里
  if ([Shot]::GetForegroundWindow() -ne $h) { Write-Output "hotkey skipped: window not foreground"; return }
  foreach ($m in $mods) { [Shot]::keybd_event([byte]$m, 0, 0, [UIntPtr]::Zero) }
  Start-Sleep -Milliseconds 30
  if ($main) { [Shot]::keybd_event([byte]$main, 0, 0, [UIntPtr]::Zero); [Shot]::keybd_event([byte]$main, 0, 2, [UIntPtr]::Zero) }
  Start-Sleep -Milliseconds 30
  foreach ($m in $mods) { [Shot]::keybd_event([byte]$m, 0, 2, [UIntPtr]::Zero) }
  Start-Sleep -Milliseconds 400
}

if ($ClickX -gt 0) { Send-Click $ClickX $ClickY $false }
if ($DoubleX -gt 0) { Send-Click $DoubleX $DoubleY $true }
if ($RightX -gt 0) { Send-RightClick $RightX $RightY }
if ($Hotkey -ne "") { Send-Hotkey $Hotkey }
if ($Type -ne "") { Send-Text $Type }
if ($Keys -ne "") { Send-Keys $Keys }

$r = New-Object Shot+R
[Shot]::GetWindowRect($h, [ref]$r) | Out-Null
$c = New-Object Shot+R
[Shot]::GetClientRect($h, [ref]$c) | Out-Null
$w = $r.Rr - $r.L; $ht = $r.B - $r.T
$bmp = New-Object System.Drawing.Bitmap $w, $ht
$g = [System.Drawing.Graphics]::FromImage($bmp)
$dc = $g.GetHdc()
$ok = if ($env:SHOT_SCREEN -eq "1") { $false } else { [Shot]::PrintWindow($h, $dc, 2) }
$g.ReleaseHdc($dc)
if (-not $ok) {
  # 极少数情况下 PrintWindow 会失败，退回抓屏
  $g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
}
$bmp.Save($Out)
Write-Output "saved $Out | window $w x $ht | client $($c.Rr) x $($c.B) physical = $([math]::Round($c.Rr/$scale)) x $([math]::Round($c.B/$scale)) DIP | dpi $dpi"
