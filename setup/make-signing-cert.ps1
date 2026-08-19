# Generates the self-signed Authenticode code-signing certificate for
# "StievenW <oeikoko@gmail.com>" and exports it to setup\signing\StievenW.pfx.
#
# Run:
#   powershell -ExecutionPolicy Bypass -File setup\make-signing-cert.ps1
#
# The certificate uses the LONGEST validity signtool accepts. Tested on
# signtool 10.0.26100: "No certificates were found" appears beyond
# ~355900 days, so we use 355000 days (~972 years, NotAfter year ~2998).
# Combined with timestamping during signing, the signature stays valid forever.
#
# NOTE: a self-signed certificate embeds the publisher identity (StievenW) but
# Windows/SmartScreen still shows "Unknown Publisher" because no trusted CA
# vouches for it. It is meant for personal distribution; for a "verified"
# green publisher you need a certificate from a CA (buy or Azure Trusted
# Signing). Keep pfx-password.txt secret - anyone holding both files can sign
# as StievenW.
$ErrorActionPreference = "Stop"

$signDir = Join-Path $PSScriptRoot "signing"
New-Item -ItemType Directory -Force -Path $signDir | Out-Null

$subject = "CN=StievenW, E=oeikoko@gmail.com"
# Longest validity signtool accepts (safe margin below the ~355880-day limit).
$maxDays = 355000
$notAfter = (Get-Date).AddDays($maxDays)

$cert = Get-ChildItem "Cert:\CurrentUser\My" |
    Where-Object { $_.Subject -like "*StievenW*" -and $_.HasPrivateKey } |
    Select-Object -First 1
if ($cert -and ($cert.NotAfter -lt $notAfter.AddDays(-365) -or $cert.NotAfter -gt $notAfter.AddDays(365))) {
    # Existing certificate does not have the maximum working validity ->
    # recreate it.
    Remove-Item "Cert:\CurrentUser\My\$($cert.Thumbprint)" -Force
    $cert = $null
}
if (-not $cert) {
    $cert = New-SelfSignedCertificate `
        -Type CodeSigningCert `
        -Subject $subject `
        -CertStoreLocation "Cert:\CurrentUser\My" `
        -NotAfter $notAfter `
        -KeyUsage DigitalSignature `
        -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3")
}

$chars = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#%"
$rand = [System.Random]::new()
$pwd = -join (1..24 | ForEach-Object { $chars[$rand.Next($chars.Length)] })

$secure = [System.Security.SecureString]::new()
$pwd.ToCharArray() | ForEach-Object { $secure.AppendChar($_) }

$pfxPath = Join-Path $signDir "StievenW.pfx"
Export-PfxCertificate -Cert $cert -FilePath $pfxPath -Password $secure -Force | Out-Null
Set-Content -Path (Join-Path $signDir "pfx-password.txt") -Value $pwd -NoNewline

Write-Host "OK: $pfxPath"
Write-Host "Subject: $($cert.Subject)"
Write-Host "Thumbprint: $($cert.Thumbprint)"
Write-Host "Valid until: $($cert.NotAfter)"