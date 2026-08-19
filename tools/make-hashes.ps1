# Generates setup\HanaLauncher-hashes.txt with SHA256 + SHA1 + size of every
# shipped file (setup EXE, MSI, launcher EXE, resources), so users can verify
# they have the correct, untampered files before installing.
#
# Run (also invoked automatically at the end of setup\build-setup.ps1):
#   powershell -ExecutionPolicy Bypass -File tools\make-hashes.ps1
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$setupDir = Join-Path $root "setup"
$release = Join-Path $root "target\release\HanaLauncher.exe"
$resources = Join-Path $root "resources"
$outFile = Join-Path $setupDir "HanaLauncher-hashes.txt"

$entries = @(
    @{ Path = Join-Path $setupDir "HanaLauncherSetup.exe"; Name = "HanaLauncherSetup.exe" },
    @{ Path = Join-Path $setupDir "HanaLauncher.msi";      Name = "HanaLauncher.msi" },
    @{ Path = $release;                                    Name = "HanaLauncher.exe" },
    @{ Path = Join-Path $resources "icon.png.x";           Name = "resources\icon.png.x" },
    @{ Path = Join-Path $resources "bg.jpg.x";             Name = "resources\bg.jpg.x" },
    @{ Path = Join-Path $resources "monogram.ttf.x";       Name = "resources\monogram.ttf.x" },
    @{ Path = Join-Path $resources "monogram-italic.ttf.x";Name = "resources\monogram-italic.ttf.x" }
)

foreach ($e in $entries) {
    if (-not (Test-Path $e.Path)) { throw "File tidak ada: $($e.Path)" }
}

$lines = New-Object System.Collections.Generic.List[string]
$lines.Add("# Hana Launcher - SHA256 / SHA1 checksums")
$lines.Add("# Dibuat: $(Get-Date -Format "yyyy-MM-dd HH:mm:ss")")
$lines.Add("#")
$lines.Add("# Cara verifikasi di Windows (PowerShell):")
$lines.Add("#   Get-FileHash HanaLauncherSetup.exe -Algorithm SHA256")
$lines.Add("# Bandingkan hasilnya dengan baris SHA256 dari file yang sama di bawah.")
$lines.Add("# Nilai harus IDENTIK. Jika berbeda, file rusak / bukan file resmi.")
$lines.Add("")
foreach ($e in $entries) {
    $sha256 = (Get-FileHash $e.Path -Algorithm SHA256).Hash
    $sha1 = (Get-FileHash $e.Path -Algorithm SHA1).Hash
    $size = (Get-Item $e.Path).Length
    $lines.Add("FILE   $($e.Name)")
    $lines.Add("SHA256 $($e.Name) = $sha256")
    $lines.Add("SHA1   $($e.Name) = $sha1")
    $lines.Add("SIZE   $($e.Name) = $size bytes")
    $lines.Add("")
}

Set-Content -Path $outFile -Value $lines.ToArray() -Encoding UTF8
Write-Host "OK: $outFile"