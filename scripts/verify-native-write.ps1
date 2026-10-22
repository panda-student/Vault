param([ValidateSet('Launch','Focus','Save','Close','Capture')][string]$Action,[string]$StatePath,[ValidateSet('notepad','ise')][string]$Editor,[string]$FilePath)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class NativeWriteAudit {
 [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
 public delegate bool EnumProc(IntPtr h,IntPtr l);
 [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb,IntPtr l);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h,StringBuilder s,int n);
 [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h,int n);
 [DllImport("user32.dll")] public static extern void keybd_event(byte k,byte scan,uint flags,UIntPtr extra);
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h,out Rect r);
 [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h,IntPtr dc,uint flags);
 [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
 [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int key);
 public static string Title(IntPtr h) { var b=new StringBuilder(1024);GetWindowText(h,b,1024);return b.ToString(); }
 public static IntPtr Find(string name) { IntPtr result=IntPtr.Zero; EnumWindows((h,l)=>{ if(IsWindowVisible(h)&&Title(h).Contains(name)){result=h;return false;}return true;},IntPtr.Zero);return result; }
 public static void Shortcut(byte k) { keybd_event(0x11,0,0,UIntPtr.Zero);keybd_event(k,0,0,UIntPtr.Zero);keybd_event(k,0,2,UIntPtr.Zero);keybd_event(0x11,0,2,UIntPtr.Zero); }
}
'@
[NativeWriteAudit]::SetProcessDPIAware()|Out-Null
if($Action -eq 'Launch') {
 $full=[IO.Path]::GetFullPath($FilePath)
 if(!$full.StartsWith([IO.Path]::GetTempPath(),[StringComparison]::OrdinalIgnoreCase) -or !(Split-Path $full -Leaf).StartsWith('vault-write-audit-')) { throw 'Only isolated temporary audit files are permitted' }
 if($Editor -eq 'ise') {
  $process=Start-Process "$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell_ise.exe" -ArgumentList @('-NoProfile','-File',('"'+$full+'"')) -PassThru
  $handle=[IntPtr]::Zero
  for($attempt=0;$attempt -lt 120;$attempt++){Start-Sleep -Milliseconds 250;$process.Refresh();if($process.MainWindowHandle -ne 0){$handle=$process.MainWindowHandle;break}}
 } else {
  Start-Process "$env:SystemRoot/System32/notepad.exe" -ArgumentList ('"'+$full+'"') | Out-Null
  $handle=[IntPtr]::Zero
  for($attempt=0;$attempt -lt 100;$attempt++){Start-Sleep -Milliseconds 200;$handle=[NativeWriteAudit]::Find((Split-Path $full -Leaf));if($handle -ne [IntPtr]::Zero){break}}
 }
 if($handle -eq [IntPtr]::Zero){throw 'Audit editor window timed out'}
 if($Editor -eq 'ise') {
  $process.WaitForInputIdle(15000)|Out-Null
  $condition=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty,[IO.Path]::GetFileName($full))
  $ready=$false
  for($attempt=0;$attempt -lt 80;$attempt++){
   $root=[System.Windows.Automation.AutomationElement]::FromHandle($handle)
   if($null -ne $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants,$condition)){$ready=$true;break}
   Start-Sleep -Milliseconds 250
  }
  if(!$ready){throw 'Isolated editor file tab did not load'}
 }
 [uint32]$owner=0
 [NativeWriteAudit]::GetWindowThreadProcessId($handle,[ref]$owner)|Out-Null
 @{editor=$Editor;file=$full;handle=$handle.ToInt64();pid=$owner}|ConvertTo-Json|Set-Content -LiteralPath $StatePath -Encoding UTF8
 Write-Output '{"launched":true}'
 exit
}
$state=Get-Content -LiteralPath $StatePath -Raw|ConvertFrom-Json
$handle=[IntPtr]$state.handle
[uint32]$owner=0
[NativeWriteAudit]::GetWindowThreadProcessId($handle,[ref]$owner)|Out-Null
if($owner -ne $state.pid){throw 'Audit window owner changed'}
$process=Get-Process -Id $owner
if($state.editor -eq 'notepad'){
 # Tabbed Notepad updates its window title asynchronously after save. Wait for
 # the exact fixture title; never relax ownership or accept another active tab.
 for($attempt=0;$attempt -lt 20 -and ![NativeWriteAudit]::Title($handle).Contains((Split-Path $state.file -Leaf));$attempt++) { Start-Sleep -Milliseconds 100 }
 if($process.ProcessName -ne 'Notepad' -or ![NativeWriteAudit]::Title($handle).Contains((Split-Path $state.file -Leaf))){throw 'Not the isolated Notepad tab'}
} elseif($process.ProcessName -ne 'powershell_ise') {throw 'Not the isolated editor'}
[NativeWriteAudit]::ShowWindow($handle,9)|Out-Null
[NativeWriteAudit]::SetForegroundWindow($handle)|Out-Null
Start-Sleep -Milliseconds 300
for($attempt=0;$attempt -lt 10 -and [NativeWriteAudit]::GetForegroundWindow() -ne $handle;$attempt++){
 $shell=New-Object -ComObject WScript.Shell
 $shell.AppActivate([int]$owner)|Out-Null
 # This is fixture automation only; release credential writing never changes focus.
 [NativeWriteAudit]::keybd_event(0x12,0,0,[UIntPtr]::Zero)
 [NativeWriteAudit]::keybd_event(0x12,0,2,[UIntPtr]::Zero)
 [NativeWriteAudit]::SetForegroundWindow($handle)|Out-Null
 Start-Sleep -Milliseconds 200
}
if([NativeWriteAudit]::GetForegroundWindow() -ne $handle){throw 'Could not focus isolated editor'}
if($Action -eq 'Capture') {
 Add-Type -AssemblyName System.Drawing
 $rect=New-Object NativeWriteAudit+Rect
 [NativeWriteAudit]::GetWindowRect($handle,[ref]$rect)|Out-Null
 $bitmap=New-Object System.Drawing.Bitmap(($rect.Right-$rect.Left),($rect.Bottom-$rect.Top))
 $graphics=[System.Drawing.Graphics]::FromImage($bitmap)
 $dc=$graphics.GetHdc()
 try {if(![NativeWriteAudit]::PrintWindow($handle,$dc,2)){throw 'Isolated window capture failed'}}finally{$graphics.ReleaseHdc($dc)}
 # Capture only the isolated focused editor region, excluding unrelated tabs.
 $bounds=[System.Windows.Automation.AutomationElement]::FocusedElement.Current.BoundingRectangle
 $crop=New-Object System.Drawing.Rectangle([int]($bounds.X-$rect.Left),[int]($bounds.Y-$rect.Top),[int]$bounds.Width,[int]$bounds.Height)
 if($crop.X -lt 0 -or $crop.Y -lt 0 -or $crop.Right -gt $bitmap.Width -or $crop.Bottom -gt $bitmap.Height){$graphics.Dispose();$bitmap.Dispose();throw 'Editor capture region outside fixture window'}
 $cropped=$bitmap.Clone($crop,[System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
 try{$cropped.Save((Join-Path (Split-Path $StatePath -Parent) ($state.editor+'.png')),[System.Drawing.Imaging.ImageFormat]::Png)}finally{$cropped.Dispose();$graphics.Dispose();$bitmap.Dispose()}
 Write-Output '{"captured":true}'
} elseif($Action -eq 'Focus') {
 if($state.editor -eq 'notepad'){
  # Activating the fixture with Alt can leave a menu focused. Explicitly
  # select its writable document, never treat window activation as caret focus.
  $root=[System.Windows.Automation.AutomationElement]::FromHandle($handle)
  $document=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty,[System.Windows.Automation.ControlType]::Document)
  $edit=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty,[System.Windows.Automation.ControlType]::Edit)
  $condition=New-Object System.Windows.Automation.OrCondition($document,$edit)
  $editors=@($root.FindAll([System.Windows.Automation.TreeScope]::Descendants,$condition)|Where-Object {!$_.Current.IsOffscreen -and $_.Current.IsEnabled})
  if($editors.Count -ne 1){throw 'Isolated Notepad editor is not unique'}
  $editors[0].SetFocus()
  Start-Sleep -Milliseconds 150
 }
 if($state.editor -eq 'ise'){
  [NativeWriteAudit]::Shortcut(0x49)
  Start-Sleep -Milliseconds 300
  # ISE keeps its console focused when showing the script pane. Select the
  # upper script editor explicitly, not the equally writable console below it.
  $root=[System.Windows.Automation.AutomationElement]::FromHandle($handle)
  $condition=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty,'WpfTextView')
  $editors=@($root.FindAll([System.Windows.Automation.TreeScope]::Descendants,$condition)|Where-Object {!$_.Current.IsOffscreen -and $_.Current.BoundingRectangle.Height -gt 0}|Sort-Object {$_.Current.BoundingRectangle.Top})
  if($editors.Count -lt 1){throw 'Isolated script editor not found'}
  $editors[0].SetFocus()
  Start-Sleep -Milliseconds 150
 }
 [NativeWriteAudit]::Shortcut(0x23)
 Start-Sleep -Milliseconds 200
 # Wait for our shortcut and any human modifiers to be released; never
 # synthesize key-up events for keys the user may actually be holding.
 for($attempt=0;$attempt -lt 50;$attempt++){
  $held=@(0x10,0x11,0x12,0x5b,0x5c)|Where-Object {[NativeWriteAudit]::GetAsyncKeyState($_) -lt 0}
  if(@($held).Count -eq 0){break}
  Start-Sleep -Milliseconds 100
 }
 if(@($held).Count -gt 0){throw 'Modifier keys are still held'}
 $element=[System.Windows.Automation.AutomationElement]::FocusedElement
 Write-Output (@{focused=([NativeWriteAudit]::GetForegroundWindow() -eq $handle);controlType=$element.Current.ControlType.ProgrammaticName;processId=$owner}|ConvertTo-Json -Compress)
} elseif($Action -eq 'Save') {
 [NativeWriteAudit]::Shortcut(0x53)
 Start-Sleep -Milliseconds 400
 Write-Output '{"saveSent":true}'
} else {
 if($state.editor -eq 'notepad'){[NativeWriteAudit]::Shortcut(0x57)}else{$process.CloseMainWindow()|Out-Null}
 Write-Output '{"closed":true}'
}
