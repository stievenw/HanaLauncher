# Generates setup\HanaLauncher-hashes.txt with the SHA-256 of the shipped
# files only (setup EXE, MSI, launcher EXE).
#
# Run (also invoked automatically at the end of setup\build-setup.ps1):
#   powershell -ExecutionPolicy Bypass -File tools\make-hashes.ps1
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$setupDir = Join-Path $root "setup"
$release = Join-Path $root "target\release\HanaLauncher.exe"
$outFile = Join-Path $setupDir "HanaLauncher-hashes.txt"

$entries = @(
    @{ Path = Join-Path $setupDir "HanaLauncherSetup.exe"; Name = "HanaLauncherSetup.exe" },
    @{ Path = Join-Path $setupDir "HanaLauncher.msi";      Name = "HanaLauncher.msi" },
    @{ Path = $release;                                    Name = "HanaLauncher.exe" }
)

foreach ($e in $entries) {
    if (-not (Test-Path $e.Path)) { throw "File tidak ada: $($e.Path)" }
}

$lines = foreach ($e in $entries) {
    $sha256 = (Get-FileHash $e.Path -Algorithm SHA256).Hash
    "{0}  {1}" -f $e.Name, $sha256
}

Set-Content -Path $outFile -Value $lines -Encoding UTF8
Write-Host "OK: $outFile"