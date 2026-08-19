# Uploads the private CA files to a Discord channel via webhook.
#
# The webhook URL is a secret - provide it either via:
#   $env:HANA_DISCORD_WEBHOOK
#   or setup\ca\discord-webhook.txt   (git-ignored, one line)
#
# Uploads (to the channel the webhook belongs to):
#   rootCA.crt        - public, install on target machines
#   codesign.cnf      - config for re-issuing
#   CA_Setup_Guide.md - install instructions
#   codesign.pfx      - SENSITIVE: signs the software; keep the channel locked
#
# Run:  powershell -ExecutionPolicy Bypass -File tools\upload-ca-discord.ps1
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$caDir = Join-Path $root "setup\ca"
$webhookFile = Join-Path $caDir "discord-webhook.txt"

$webhook = $env:HANA_DISCORD_WEBHOOK
if (-not $webhook -and (Test-Path $webhookFile)) {
    $webhook = (Get-Content $webhookFile -Raw).Trim()
}
if (-not $webhook) {
    throw "Webhook URL tidak ditemukan. Set env HANA_DISCORD_WEBHOOK atau buat setup\ca\discord-webhook.txt"
}

$curl = (Get-Command curl.exe).Source
if (-not $curl) { throw "curl.exe tidak tersedia" }

$files = @(
    (Join-Path $caDir "rootCA.crt"),
    (Join-Path $caDir "codesign.cnf"),
    (Join-Path $root "setup\CA_Setup_Guide.md"),
    (Join-Path $caDir "codesign.pfx")
)
foreach ($f in $files) {
    if (-not (Test-Path $f)) { throw "File tidak ada: $f" }
}

Write-Host "Mengunggah CA files ke Discord (private channel)..."
# Note: use a plain "content=" form field - payload_json gets mangled by
# PowerShell -> curl.exe native argument quoting.
$content = "Hana Launcher CA - HanaLauncher - " + (Get-Date -Format 'yyyy-MM-dd HH:mm')
$args = @("-s", "-f", "-i", "-X", "POST", $webhook,
    "-F", "content=$content")
for ($i = 0; $i -lt $files.Count; $i++) {
    $args += "-F"; $args += ("file{0}=@{1}" -f ($i + 1), $files[$i])
}
& $curl @args
if ($LASTEXITCODE -ne 0) { throw "Upload Discord gagal (exit $LASTEXITCODE)" }
Write-Host ""
Write-Host "Upload selesai. Cek channel Discord Anda."
Write-Host "PERINGATAN: codesign.pfx berisi kunci privat - pastikan channel terkunci rapat & aktifkan 2FA."