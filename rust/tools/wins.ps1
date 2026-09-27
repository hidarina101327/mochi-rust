# 列出 mochi-app 进程的全部顶层窗口（类名、标题、可见性、矩形）。
# 截图脚本靠 MainWindowHandle 找窗口；当它找错（比如找到控制台）时用这个排查。
Add-Type @'
using System; using System.Text; using System.Runtime.InteropServices;
public class Wins {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
  public struct R { public int L, T, Rr, B; }
  public static void Dump(uint pid) {
    EnumWindows((h, l) => {
      uint o; GetWindowThreadProcessId(h, out o);
      if (o == pid) {
        var t = new StringBuilder(256); GetWindowTextW(h, t, 256);
        var c = new StringBuilder(256); GetClassNameW(h, c, 256);
        R r; GetWindowRect(h, out r);
        Console.WriteLine("hwnd=" + h + " class=" + c + " title=[" + t + "] visible=" + IsWindowVisible(h) + " iconic=" + IsIconic(h) + " rect=" + r.L + "," + r.T + "-" + r.Rr + "," + r.B);
      }
      return true;
    }, IntPtr.Zero);
  }
}
'@
$p = Get-Process mochi-app -ErrorAction SilentlyContinue
if (-not $p) { Write-Output "not running"; exit 1 }
[Wins]::Dump([uint32]$p[0].Id)
