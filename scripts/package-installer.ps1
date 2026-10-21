param([string]$ArtifactDirectory)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$artifactRoot = if ($ArtifactDirectory) { [IO.Path]::GetFullPath($ArtifactDirectory) } else { Join-Path $projectRoot 'artifacts' }
$version = (Get-Content -LiteralPath (Join-Path $projectRoot 'package.json') -Raw | ConvertFrom-Json).version
$package = Join-Path $artifactRoot "vault-$version"
if (!(Test-Path -LiteralPath (Join-Path $package 'Vault.exe'))) { throw 'Package the portable ZIP first.' }
$output = Join-Path $artifactRoot "Vault-$version-Setup.exe"
if (Test-Path -LiteralPath $output) { throw 'Installer already exists. Use a new artifact directory.' }
$toolRoot = Join-Path $env:LOCALAPPDATA 'tauri/nsis-3.13'
$compiler = Join-Path $toolRoot 'makensis.exe'
if (!(Test-Path -LiteralPath $compiler)) {
    $toolRoot = Join-Path $projectRoot 'artifacts/tools/nsis-3.13'
    $compiler = Join-Path $toolRoot 'makensis.exe'
    if (!(Test-Path -LiteralPath $compiler)) {
        $toolParent = Split-Path -Parent $toolRoot
        New-Item -ItemType Directory -Force -Path $toolParent | Out-Null
        $download = Join-Path $toolParent 'nsis-3.13.zip'
        Invoke-WebRequest 'https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.13/nsis-3.13.zip' -OutFile $download -UseBasicParsing
        $hash = (Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($hash -ne 'ba63dffc4410ee89193e1cb5a41989991bd77c61068da17e3156d136b7b0b3d8') { throw 'NSIS compiler checksum mismatch.' }
        Expand-Archive -LiteralPath $download -DestinationPath $toolParent
    }
}
& $compiler /INPUTCHARSET UTF8 "/DAPP_VERSION=$version" "/DPACKAGE_DIR=$package" "/DARTIFACT_DIR=$artifactRoot" (Join-Path $PSScriptRoot 'installer.nsi')
if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed.' }
$report = [ordered]@{ version=$version; bytes=(Get-Item -LiteralPath $output).Length; sha256=(Get-FileHash -LiteralPath $output -Algorithm SHA256).Hash.ToLowerInvariant(); defaultDirectory='D:\Vault'; dataPreservedOnUninstall=$true }
$report | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $artifactRoot 'installer-size.json') -Encoding UTF8
$report | ConvertTo-Json
