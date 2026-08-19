# CA Setup Guide — Hana Launcher

To make Windows show the signed Hana Launcher files as a **valid, trusted
publisher** (instead of "Unknown Publisher"), install the private Root CA on
each machine once.

> The launcher is signed with a certificate issued by our private CA
> (`setup\ca\codesign.pfx`). Windows only trusts it if `rootCA.crt` is
> installed in the machine's **Trusted Root Certification Authorities** store.

## Distribution

Files are distributed through the private Discord channel (`#hana-ca`):

| File | Security | What it is |
|---|---|---|
| `rootCA.crt` | Public (team) | The Root CA. Must be installed on target machines. |
| `codesign.pfx` | Sensitive | Used by the build/signing pipeline. Never give to end users. |
| `codesign.cnf` | Public (team) | OpenSSL config for re-issuing signing certs. |
| `CA_Setup_Guide.md` | Public (team) | This file. |

**Never upload** `rootCA.key`, `codesign.key`, or `secrets.txt` to Discord.

## Install on a target machine (requires admin)

### Option A — certmgr.msc (graphical)
1. Open `certmgr.msc` → **Trusted Root Certification Authorities** → Certificates.
2. Right-click → **All Tasks → Import…** → Next.
3. Browse to `rootCA.crt` → Next → "Place all certificates in the following
   store" = **Trusted Root Certification Authorities** → Next → Finish.
4. Confirm the security warning ("You are about to install a certificate from
   a certification authority (CA) claiming to represent…").

### Option B — PowerShell (admin)
```powershell
Import-Certificate -FilePath .\rootCA.crt -CertStoreLocation Cert:\LocalMachine\Root
```

### Option C — certutil (admin, silent)
```cmd
certutil -addstore Root rootCA.crt
```

## Verify

```powershell
Get-AuthenticodeSignature .\HanaLauncher.exe
```
Should report: `Status: Valid` — publisher **StievenW**, issuer **HanaLauncher CA**.

## Notes / limits
- Trust applies **only to machines where `rootCA.crt` is installed**. Public
  users who do not install it still see "Unknown Publisher".
- **SmartScreen still warns** for new/unknown publishers, even with a trusted
  private CA — the warning comes from Microsoft's reputation service, not the
  certificate chain.
- Do **not** share `rootCA.crt` with people you don't trust: a Root CA can
  impersonate any website or software on a machine that trusts it.
