param([string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
function Get-Sha256([string]$Path) {
    $stream = [System.IO.File]::OpenRead($Path)
    $hasher = [System.Security.Cryptography.SHA256]::Create()
    try { return ([System.BitConverter]::ToString($hasher.ComputeHash($stream))).Replace('-', '').ToLowerInvariant() }
    finally { $stream.Dispose(); $hasher.Dispose() }
}
$projectRoot = Split-Path -Parent $PSScriptRoot
$releaseBinary = Join-Path $projectRoot 'src-tauri\target\release\local-password-manager.exe'
if (!(Test-Path -LiteralPath $releaseBinary)) { throw 'Run npm run desktop:build first.' }
$version = (Get-Content -LiteralPath (Join-Path $projectRoot 'package.json') -Raw | ConvertFrom-Json).version
$artifactRoot = if ($OutputDirectory) { [System.IO.Path]::GetFullPath($OutputDirectory) } else { Join-Path $projectRoot 'artifacts' }
$packageName = "vault-$version"
$packageDirectory = Join-Path $artifactRoot $packageName
New-Item -ItemType Directory -Path $packageDirectory -Force | Out-Null
# Only known distribution files belong here. Runtime caches are never included.
$existing = @(Get-ChildItem -LiteralPath $packageDirectory -Force)
if ($existing.Count -gt 0) { throw 'Package directory already exists and is not empty. Move it aside before repackaging.' }
Copy-Item -LiteralPath $releaseBinary -Destination (Join-Path $packageDirectory 'vault.exe')
Copy-Item -LiteralPath (Join-Path $projectRoot 'docs\PORTABLE-README.txt') -Destination (Join-Path $packageDirectory 'README.txt')
Copy-Item -LiteralPath (Join-Path $projectRoot 'LICENSE') -Destination (Join-Path $packageDirectory 'LICENSE.txt')
Copy-Item -LiteralPath (Join-Path $projectRoot 'NOTICE') -Destination (Join-Path $packageDirectory 'NOTICE.txt')
Copy-Item -LiteralPath (Join-Path $projectRoot 'THIRD_PARTY_NOTICES.txt') -Destination $packageDirectory
Copy-Item -LiteralPath (Join-Path $projectRoot 'docs\third-party-sources') -Destination (Join-Path $packageDirectory 'third-party-sources') -Recurse
$zipPath = Join-Path $artifactRoot "$packageName.zip"
if (Test-Path -LiteralPath $zipPath) { throw 'ZIP already exists. Move it aside before repackaging.' }
Compress-Archive -LiteralPath $packageDirectory -DestinationPath $zipPath -CompressionLevel Optimal
$files = @(Get-ChildItem -LiteralPath $packageDirectory -File -Recurse)
$report = [ordered]@{
    version = $version
    platform = 'windows-x64'
    measuredAt = (Get-Date).ToUniversalTime().ToString('o')
    buildScope = $(if ($version -match 'p0') { 'P0 engineering baseline' } else { 'Desktop + SQLite + AES-256-GCM + user DPAPI + MCP + tray + optional startup' })
    bytesPerMB = 1000000
    bytesPerMiB = 1048576
    executableBytes = (Get-Item -LiteralPath (Join-Path $packageDirectory 'vault.exe')).Length
    extractedTotalBytes = ($files | Measure-Object -Property Length -Sum).Sum
    zipBytes = (Get-Item -LiteralPath $zipPath).Length
    zipSha256 = Get-Sha256 $zipPath
    files = @($files | ForEach-Object { [ordered]@{ name = $_.Name; bytes = $_.Length; sha256 = Get-Sha256 $_.FullName } })
}
$reportName = if ($version -match 'p0') { 'p0-size.json' } else { 'size.json' }
$report | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $artifactRoot $reportName) -Encoding UTF8
$report | ConvertTo-Json -Depth 6
