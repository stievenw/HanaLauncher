#![allow(dead_code)]
use std::cmp::Ordering;
use std::path::Path;

use sha1::{Digest, Sha1};

pub const LAUNCHER_NAME: &str = "HanaLauncher";
pub const LAUNCHER_VERSION: &str = "1.1.0";

/// Default launcher brand (Legacy Launcher style), overridable via `--brand`.
pub const DEFAULT_BRAND: &str = "hana";
/// Default update channel, overridable via `--channel`.
pub const DEFAULT_CHANNEL: &str = "hanakama";

pub const VERSION_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

pub const OAUTH_DEVICE_CODE_URL: &str = "https://account.ely.by/api/oauth2/v1/devicecode";
pub const OAUTH_TOKEN_URL: &str = "https://account.ely.by/api/oauth2/v1/token";
pub const OAUTH_INFO_URL: &str = "https://account.ely.by/api/account/v1/info";

pub const ELY_REGISTER_URL: &str = "https://account.ely.by/register";

pub const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";

pub const YGGDRASIL_AUTH_URL: &str = "https://authserver.ely.by/auth/authenticate";
pub const YGGDRASIL_REFRESH_URL: &str = "https://authserver.ely.by/auth/refresh";

pub const JAVA_RUNTIME_URL: &str = "https://piston-meta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

pub const AUTH_LIB_INJECTOR_RELEASE_URL: &str =
    "https://api.github.com/repos/yushijinhun/authlib-injector/releases/latest";
pub const AUTH_LIB_INJECTOR_FALLBACK_URL: &str =
    "https://github.com/yushijinhun/authlib-injector/releases/download/v1.2.8/authlib-injector-1.2.8.jar";

pub const OAUTH_SCOPES: &str = "account_info account_email minecraft_server_session offline_access";

/// Hardcoded Ely.by application Client ID (Desktop application, no secret).
pub const ELY_CLIENT_ID: &str = "hanalauncher1";

pub fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Format a byte count into a human readable size (e.g. "295.9 MB").
pub fn fmt_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn sha1_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buf = [0u8; 81920];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

pub fn os_rule_name() -> &'static str {
    match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "osx",
        _ => "linux",
    }
}

pub fn arch_rule() -> &'static str {
    match std::env::consts::ARCH {
        "x86" => "x86",
        "aarch64" => "arm64",
        "arm" => "arm",
        _ => "x86_64",
    }
}

/// Classifier substitution value used inside natives keys (e.g. `natives-windows-${arch}`).
pub fn arch_native() -> &'static str {
    match std::env::consts::ARCH {
        "x86" => "32",
        "aarch64" => "arm64",
        "arm" => "arm",
        _ => "64",
    }
}

pub fn java_platform_key() -> String {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "windows-x64".to_string(),
        ("windows", "x86") => "windows-x86".to_string(),
        ("windows", "aarch64") => "windows-arm64".to_string(),
        ("macos", "x86_64") => "osx-x86_64".to_string(),
        ("macos", "aarch64") => "osx-arm64".to_string(),
        ("linux", "x86_64") => "linux-x64".to_string(),
        ("linux", "aarch64") => "linux-arm64".to_string(),
        ("linux", "x86") => "linux-x86".to_string(),
        (os, arch) => format!("{os}-{arch}"),
    }
}

pub fn native_extension() -> &'static str {
    match std::env::consts::OS {
        "windows" => "dll",
        "macos" => "dylib",
        _ => "so",
    }
}

/// Compare two Minecraft version ids numerically (`1.21.4` < `26.2`).
/// Non-numeric parts are ignored, so pre-release ids fall back to the numeric
/// prefix comparison.
pub fn cmp_version(a: &str, b: &str) -> Ordering {
    let nums = |s: &str| {
        s.split('.')
            .filter_map(|p| p.parse::<u64>().ok())
            .collect::<Vec<u64>>()
    };
    let pa = nums(a);
    let pb = nums(b);
    for (x, y) in pa.iter().zip(pb.iter()) {
        match x.cmp(y) {
            Ordering::Equal => continue,
            o => return o,
        }
    }
    pa.len().cmp(&pb.len())
}
