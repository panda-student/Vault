param(
    [ValidateSet('Resize','Capture','Close','Invoke','Click')][string]$Action,
    [int]$Width=1120,
    [int]$Height=740,
    [string]$OutputPath,
    [string]$NamePattern,
    [int]$Index=-1,
    [int]$ClientX=-1,
    [int]$ClientY=-1
)
$ErrorActionPreference='Stop'
$runtime=Get-Content -LiteralPath (Join-Path $PSScriptRoot '../src-tauri/target/ui-audit-runtime.json') -Raw | ConvertFrom-Json
$process=Get-Process -Id $runtime.pid -ErrorAction Stop
$details=Get-CimInstance Win32_Process -Filter "ProcessId = $($runtime.pid)"
if ($process.Path -ne $runtime.executable -or !$details.CommandLine.Contains($runtime.directory)) { throw 'Not the isolated UI fixture' }
if ($Action -eq 'Close') { $process.CloseMainWindow() | Out-Null; exit }
if ($Action -eq 'Invoke') {
 if (!$NamePattern) { throw 'A fixture button name pattern is required' }
 Add-Type -AssemblyName UIAutomationClient
 Add-Type -AssemblyName UIAutomationTypes
 $element=[System.Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
 $condition=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty,[System.Windows.Automation.ControlType]::Button)
 $buttons=$element.FindAll([System.Windows.Automation.TreeScope]::Descendants,$condition)
 $matches=@($buttons | Where-Object { $_.Current.Name -like $NamePattern })
 if ($Index -ge 0) {
  if ($Index -ge $matches.Count) { throw 'Fixture button index outside matched controls' }
  $matched=$matches[$Index]
 } else {
  if ($matches.Count -ne 1) { throw "Expected exactly one isolated button, found $($matches.Count)" }
  $matched=$matches[0]
 }
 $pattern=$matched.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
 $pattern.Invoke()
 Write-Output 'Isolated native button invoked'
 exit
}
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class VaultAuditWindow {
 [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
 [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
 [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h,out Rect r);
 [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h,out Rect r);
 [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int w,int hgt,uint flags);
 [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h,IntPtr dc,uint flags);
 [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
 [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h,ref Point p);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point p);
 [DllImport("user32.dll")] public static extern void mouse_event(uint flags,uint x,uint y,uint data,UIntPtr extra);
}
'@
[VaultAuditWindow]::SetProcessDPIAware() | Out-Null
$handle=$process.MainWindowHandle
if ($handle -eq 0) { throw 'Fixture window not ready' }
$rect=New-Object VaultAuditWindow+Rect
$client=New-Object VaultAuditWindow+Rect
[VaultAuditWindow]::GetWindowRect($handle,[ref]$rect) | Out-Null
[VaultAuditWindow]::GetClientRect($handle,[ref]$client) | Out-Null
if ($Action -eq 'Click') {
 $scale=[VaultAuditWindow]::GetDpiForWindow($handle)/96.0
 $point=New-Object VaultAuditWindow+Point
 $point.X=[int]($ClientX*$scale);$point.Y=[int]($ClientY*$scale)
 if ($point.X -lt 0 -or $point.Y -lt 0 -or $point.X -ge $client.Right -or $point.Y -ge $client.Bottom) { throw 'Click outside isolated client' }
 [VaultAuditWindow]::SetForegroundWindow($handle) | Out-Null
 if ([VaultAuditWindow]::GetForegroundWindow() -ne $handle) { throw 'Isolated window is not foreground' }
 $oldPoint=New-Object VaultAuditWindow+Point
 [VaultAuditWindow]::GetCursorPos([ref]$oldPoint) | Out-Null
 [VaultAuditWindow]::ClientToScreen($handle,[ref]$point) | Out-Null
 try {
  [VaultAuditWindow]::SetCursorPos($point.X,$point.Y) | Out-Null
  if ([VaultAuditWindow]::GetForegroundWindow() -ne $handle) { throw 'Focus changed before click' }
  [VaultAuditWindow]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
  [VaultAuditWindow]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
 } finally { [VaultAuditWindow]::SetCursorPos($oldPoint.X,$oldPoint.Y) | Out-Null }
 Write-Output 'Clicked isolated native client'
} elseif ($Action -eq 'Resize') {
 $scale=[VaultAuditWindow]::GetDpiForWindow($handle)/96.0
 $outerWidth=[int]($Width*$scale)+($rect.Right-$rect.Left)-$client.Right
 $outerHeight=[int]($Height*$scale)+($rect.Bottom-$rect.Top)-$client.Bottom
 [VaultAuditWindow]::SetWindowPos($handle,[IntPtr]::Zero,0,0,$outerWidth,$outerHeight,0x16) | Out-Null
 Write-Output "Resized isolated client to $Width x $Height"
} else {
 if (!$OutputPath) { throw 'Output path required' }
 $bitmap=New-Object System.Drawing.Bitmap(($rect.Right-$rect.Left),($rect.Bottom-$rect.Top))
 $graphics=[System.Drawing.Graphics]::FromImage($bitmap)
 $dc=$graphics.GetHdc()
 try { if (![VaultAuditWindow]::PrintWindow($handle,$dc,2)) { throw 'Window capture failed' } }
 finally { $graphics.ReleaseHdc($dc) }
 try { $bitmap.Save([IO.Path]::GetFullPath($OutputPath),[System.Drawing.Imaging.ImageFormat]::Png) }
 finally { $graphics.Dispose();$bitmap.Dispose() }
 Write-Output $OutputPath
}
