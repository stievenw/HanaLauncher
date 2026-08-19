# One-shot full build for Hana Launcher.
#   - bootstraps the private CA + code-signing cert if missing (make-private-ca)
#   - runs the full setup pipeline (cargo release build, asset packing, WiX
#     MSI + Burn bundle, Authenticode signing, checksums) via build-setup.ps1
#   - optional: installs rootCA.crt on THIS machine so its own builds verify
#
# Run from the project root:
#   powershell -ExecutionPolicy Bypass -File .\build-all.ps1
#   powershell -ExecutionPolicy Bypass -File .\build-all.ps1 -InstallCaToThisPc
param(
    [switch]$InstallCaToThisPc,   # add rootCA.crt to the current user's Trusted Root store
    [switch]$SkipCa               # skip the CA bootstrap check
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$caDir = Join-Path $root "setup\ca"
$rootCrt = Join-Path $caDir "rootCA.crt"

Write-Host "================ HANA LAUNCHER - ONE-SHOT BUILD ================"
Write-Host ""

if (-not $SkipCa) {
    if (-not (Test-Path (Join-Path $caDir "codesign.pfx"))) {
        Write-Host "[1/1] CA belum ada - bootstrap root CA + codesign cert..."
        & powershell -ExecutionPolicy Bypass -File (Join-Path $root "setup\make-private-ca.ps1")
    } else {
        Write-Host "[skip] Private CA sudah ada (setup\ca\)."
    }

    if ($InstallCaToThisPc) {
        Write-Host "Install rootCA.crt ke Trusted Root (current user)..."
        & certutil -user -addstore Root $rootCrt | Out-Null
    }
} else {
    Write-Host "[skip] CA bootstrap dilewati (-SkipCa)."
}

Write-Host ""
Write-Host "Jalankan build setup lengkap (build -> assets -> MSI+bundle -> sign -> hash)..."
& powershell -ExecutionPolicy Bypass -File (Join-Path $root "setup\build-setup.ps1")
if ($LASTEXITCODE -ne 0) { throw "build-setup.ps1 gagal" }

Write-Host ""
Write-Host "================ VERIFIKASI TANDA TANGAN ================"
$targets = @(
    (Join-Path $root "target\release\HanaLauncher.exe"),
    (Join-Path $root "setup\HanaLauncher.msi"),
    (Join-Path $root "setup\HanaLauncherSetup.exe")
)
foreach ($t in $targets) {
    $s = Get-AuthenticodeSignature $t
    Write-Host ("  {0,-28} -> {1}  ({2})" -f (Split-Path $t -Leaf), $s.Status, $s.SignerCertificate.Subject)
}

Write-Host ""
Write-Host "================ HASH (setup\HanaLauncher-hashes.txt) ================"
Get-Content (Join-Path $root "setup\HanaLauncher-hashes.txt")
Write-Host ""
Write-Host "SELESAI. Artifak:"
Write-Host "  setup\HanaLauncherSetup.exe  - installer"
Write-Host "  setup\HanaLauncher.msi        - MSI"
Write-Host "  target\release\HanaLauncher.exe"
Write-Host "  resources\                    - aset UI + font"