# HanaLauncher - scraper katalog client mod (Fabric/Quilt/Forge)
# Menghasilkan client-mod-catalog.json yang dipakai launcher untuk daftar
# semua versi loader per versi Minecraft (hanya rilis stabil, karena UI
# launcher memilih dari daftar release).
# Memakai curl.exe (native) + proses paralel supaya cepat.
param(
    [string]$Out = "E:\Project\Minecraft\Software\HanaLauncher-portal\client-mod-catalog.json",
    [int]$Parallel = 12
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$tmp = Join-Path $env:TEMP "hana-catalog"
if (-not (Test-Path $tmp)) { New-Item -ItemType Directory -Path $tmp -Force | Out-Null }
Get-ChildItem $tmp -ErrorAction SilentlyContinue | Remove-Item -Force

function Get-Curl([string]$Url, [string]$OutFile) {
    & curl.exe -s --max-time 40 -o $OutFile $Url 2>$null
}

function Get-GameList([string]$Url) {
    $f = Join-Path $tmp ([guid]::NewGuid().ToString("N") + ".json")
    Get-Curl $Url $f
    $list = @()
    if (Test-Path $f) {
        try { $list = Get-Content $f -Raw | ConvertFrom-Json } catch { }
    }
    $list
}

function Scrape-Loaders([string]$Family, [string]$GamesUrl) {
    $games = Get-GameList $GamesUrl | Where-Object { $_.stable -eq $true }
    Write-Host "${Family}: $($games.Count) versi game stabil"
    $jobs = @()
    $byGame = @{}
    foreach ($g in $games) {
        $mc = $g.version
        $safe = ($mc -replace '[^0-9A-Za-z.]', '_')
        $f = Join-Path $tmp "$Family-$safe.json"
        $byGame[$mc] = $f
        if ($Family -eq "fabric") {
            $url = "https://meta.fabricmc.net/v2/versions/loader/$mc"
        } else {
            $url = "https://meta.quiltmc.org/v3/versions/loader/$mc"
        }
        $jobs += Start-Process -FilePath "curl.exe" -ArgumentList "-s","--max-time","40","-o",$f,$url -WindowStyle Hidden -PassThru
        if ($jobs.Count -ge $Parallel) {
            $jobs | Wait-Process
            $jobs = @()
        }
    }
    $jobs | Wait-Process
    $loaders = @{}
    foreach ($mc in $byGame.Keys) {
        $f = $byGame[$mc]
        if (-not (Test-Path $f)) { continue }
        $list = @()
        try {
            $entries = Get-Content $f -Raw | ConvertFrom-Json
            foreach ($e in $entries) {
                if ($e.loader.version) { $list += $e.loader.version }
            }
        } catch { }
        if ($list.Count -gt 0) {
            $loaders[$mc] = @($list | Sort-Object -Unique)
        }
    }
    $loaders
}

Write-Host "== Fabric =="
$fabric = Scrape-Loaders "fabric" "https://meta.fabricmc.net/v2/versions/game"
$fabricCount = ($fabric.Values | ForEach-Object { $_.Count } | Measure-Object -Sum).Sum
Write-Host "fabric total: $fabricCount entri"

Write-Host "== Quilt =="
$quilt = Scrape-Loaders "quilt" "https://meta.quiltmc.org/v3/versions/game"
$quiltCount = ($quilt.Values | ForEach-Object { $_.Count } | Measure-Object -Sum).Sum
Write-Host "quilt total: $quiltCount entri"

Write-Host "== Forge =="
$forgeMeta = Join-Path $tmp "forge.xml"
Get-Curl "https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml" $forgeMeta
$forgeXml = Get-Content $forgeMeta -Raw
$forgeVersions = [regex]::Matches($forgeXml, "<version>([^<]+)</version>") | ForEach-Object { $_.Groups[1].Value }
$forgeLoaders = @{}
foreach ($v in $forgeVersions) {
    $idx = $v.LastIndexOf("-")
    if ($idx -le 0) { continue }
    $mc = $v.Substring(0, $idx)
    $build = $v.Substring($idx + 1)
    if (-not $forgeLoaders.ContainsKey($mc)) { $forgeLoaders[$mc] = @() }
    $forgeLoaders[$mc] += $build
}
foreach ($k in @($forgeLoaders.Keys)) {
    $forgeLoaders[$k] = @($forgeLoaders[$k] | Sort-Object -Unique)
}
$forgeCount = ($forgeLoaders.Values | ForEach-Object { $_.Count } | Measure-Object -Sum).Sum
Write-Host "forge total: $forgeCount entri ($($forgeLoaders.Keys.Count) versi game)"

$catalog = [ordered]@{
    schema  = 1
    updated = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    sources = [ordered]@{
        fabric = "https://meta.fabricmc.net/v2"
        quilt  = "https://meta.quiltmc.org/v3"
        forge  = "https://maven.minecraftforge.net"
    }
    versions = [ordered]@{
        fabric = [ordered]@{ loaders = $fabric }
        quilt  = [ordered]@{ loaders = $quilt }
        forge  = [ordered]@{ loaders = $forgeLoaders }
    }
}

$json = $catalog | ConvertTo-Json -Depth 8 -Compress
$dir = Split-Path -Parent $Out
if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
[System.IO.File]::WriteAllText($Out, $json, (New-Object System.Text.UTF8Encoding($false)))
Get-ChildItem $tmp -ErrorAction SilentlyContinue | Remove-Item -Force
Write-Host "Ditulis: $Out ($((Get-Item $Out).Length) bytes)"