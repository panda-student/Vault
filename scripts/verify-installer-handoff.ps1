param([Parameter(Mandatory=$true)][string]$Installer,[Parameter(Mandatory=$true)][string]$Directory,[Parameter(Mandatory=$true)][string]$Evidence)
$ErrorActionPreference='Stop'
if (!$Directory.StartsWith([IO.Path]::GetTempPath(),[StringComparison]::OrdinalIgnoreCase) -or $Directory -notlike '*vault-installer-handoff-*') { throw 'Only isolated installer fixtures are allowed' }
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$process=Start-Process -FilePath $Installer -ArgumentList ('/D='+$Directory) -WindowStyle Normal -PassThru
function Window {
 $condition=New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ProcessIdProperty,$process.Id)
 return [Windows.Automation.AutomationElement]::RootElement.FindFirst([Windows.Automation.TreeScope]::Children,$condition)
}
function Button($root) {
 $condition=New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::AutomationIdProperty,'1')
 return $root.FindFirst([Windows.Automation.TreeScope]::Descendants,$condition)
}
$until=(Get-Date).AddSeconds(20)
do { $window=Window; if(!$window){Start-Sleep -Milliseconds 150} } while(!$window -and (Get-Date) -lt $until)
if(!$window){throw 'Installer UI did not open'}
for($step=0;$step -lt 2;$step++) { (Button $window).GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke();Start-Sleep -Milliseconds 400 }
$edits=$window.FindAll([Windows.Automation.TreeScope]::Descendants,(New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Edit)))
$paths=@($edits|Where-Object {$_.Current.IsEnabled -and !$_.Current.IsOffscreen})
if($paths.Count -ne 1){throw ('Unexpected directory control count: '+$paths.Count)}
$value=$paths[0].GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern)
if($value.Current.IsReadOnly){throw 'Directory page is not active'}
$value.SetValue($Directory)
if($value.Current.Value -ne $Directory){throw 'Directory edit did not accept fixture path'}
(Button $window).GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
$until=(Get-Date).AddSeconds(130);$finished=$false
do {
 Start-Sleep -Milliseconds 200
 $window=Window
 if($window){$button=Button $window;if($button -and $button.Current.IsEnabled -and $button.Current.Name -match 'Finish|\u5b8c\u6210'){$finished=$true;break}}
}while(!$process.HasExited -and (Get-Date) -lt $until)
if(!$finished){throw 'Installer did not reach the finish page'}
# Keep the run checkbox enabled: the finish action must not start a duplicate desktop.
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class HandoffCapture { [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h,IntPtr dc,uint flags); }
'@
$bounds=$window.Current.BoundingRectangle
$bitmap=New-Object Drawing.Bitmap([int]$bounds.Width,[int]$bounds.Height)
$graphics=[Drawing.Graphics]::FromImage($bitmap);$dc=$graphics.GetHdc()
try{[void][HandoffCapture]::PrintWindow([IntPtr]$window.Current.NativeWindowHandle,$dc,2)}finally{$graphics.ReleaseHdc($dc)}
try{$bitmap.Save($Evidence,[Drawing.Imaging.ImageFormat]::Png)}finally{$graphics.Dispose();$bitmap.Dispose()}
$button.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
if(!$process.WaitForExit(10000) -or $process.ExitCode -ne 0){throw 'Installer did not exit successfully'}
