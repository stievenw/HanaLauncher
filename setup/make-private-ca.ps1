# Generates a private Root CA + code-signing certificate chain with OpenSSL
# (the "online CA via Discord" distribution idea):
#   setup\ca\rootCA.crt      - PUBLIC. Install on target Windows machines
#                              (Trusted Root) to make the publisher trusted.
#   setup\ca\codesign.pfx    - SENSITIVE. Used by SignTool to sign the exe/msi.
#   setup\ca\codesign.cnf    - config, kept for re-issuing later.
#   setup\ca\codesign.key    - NEVER upload. Stays local.
#   setup\ca\rootCA.key      - NEVER upload. Stays local (passphrase protected).
#
# NOTE: this only makes the publisher "trusted" on machines where rootCA.crt is
# manually installed. It does NOT grant global trust, and SmartScreen still
# warns for new publishers. Intended for internal / controlled machines.
#
# Requires OpenSSL on PATH or $env:OPENSSL_BIN.
#
# Run:
#   powershell -ExecutionPolicy Bypass -File setup\make-private-ca.ps1
$ErrorActionPreference = "Stop"

$openssl = "C:\Program Files\OpenSSL-Win64\bin\openssl.exe"
if (-not (Test-Path $openssl)) { throw "openssl.exe not found" }

$caDir = Join-Path $PSScriptRoot "ca"
$secretFile = Join-Path $caDir "secrets.txt"
New-Item -ItemType Directory -Force -Path $caDir | Out-Null

# --- passwords / passphrases (kept local only) ---------------------------
if (Test-Path $secretFile) {
    $rootPass = (Select-String -Path $secretFile -Pattern "^ROOT_PASS=(.*)$").Matches[0].Groups[1].Value
    $pfxPass  = (Select-String -Path $secretFile -Pattern "^PFX_PASS=(.*)$").Matches[0].Groups[1].Value
} else {
    $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    function New-Rand([int]$len) {
        $b = New-Object byte[] $len; $rng.GetBytes($b)
        -join ($b | ForEach-Object { "{0:x2}" -f $_ })
    }
    $rootPass = New-Rand 24
    $pfxPass  = New-Rand 24
    Set-Content -Path $secretFile -Value ("ROOT_PASS={0}`nPFX_PASS={1}" -f $rootPass, $pfxPass) -NoNewline
}

$rootKey = Join-Path $caDir "rootCA.key"
$rootCrt = Join-Path $caDir "rootCA.crt"
$csrKey  = Join-Path $caDir "codesign.key"
$csr     = Join-Path $caDir "codesign.csr"
$csrCrt  = Join-Path $caDir "codesign.crt"
$pfx     = Join-Path $caDir "codesign.pfx"
$cnf     = Join-Path $caDir "codesign.cnf"

$rootSubj = "/C=ID/O=Hanakama/CN=HanaLauncher CA/E=oeikoko@gmail.com"
$csrSubj  = "/C=ID/O=Hanakama/CN=StievenW/E=oeikoko@gmail.com"

if (-not (Test-Path $rootKey)) {
    Write-Host "== 1/6 Root CA private key (4096, AES-256) =="
    & $openssl genrsa -aes256 -passout pass:$rootPass -out $rootKey 4096
    if ($LASTEXITCODE -ne 0) { throw "genrsa root failed" }
}

if (-not (Test-Path $rootCrt)) {
    Write-Host "== 2/6 Root CA certificate (10 years) =="
    & $openssl req -x509 -new -sha256 -days 3650 -key $rootKey -passin pass:$rootPass `
        -out $rootCrt -subj $rootSubj
    if ($LASTEXITCODE -ne 0) { throw "root cert failed" }
}

Write-Host "== 3/6 Code-signing key (2048) =="
& $openssl genrsa -out $csrKey 2048
if ($LASTEXITCODE -ne 0) { throw "genrsa codesign failed" }

Write-Host "== 4/6 CSR for code-signing cert =="
& $openssl req -new -key $csrKey -out $csr -subj $csrSubj
if ($LASTEXITCODE -ne 0) { throw "csr failed" }

Set-Content -Path $cnf -Value @"
[ v3_ext ]
authorityKeyIdentifier=keyid,issuer
subjectKeyIdentifier=hash
basicConstraints=CA:FALSE
keyUsage = digitalSignature, nonRepudiation, keyEncipherment, dataEncipherment
extendedKeyUsage = codeSigning
"@

Write-Host "== 5/6 Issue code-signing cert from Root CA (730 days) =="
& $openssl x509 -req -in $csr -CA $rootCrt -CAkey $rootKey -CAcreateserial `
    -out $csrCrt -days 730 -sha256 -extfile $cnf -extensions v3_ext -passin pass:$rootPass
if ($LASTEXITCODE -ne 0) { throw "codesign cert failed" }

Write-Host "== 6/6 Export codesign.pfx (password-protected) =="
& $openssl pkcs12 -export -out $pfx -inkey $csrKey -in $csrCrt -certfile $rootCrt -passout pass:$pfxPass
if ($LASTEXITCODE -ne 0) { throw "pkcs12 failed" }

Write-Host ""
Write-Host "Verify chain:"
& $openssl verify -CAfile $rootCrt $csrCrt
Write-Host ""
Write-Host "Root CA     : $rootCrt"
Write-Host "codesign.pfx: $pfx"
Write-Host "Secrets     : $secretFile  (keep local, never upload)"