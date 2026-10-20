param([int]$ProcessId,[string]$ReportPath)
$ErrorActionPreference='Stop'
$process=Get-Process -Id $ProcessId
if ($process.ProcessName -notin @('local-password-manager','Vault')) { throw 'Unexpected fixture process' }
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class TrashDialogAudit {
 public delegate bool EnumProc(IntPtr h,IntPtr l);
 [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb,IntPtr l);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h,StringBuilder s,int n);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
 [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h,int id);
 [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h,uint msg,IntPtr w,IntPtr l);
 [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
}
'@
$checks=@()
try {
  foreach ($buttonId in @(7,6)) {
    $script:target=[IntPtr]::Zero
    $deadline=(Get-Date).AddSeconds(100)
    while ($script:target -eq [IntPtr]::Zero -and (Get-Date) -lt $deadline) {
      if (-not (Get-Process -Id $ProcessId -ErrorAction SilentlyContinue)) { throw 'Fixture exited before confirmation' }
      [TrashDialogAudit]::EnumWindows({
        param($h,$l)
        [uint32]$owner=0
        [void][TrashDialogAudit]::GetWindowThreadProcessId($h,[ref]$owner)
        if ($owner -eq $ProcessId -and [TrashDialogAudit]::IsWindowVisible($h)) {
          $title=[Text.StringBuilder]::new(256)
          [void][TrashDialogAudit]::GetWindowText($h,$title,256)
          if ($title.ToString() -eq '确认永久清空垃圾桶') { $script:target=$h;return $false }
        }
        return $true
      },[IntPtr]::Zero)|Out-Null
      if ($script:target -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 100 }
    }
    if ($script:target -eq [IntPtr]::Zero) { throw 'Expected native dialog missing' }
    $default=[TrashDialogAudit]::SendMessage($script:target,0x400,[IntPtr]::Zero,[IntPtr]::Zero).ToInt64() -band 0xffff
    if ($default -ne 7) { throw 'Cancel is not the default action' }
    $button=[TrashDialogAudit]::GetDlgItem($script:target,$buttonId)
    if ($button -eq [IntPtr]::Zero) { throw 'Confirmation button missing' }
    [void][TrashDialogAudit]::SendMessage($button,0xF5,[IntPtr]::Zero,[IntPtr]::Zero)
    $checks+=@{action=$(if($buttonId -eq 7){'cancel'}else{'confirm'});defaultCancel=$true;passed=$true}
    Start-Sleep -Milliseconds 300
  }
  @{passed=$true;checks=$checks}|ConvertTo-Json -Depth 4|Set-Content -LiteralPath $ReportPath -Encoding UTF8
} catch {
  @{passed=$false;checks=$checks;error='Native confirmation fixture failed'}|ConvertTo-Json -Depth 4|Set-Content -LiteralPath $ReportPath -Encoding UTF8
  throw
}
