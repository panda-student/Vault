param([Parameter(Mandatory=$true)][string]$Title)
$ErrorActionPreference='Stop'
if(!$Title.StartsWith('vault browser fill audit ')){throw 'Only isolated browser fixture windows are allowed'}
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class BrowserFixtureWindow {
 public delegate bool Callback(IntPtr h,IntPtr p);
 [DllImport("user32.dll")] public static extern bool EnumWindows(Callback callback,IntPtr p);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h,StringBuilder b,int n);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
 [DllImport("user32.dll")] public static extern void keybd_event(byte key,byte scan,uint flags,UIntPtr extra);
 public static IntPtr Find(string title){IntPtr result=IntPtr.Zero;EnumWindows((h,p)=>{var b=new StringBuilder(1024);GetWindowText(h,b,1024);if(b.ToString().StartsWith(title)){result=h;return false;}return true;},IntPtr.Zero);return result;}
}
'@
$window=[BrowserFixtureWindow]::Find($Title)
if($window -eq [IntPtr]::Zero){throw 'Browser fixture window not found'}
[uint32]$owner=0
[void][BrowserFixtureWindow]::GetWindowThreadProcessId($window,[ref]$owner)
if((Get-Process -Id $owner).ProcessName -notin @('msedge','chrome')){throw 'Unexpected browser fixture process'}
for($attempt=0;$attempt -lt 10;$attempt++){
 [void][BrowserFixtureWindow]::SetForegroundWindow($window)
 if([BrowserFixtureWindow]::GetForegroundWindow() -eq $window){break}
 # Test setup only: activate the window, never select an input or type a value.
 [BrowserFixtureWindow]::keybd_event(0x12,0,0,[UIntPtr]::Zero)
 [BrowserFixtureWindow]::keybd_event(0x12,0,2,[UIntPtr]::Zero)
 Start-Sleep -Milliseconds 100
}
if([BrowserFixtureWindow]::GetForegroundWindow() -ne $window){throw 'Browser fixture did not acquire native foreground'}
@{processId=$owner;foreground=$true}|ConvertTo-Json -Compress
