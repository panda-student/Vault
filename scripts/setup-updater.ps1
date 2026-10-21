param([switch]$UploadSecret)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Security
$root = Split-Path -Parent $PSScriptRoot
$keyDirectory = Join-Path $env:USERPROFILE '.codex\secrets\local-password-manager'
$protectedPath = Join-Path $keyDirectory 'update-signing-key.dpapi'
$publicPath = Join-Path $root 'src-tauri\update-public-key.txt'
New-Item -ItemType Directory -Path $keyDirectory -Force | Out-Null
if (!(Test-Path -LiteralPath $protectedPath)) {
    if (Test-Path -LiteralPath $publicPath) { throw 'Public key already exists. Restore its private key rather than generating a different key.' }
    $temporary = Join-Path $keyDirectory ([guid]::NewGuid().ToString() + '.key')
    try {
        & node (Join-Path $PSScriptRoot 'update-release.mjs') keygen $publicPath $temporary
        if ($LASTEXITCODE -ne 0) { throw 'Key generation failed' }
        $plain = [IO.File]::ReadAllBytes($temporary)
        $encrypted = [Security.Cryptography.ProtectedData]::Protect($plain, $null, [Security.Cryptography.DataProtectionScope]::CurrentUser)
        [IO.File]::WriteAllBytes($protectedPath, $encrypted)
    } finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
        if ($plain) { [Array]::Clear($plain, 0, $plain.Length) }
    }
}
if ($UploadSecret) {
    $plain = [Security.Cryptography.ProtectedData]::Unprotect([IO.File]::ReadAllBytes($protectedPath), $null, [Security.Cryptography.DataProtectionScope]::CurrentUser)
    try {
        [Text.Encoding]::UTF8.GetString($plain) | & gh secret set VAULT_UPDATE_SIGNING_KEY --repo panda-student/local-password-manager
        if ($LASTEXITCODE -ne 0) { throw 'Failed to upload signing secret' }
    } finally { [Array]::Clear($plain, 0, $plain.Length) }
}
Write-Output 'Updater public key is ready; private backup is protected with current-user DPAPI outside the repository.'
