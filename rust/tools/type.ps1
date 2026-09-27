param(
  [string]$Out = (Join-Path $PSScriptRoot '../target/type.png'),
  [string]$Keys = "",          # 要敲的 ASCII
  [switch]$CtrlE,              # 兼容旧参数名：现在按 Ctrl+Shift+E 进入源码模式
  [Parameter(Mandatory=$true)][int]$TargetProcessId,
  [switch]$ChineseIme,         # 先切到中文输入法
  [int]$DoubleX = 0, [int]$DoubleY = 0
)

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class Typer {
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr SetFocus(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern uint SendInput(uint n, INPUT[] i, int size);
  [DllImport("user32.dll")] public static extern short VkKeyScanW(char c);
  [DllImport("user32.dll")] public static extern IntPtr LoadKeyboardLayoutW(string id, uint flags);
  [DllImport("user32.dll")] public static extern IntPtr ActivateKeyboardLayout(IntPtr hkl, uint flags);
  // SetForegroundWindow 从后台进程调用会被系统的前台保护挡掉（静默失败）。
  // 标准绕法：把自己的输入队列挂到目标线程上，抢过来再放开。
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint id, uint to, bool attach);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr pid);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
  public struct R { public int L, T, Rr, B; }
  [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT {
    public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr dwExtraInfo;
  }
  [StructLayout(LayoutKind.Sequential)] public struct INPUT {
    public uint type; public KEYBDINPUT ki; public int pad1; public int pad2;
  }
}
'@

$p = Get-Process -Id $TargetProcessId -ErrorAction Stop
if ($p.ProcessName -ne 'mochi-app') { throw '目标不是墨池原生进程' }
if (-not $p) { Write-Output "not running"; exit 1 }
$h = $p.MainWindowHandle
function Force-Focus([IntPtr]$target) {
  $me = [Typer]::GetCurrentThreadId()
  $fg = [Typer]::GetWindowThreadProcessId([Typer]::GetForegroundWindow(), [IntPtr]::Zero)
  $tt = [Typer]::GetWindowThreadProcessId($target, [IntPtr]::Zero)
  [Typer]::AttachThreadInput($me, $fg, $true) | Out-Null
  [Typer]::AttachThreadInput($me, $tt, $true) | Out-Null
  [Typer]::BringWindowToTop($target) | Out-Null
  [Typer]::SetForegroundWindow($target) | Out-Null
  [Typer]::SetFocus($target) | Out-Null
  Start-Sleep -Milliseconds 300
  [Typer]::AttachThreadInput($me, $tt, $false) | Out-Null
  [Typer]::AttachThreadInput($me, $fg, $false) | Out-Null
}

Force-Focus $h
Start-Sleep -Milliseconds 500
$got = [Typer]::GetForegroundWindow()
if ($got -ne $h) { throw ("no focus: fg={0} target={1} -- refusing to send keys" -f $got, $h) }

$dpi = [Typer]::GetDpiForWindow($h)
$scale = $dpi / 96.0

function Send-Key([uint16]$vk, [bool]$up) {
  if (-not $up -and [Typer]::GetForegroundWindow() -ne $h) { throw '目标已失去焦点，停止输入' }
  # PowerShell 里结构体是值类型：`$i.ki.wVk = x` 改的是取出来的**副本**，
  # 写不回 $i。必须先把 KEYBDINPUT 建完整再整个赋过去。
  # 踩过一次：所有按键都以 vk=0 到达应用。
  $ki = New-Object Typer+KEYBDINPUT
  $ki.wVk = $vk
  $ki.dwFlags = $(if ($up) { 2 } else { 0 })
  $i = New-Object Typer+INPUT
  $i.type = 1
  $i.ki = $ki
  [Typer]::SendInput(1, @($i), [System.Runtime.InteropServices.Marshal]::SizeOf($i)) | Out-Null
  Start-Sleep -Milliseconds 40
}

function Send-Char([char]$c) {
  $vk = [Typer]::VkKeyScanW($c)
  $key = [uint16]($vk -band 0xFF)
  $shift = ($vk -shr 8) -band 1
  if ($shift) { Send-Key 0x10 $false }
  Send-Key $key $false
  Send-Key $key $true
  if ($shift) { Send-Key 0x10 $true }
}

# 双击打开文件（这个走 SendMessage 就够，不涉及键盘状态）
if ($DoubleX -gt 0) {
  $px = [int]($DoubleX * $scale); $py = [int]($DoubleY * $scale)
  $lp = [IntPtr](($py -shl 16) -bor ($px -band 0xFFFF))
  [Typer]::SendMessageW($h, 0x0201, [IntPtr]1, $lp) | Out-Null
  [Typer]::SendMessageW($h, 0x0202, [IntPtr]0, $lp) | Out-Null
  [Typer]::SendMessageW($h, 0x0203, [IntPtr]1, $lp) | Out-Null
  [Typer]::SendMessageW($h, 0x0202, [IntPtr]0, $lp) | Out-Null
  Start-Sleep -Milliseconds 500
}

if ($CtrlE) {
  Send-Key 0x11 $false   # Ctrl
  Send-Key 0x10 $false   # Shift
  Send-Key 0x45 $false   # E
  Send-Key 0x45 $true
  Send-Key 0x10 $true
  Send-Key 0x11 $true
  Start-Sleep -Milliseconds 300
}

if ($ChineseIme) {
  # 中文键盘布局；随后输入法自己会把按键翻成组合串
  $hkl = [Typer]::LoadKeyboardLayoutW("00000804", 1)
  [Typer]::ActivateKeyboardLayout($hkl, 0) | Out-Null
  Start-Sleep -Milliseconds 500
}

if ($Keys) {
  foreach ($c in $Keys.ToCharArray()) { Send-Char $c }
  Start-Sleep -Milliseconds 600
}

$r = New-Object Typer+R
[Typer]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.Rr - $r.L; $ht = $r.B - $r.T
$bmp = New-Object System.Drawing.Bitmap $w, $ht
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
$bmp.Save($Out)
Write-Output "saved $Out ($w x $ht, dpi $dpi)"
