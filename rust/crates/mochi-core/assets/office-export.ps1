$ErrorActionPreference = 'Stop'
$inputPath = [System.IO.Path]::GetFullPath($args[0])
$outputPath = [System.IO.Path]::GetFullPath($args[1])
$extension = [System.IO.Path]::GetExtension($inputPath).ToLowerInvariant()
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class MochiOfficeOwner {
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
  public static uint ProcessId(IntPtr hwnd) { uint id; GetWindowThreadProcessId(hwnd, out id); return id; }
}
'@
function Owns-Application($application, $previousIds) {
  try { return $previousIds -notcontains [MochiOfficeOwner]::ProcessId([IntPtr]$application.Hwnd) }
  catch { return $previousIds.Count -eq 0 }
}
function Release-Com($object) {
  if ($null -ne $object) { [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($object) }
}
if (@('.doc', '.docx') -contains $extension) {
  $word = $null
  $document = $null
  $ownsWord = $false
  $previousIds = @(Get-Process WINWORD -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id)
  $previousSecurity = $null
  $previousAlerts = $null
  try {
    $word = New-Object -ComObject Word.Application
    $ownsWord = Owns-Application $word $previousIds
    if ($ownsWord) { $word.Visible = $false }
    $previousSecurity = $word.AutomationSecurity
    $previousAlerts = $word.DisplayAlerts
    $word.DisplayAlerts = 0
    $word.AutomationSecurity = 3
    $document = $word.Documents.Open($inputPath, $false, $true)
    $document.ExportAsFixedFormat($outputPath, 17)
  } finally {
    if ($null -ne $document) { $document.Close($false) }
    if ($null -ne $word) {
      if ($null -ne $previousSecurity) { $word.AutomationSecurity = $previousSecurity }
      if ($null -ne $previousAlerts) { $word.DisplayAlerts = $previousAlerts }
      if ($ownsWord -and $word.Documents.Count -eq 0) { $word.Quit() }
    }
    Release-Com $document
    Release-Com $word
  }
} elseif (@('.ppt', '.pptx') -contains $extension) {
  $powerPoint = $null
  $presentation = $null
  $ownsPowerPoint = $false
  $previousIds = @(Get-Process POWERPNT -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id)
  $previousSecurity = $null
  try {
    $powerPoint = New-Object -ComObject PowerPoint.Application
    $ownsPowerPoint = Owns-Application $powerPoint $previousIds
    $previousSecurity = $powerPoint.AutomationSecurity
    $powerPoint.AutomationSecurity = 3
    $presentation = $powerPoint.Presentations.Open($inputPath, $true, $true, $false)
    $presentation.SaveAs($outputPath, 32)
  } finally {
    if ($null -ne $presentation) { $presentation.Close() }
    if ($null -ne $powerPoint) {
      if ($null -ne $previousSecurity) { $powerPoint.AutomationSecurity = $previousSecurity }
      if ($ownsPowerPoint -and $powerPoint.Presentations.Count -eq 0) { $powerPoint.Quit() }
    }
    Release-Com $presentation
    Release-Com $powerPoint
  }
} else { throw "Unsupported Office preview type: $extension" }
