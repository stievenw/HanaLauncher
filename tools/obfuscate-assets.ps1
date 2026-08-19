# Packs UI assets + Monogram fonts into XOR-obfuscated blobs stored in the
# external `resources/` folder next to the launcher exe (see src/assets.rs).
# Run from the project root:
#
#   powershell -ExecutionPolicy Bypass -File tools\obfuscate-assets.ps1
#
# The original files in assets/ and monogram/ are kept for editing only; the
# launcher only reads the obfuscated copies from resources/, so the artwork is
# not trivially extractable (hex editor / resource dumpers). The setup bundles
# these files (setup/HanaLauncher.wxs).
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$assets = Join-Path $root "assets"
$fonts = Join-Path $root "monogram\ttf"
$resources = Join-Path $root "resources"
New-Item -ItemType Directory -Force -Path $resources | Out-Null

# Repeating XOR key. Must match the KEY in src/assets.rs.
$key = @(0x5A, 0x3C, 0xA7)

$jobs = @(
    @{ Src = Join-Path $assets "icon.png";         Out = "icon.png.x" },
    @{ Src = Join-Path $assets "background.jpg";   Out = "bg.jpg.x" },
    @{ Src = Join-Path $fonts "monogram-extended.ttf";        Out = "monogram.ttf.x" },
    @{ Src = Join-Path $fonts "monogram-extended-italic.ttf"; Out = "monogram-italic.ttf.x" }
)

foreach ($j in $jobs) {
    $bytes = [System.IO.File]::ReadAllBytes($j.Src)
    for ($i = 0; $i -lt $bytes.Length; $i++) {
        $bytes[$i] = $bytes[$i] -bxor $key[$i % $key.Length]
    }
    $out = Join-Path $resources $j.Out
    [System.IO.File]::WriteAllBytes($out, $bytes)
    $s = [math]::Round((Get-Item $j.Src).Length / 1KB, 0)
    $o = [math]::Round((Get-Item $out).Length / 1KB, 0)
    Write-Host ("packed {0} -> resources\{1} ({2} KB -> {3} KB)" -f (Split-Path $j.Src -Leaf), $j.Out, $s, $o)
}