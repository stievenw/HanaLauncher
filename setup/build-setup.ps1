# Builds the Hana Launcher setup EXE:
#   1. cargo build --release
#   2. repacks the obfuscated UI assets + fonts into resources/
#   3. compiles the WiX MSI (HanaLauncher.wxs) -> HanaLauncher.msi
#   4. compiles the Burn bundle (HanaLauncherBundle.wxs) -> HanaLauncherSetup.exe
#   5. (optional) Authenticode-signs the exe + MSI + setup (uses
#      setup\signing\StievenW.pfx automatically, or $env:CODE_SIGN_THUMBPRINT)
#   6. writes setup\HanaLauncher-hashes.txt (SHA256/SHA1 of every shipped file)
#
# Uses the WiX 3.14 toolset bundled in the "Avatar Online TeaMobi" project
# (candle.exe + light.exe). Override with:  $env:WIX_BIN = "C:\...\bin"
#
# Run from the project root:
#   powershell -ExecutionPolicy Bypass -File setup\build-setup.ps1
# NOTE: EAP must stay "Continue", not "Stop": under PowerShell 5.1 a native
# command writing to stderr (cargo prints "Compiling ..." there) becomes a
# terminating RemoteException with EAP=Stop, aborting the whole build.
# Failures are detected via the explicit $LASTEXITCODE checks below instead.
$ErrorActionPreference = "Continue"

$root = Split-Path -Parent $PSScriptRoot
$release = Join-Path $root "target\release\HanaLauncher.exe"
$setupDir = $PSScriptRoot

if (Get-Process -Name "HanaLauncher", "hana_launcher" -ErrorAction SilentlyContinue) {
    throw "HanaLauncher is still running - close it before building the setup."
}

Write-Host "== 1/6 Release build =="
Push-Location $root
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build --release failed" }
Pop-Location

Write-Host "== 2/6 Packing UI assets + fonts into resources/ =="
& powershell -ExecutionPolicy Bypass -File (Join-Path $root "tools\obfuscate-assets.ps1")

Write-Host "== 3/6 Finding WiX 3.14 toolset =="
$wixCandidates = @(
    "D:\Project\Avatar Online TeaMobi\tools-wix\wix314",
    "C:\Program Files (x86)\WiX Toolset v3.14\bin"
)
if ($env:WIX_BIN) { $wixCandidates = @($env:WIX_BIN) + $wixCandidates }
$wixBin = $wixCandidates | Where-Object { Test-Path (Join-Path $_ "candle.exe") } | Select-Object -First 1
if (-not $wixBin) { throw "WiX toolset not found (candle.exe). Set WIX_BIN to the tools-wix\wix314 folder." }
Write-Host "WiX: $wixBin"
$env:PATH = "$wixBin;$env:PATH"

Push-Location $setupDir

Write-Host "== 4/6 Compiling MSI + Bundle =="
& candle.exe HanaLauncher.wxs -arch x64 -ext WixUIExtension.dll -out HanaLauncher.wixobj
if ($LASTEXITCODE -ne 0) { throw "candle (MSI) failed" }
& light.exe HanaLauncher.wixobj -ext WixUIExtension.dll -spdb -out HanaLauncher.msi
if ($LASTEXITCODE -ne 0) { throw "light (MSI) failed" }

& candle.exe HanaLauncherBundle.wxs -arch x64 -ext WixBalExtension.dll -out HanaLauncherBundle.wixobj
if ($LASTEXITCODE -ne 0) { throw "candle (bundle) failed" }
& light.exe HanaLauncherBundle.wixobj -ext WixBalExtension.dll -spdb -out HanaLauncherSetup.exe
if ($LASTEXITCODE -ne 0) { throw "light (bundle) failed" }

# Optional Authenticode signing. Priority:
#   1. setup\ca\codesign.pfx   - code-signing cert issued by the private
#      HanaLauncher CA (make-private-ca.ps1). The publisher is only "trusted"
#      on machines where setup\ca\rootCA.crt is installed into the Trusted Root
#      store (see CA_Setup_Guide.md). Used automatically when present.
#   2. setup\signing\StievenW.pfx - self-signed (make-signing-cert.ps1);
#      publisher shows as "Unknown Publisher" (no trusted CA).
#   3. $env:CODE_SIGN_THUMBPRINT - sign with a cert in the certificate store.
#
# IMPORTANT: a Burn bundle must NOT be signed with signtool directly - that
# corrupts the attached container ("Failed to extract all files from
# container, erf: 1:2:0"). The correct flow is insignia: extract the engine,
# sign the engine, re-attach it, then sign the outer bundle.
$setup = Join-Path $setupDir "HanaLauncherSetup.exe"
$msi = Join-Path $setupDir "HanaLauncher.msi"
$caPfx = Join-Path $setupDir "ca\codesign.pfx"
$caSecrets = Join-Path $setupDir "ca\secrets.txt"
$selfPfx = Join-Path $setupDir "signing\StievenW.pfx"
$selfPwFile = Join-Path $setupDir "signing\pfx-password.txt"
$pfx = $null
$pwFile = $null
if ((Test-Path $caPfx) -and (Test-Path $caSecrets)) {
    $pfx = $caPfx
    $pwFile = $caSecrets
    Write-Host "Signing with private CA cert (codesign.pfx)"
} elseif ((Test-Path $selfPfx) -and (Test-Path $selfPwFile)) {
    $pfx = $selfPfx
    $pwFile = $selfPwFile
    Write-Host "Signing with self-signed cert (StievenW.pfx)"
}
$usePfx = $null -ne $pfx

if ($usePfx -or $env:CODE_SIGN_THUMBPRINT) {
    Write-Host "== 5/6 Signing (Authenticode) =="
    $signtool = Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin" `
        -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
        Sort-Object FullName -Descending | Select-Object -First 1
    if (-not $signtool) { throw "signtool.exe not found (Windows SDK)" }

    # Real-time AV briefly locks freshly written files (e.g. the extracted
    # Burn engine). Wait until the file is no longer held before signing.
    function Wait-FileUnlocked([string]$path, [int]$maxSeconds = 30) {
        for ($i = 0; $i -lt $maxSeconds; $i++) {
            try {
                $fs = [System.IO.File]::Open($path, 'Open', 'ReadWrite', 'None')
                $fs.Close()
                return
            } catch {
                Start-Sleep -Milliseconds 500
            }
        }
        throw "File masih terkunci oleh proses lain: $path"
    }

    function Sign-File([string]$path) {
        Wait-FileUnlocked $path
        if ($usePfx) {
            $pw = if ($pfx -eq $caPfx) {
                (Select-String -Path $pwFile -Pattern "^PFX_PASS=(.*)$").Matches[0].Groups[1].Value
            } else {
                (Get-Content $pwFile -Raw).Trim()
            }
            & $signtool.FullName sign /f $pfx /p $pw /fd SHA256 `
                /tr "http://timestamp.digicert.com" /td SHA256 /v $path
        } else {
            & $signtool.FullName sign /sha1 $env:CODE_SIGN_THUMBPRINT /fd SHA256 `
                /tr "http://timestamp.digicert.com" /td SHA256 /v $path
        }
        if ($LASTEXITCODE -ne 0) { throw "signing failed: $path" }
    }

    # The plain launcher exe and the MSI are signed directly.
    Sign-File $release
    Sign-File $msi

    # The Burn bundle needs the insignia two-step (see comment above).
    $engineTmp = Join-Path $setupDir "engine.exe"
    $bundleTmp = Join-Path $setupDir "HanaLauncherSetup.new.exe"
    & insignia.exe -ib $setup -o $engineTmp
    if ($LASTEXITCODE -ne 0) { throw "insignia extract engine failed" }
    Sign-File $engineTmp
    & insignia.exe -ab $engineTmp $setup -o $bundleTmp
    if ($LASTEXITCODE -ne 0) { throw "insignia re-attach failed" }
    Remove-Item $engineTmp -Force
    Remove-Item $setup -Force
    Move-Item $bundleTmp $setup -Force
    Sign-File $setup
}

Pop-Location

Write-Host "== 6/6 Writing checksums =="
& powershell -ExecutionPolicy Bypass -File (Join-Path $root "tools\make-hashes.ps1")

$setup = Join-Path $setupDir "HanaLauncherSetup.exe"
$msi = Join-Path $setupDir "HanaLauncher.msi"
$exe = Get-Item $release
$resDir = Join-Path $root "resources"
$resFiles = @(Get-ChildItem $resDir -File)
$resBytes = ($resFiles | ForEach-Object { $_.Length } | Measure-Object -Sum).Sum
Write-Host ""
Write-Host ("OK: {0}  ({1} MB)" -f $setup, [math]::Round((Get-Item $setup).Length / 1MB, 1))
Write-Host ("    {0}  ({1} MB)" -f $msi, [math]::Round((Get-Item $msi).Length / 1MB, 1))
Write-Host ("    HanaLauncher.exe ({0} MB)" -f [math]::Round($exe.Length / 1MB, 1))
Write-Host ("    resources/     ({0} KB, {1} files)" -f [math]::Round($resBytes / 1KB, 0), $resFiles.Count)