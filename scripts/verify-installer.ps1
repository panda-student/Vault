param([string]$ArtifactDirectory='artifacts', [ValidateSet('Install','Reinstall','Remove')][string]$Phase='Install', [switch]$Silent)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$projectRoot=Split-Path -Parent $PSScriptRoot
$artifacts=[IO.Path]::GetFullPath($ArtifactDirectory)
$reportPath=Join-Path $artifacts 'installer-verification.json'
$installKey='HKCU:/Software/Microsoft/Windows/CurrentVersion/Uninstall/Vault'
$shortcut=Join-Path ([Environment]::GetFolderPath('Programs')) 'Vault/Vault.lnk'
function Check([string]$Name,[bool]$Passed) {
    $script:report.checks+=@{name=$Name;passed=$Passed}
    if (!$Passed) { throw "Installer acceptance failed: $Name" }
    Write-Output "PASS $Name"
}
function Find-Window([int]$Owner) {
    $until=(Get-Date).AddSeconds(20)
    do {
        $condition=New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ProcessIdProperty,$Owner)
        $window=[Windows.Automation.AutomationElement]::RootElement.FindFirst([Windows.Automation.TreeScope]::Children,$condition)
        if ($window) { return $window }
        Start-Sleep -Milliseconds 150
    } while ((Get-Date) -lt $until)
    throw 'Installer window unavailable'
}
function Next-Page($Window) {
    $condition=New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::AutomationIdProperty,'1')
    $button=$Window.FindFirst([Windows.Automation.TreeScope]::Descendants,$condition)
    $button.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 400
}
function Capture-Page($Window,[string]$Name) {
    Add-Type -AssemblyName System.Drawing
    if (!('InstallerCapture' -as [type])) {
        Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class InstallerCapture {
 [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h,IntPtr dc,uint flags);
}
'@
    }
    $rectangle=$Window.Current.BoundingRectangle
    $bitmap=New-Object Drawing.Bitmap([int]$rectangle.Width,[int]$rectangle.Height)
    $graphics=[Drawing.Graphics]::FromImage($bitmap)
    $dc=$graphics.GetHdc()
    try { [void][InstallerCapture]::PrintWindow([IntPtr]$Window.Current.NativeWindowHandle,$dc,2) }
    finally { $graphics.ReleaseHdc($dc) }
    try { $bitmap.Save((Join-Path $artifacts $Name),[Drawing.Imaging.ImageFormat]::Png) }
    finally { $graphics.Dispose();$bitmap.Dispose() }
}
try {
    if ($Phase -eq 'Install') {
        if ((Test-Path 'HKCU:/Software/Vault') -or (Test-Path $installKey) -or (Test-Path $shortcut)) { throw 'Existing installed Vault registration detected; refusing to change it during acceptance.' }
        $version=(Get-Content (Join-Path $projectRoot 'package.json') -Raw | ConvertFrom-Json).version
        $fixture=Join-Path ([IO.Path]::GetTempPath()) ('vault-installer-'+[guid]::NewGuid().ToString('N'))
        $directory=Join-Path $fixture 'custom path/Vault'
        New-Item -ItemType Directory -Force -Path (Join-Path $directory 'data') | Out-Null
        [IO.File]::WriteAllText((Join-Path $directory 'data/keep.txt'),'SYNTHETIC_INSTALLER_DATA_ONLY')
        $script:report=@{passed=$false;version=$version;directory=$directory;fixture=$fixture;checks=@();nativeWizard=(!$Silent)}
        $installer=Join-Path $artifacts "Vault-$version-Setup.exe"
        if ($Silent) {
            $process=Start-Process -FilePath $installer -ArgumentList @('/S',('/D='+$directory)) -WindowStyle Hidden -PassThru -Wait
        } else {
            $process=Start-Process -FilePath $installer -ArgumentList ('/D='+$directory) -WindowStyle Normal -PassThru
            $window=Find-Window $process.Id
            Next-Page $window
            Next-Page $window
            $condition=New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Edit)
            $edits=$window.FindAll([Windows.Automation.TreeScope]::Descendants,$condition)
            $pathEdit=@($edits | Where-Object { $_.Current.IsEnabled -and !$_.Current.IsOffscreen }) | Select-Object -First 1
            if (!$pathEdit) { throw 'Installation directory input missing' }
            $value=$pathEdit.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern)
            Check 'native-test-directory-explicit' ($value.Current.Value -eq $directory)
            Capture-Page $window 'installer-default-directory.png'
            $value.SetValue($directory)
            Check 'native-directory-editable' ($value.Current.Value -eq $directory)
            Next-Page $window
            $until=(Get-Date).AddSeconds(30)
            while (!(Test-Path (Join-Path $directory 'Uninstall.exe')) -and (Get-Date) -lt $until) { Start-Sleep -Milliseconds 200 }
            Start-Sleep -Milliseconds 800
            $boxes=$window.FindAll([Windows.Automation.TreeScope]::Descendants,(New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::CheckBox)))
            foreach ($box in $boxes) {
                if (!$box.Current.IsOffscreen) {
                    $toggle=$box.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern)
                    if ($toggle.Current.ToggleState -eq [Windows.Automation.ToggleState]::On) { $toggle.Toggle() }
                }
            }
            Capture-Page $window 'installer-finished.png'
            Next-Page $window
            if (!$process.WaitForExit(10000)) { throw 'Installer did not finish' }
        }
        Check 'installer-success' ($process.ExitCode -eq 0)
        $installed=Join-Path $directory 'Vault.exe'
        Check 'installed-binary-matches-portable' ((Get-FileHash $installed).Hash -eq (Get-FileHash (Join-Path $artifacts "vault-$version/Vault.exe")).Hash)
        Check 'existing-data-preserved-on-install' ((Get-Content (Join-Path $directory 'data/keep.txt') -Raw) -eq 'SYNTHETIC_INSTALLER_DATA_ONLY')
        Check 'per-user-uninstall-registration' ((Get-ItemProperty $installKey).DisplayVersion -eq $version)
        $link=(New-Object -ComObject WScript.Shell).CreateShortcut($shortcut)
        Check 'start-menu-targets-installed-executable' ($link.TargetPath -eq $installed)
    } else {
        $script:report=Get-Content $reportPath -Raw | ConvertFrom-Json
        $directory=$report.directory
        if (![IO.Path]::GetFullPath($directory).StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath())) -or $directory -notlike '*vault-installer-*') { throw 'Unrecognized test fixture' }
        if ($Phase -eq 'Reinstall') {
            $version=$report.version
            $installer=Join-Path $artifacts "Vault-$version-Setup.exe"
            $process=Start-Process -FilePath $installer -ArgumentList @('/S',('/D='+$directory)) -WindowStyle Hidden -Wait -PassThru
            Check 'reinstall-success' ($process.ExitCode -eq 0)
            Check 'data-preserved-on-reinstall' ((Get-Content (Join-Path $directory 'data/keep.txt') -Raw) -eq 'SYNTHETIC_INSTALLER_DATA_ONLY')
        } else {
        $uninstaller=Join-Path $directory 'Uninstall.exe'
        Start-Process -FilePath $uninstaller -ArgumentList '/S' -WindowStyle Hidden -Wait | Out-Null
        $until=(Get-Date).AddSeconds(15)
        while ((Test-Path (Join-Path $directory 'Vault.exe')) -and (Get-Date) -lt $until) { Start-Sleep -Milliseconds 200 }
        Check 'uninstall-removes-program' (!(Test-Path (Join-Path $directory 'Vault.exe')))
        Check 'uninstall-preserves-data' ((Get-Content (Join-Path $directory 'data/keep.txt') -Raw) -eq 'SYNTHETIC_INSTALLER_DATA_ONLY')
        Check 'uninstall-removes-registration-and-shortcut' (!(Test-Path $installKey) -and !(Test-Path 'HKCU:/Software/Vault') -and !(Test-Path $shortcut))
        $report.passed=$true
        }
    }
} catch {
    if ($report) { $report | Add-Member -NotePropertyName error -NotePropertyValue $_.Exception.Message -Force }
    throw
} finally {
    if ($report) { $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $reportPath -Encoding UTF8 }
}
