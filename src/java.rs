#![allow(dead_code)]
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use reqwest::blocking::Client;
use serde_json::Value;

use crate::download::{download_bytes, download_file};
use crate::util::java_platform_key;
use crate::worker::Reporter;

pub struct JavaInfo {
    pub path: PathBuf,
    pub major: u32,
}

pub fn java_binary_name() -> &'static str {
    if cfg!(windows) {
        "java.exe"
    } else {
        "java"
    }
}

/// Try to locate an installed Java (config path, JAVA_HOME, PATH, cached runtime).
pub fn detect_java(config_path: Option<&str>, root: &Path) -> Option<PathBuf> {
    if let Some(p) = config_path {
        let b = PathBuf::from(p);
        if b.exists() {
            return Some(b);
        }
    }

    if let Ok(home) = std::env::var("JAVA_HOME") {
        let cand = PathBuf::from(home).join("bin").join(java_binary_name());
        if cand.exists() {
            return Some(cand);
        }
    }

    if let Some(found) = find_on_path(java_binary_name()) {
        return Some(found);
    }

    if let Some(cached) = find_cached_runtime(root) {
        return Some(cached.join("bin").join(java_binary_name()));
    }

    None
}

fn find_on_path(program: &str) -> Option<PathBuf> {
    let path_var = std::env::var("PATH").ok()?;
    for dir in std::env::split_paths(&path_var) {
        let cand = dir.join(program);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

pub fn find_cached_runtime(root: &Path) -> Option<PathBuf> {
    let runtime_dir = root.join("runtime");
    if !runtime_dir.exists() {
        return None;
    }
    let entries = std::fs::read_dir(&runtime_dir).ok()?;
    for entry in entries.flatten() {
        let bin = entry.path().join("bin").join(java_binary_name());
        if bin.is_file() {
            return Some(entry.path());
        }
    }
    None
}

/// Find a cached runtime whose Java major version is compatible with
/// `required`. Probes each candidate with `java -version`.
pub fn find_cached_runtime_major(root: &Path, required: u32) -> Option<PathBuf> {
    let runtime_dir = root.join("runtime");
    if !runtime_dir.exists() {
        return None;
    }
    for entry in std::fs::read_dir(&runtime_dir).ok()?.flatten() {
        let bin = entry.path().join("bin").join(java_binary_name());
        if bin.is_file() {
            if let Ok(major) = java_major(&bin) {
                if java_compatible(major, required) {
                    return Some(entry.path());
                }
            }
        }
    }
    None
}

/// Whether a detected Java of `major` may run a version that needs `required`.
/// Legacy LaunchWrapper versions break on anything above Java 8, so they must
/// use Java 8 exactly. Newer runtimes accept a small range of majors.
pub fn java_compatible(major: u32, required: u32) -> bool {
    if major == required {
        return true;
    }
    match required {
        8 => false,
        17 => (17..=21).contains(&major),
        21 => (21..=25).contains(&major),
        _ => major >= required,
    }
}

pub fn java_major(java_path: &Path) -> Result<u32> {
    let mut cmd = Command::new(java_path);
    cmd.arg("-version");
    // Never pop a console window for the version probe.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().context(crate::lang::current().failed_java_version)?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let text = format!("{stdout}\n{stderr}");
    parse_java_major(&text)
}

pub fn parse_java_major(version_output: &str) -> Result<u32> {
    let lang = crate::lang::current();
    let line = version_output
        .lines()
        .find(|l| l.contains("version"))
        .ok_or_else(|| anyhow!(lang.cannot_read_java_version))?;
    let start = line
        .find('"')
        .ok_or_else(|| anyhow!(lang.invalid_java_version_format))? + 1;
    let rest = line[start..].split('"').next().unwrap_or("");
    let nums: Vec<&str> = rest.split('.').collect();
    let major = match nums.as_slice() {
        ["1", minor, ..] => minor.parse::<u32>().unwrap_or(8),
        [first, ..] => first.parse::<u32>().unwrap_or(17),
        [] => 17,
    };
    Ok(major)
}

fn category_for_major(major: u32) -> &'static str {
    match major {
        0..=8 => "jre-legacy",
        9..=16 => "java-runtime-beta",
        17 => "java-runtime-gamma",
        18..=20 => "java-runtime-delta",
        _ => "java-runtime-epsilon",
    }
}

/// Download and extract a Mojang Java runtime suitable for the required major
/// version. Returns the directory containing `bin/java`.
pub fn download_runtime(
    client: &Client,
    root: &Path,
    required_major: u32,
    reporter: &Reporter,
) -> Result<PathBuf> {
    reporter.log("Mengambil indeks Java runtime dari Mojang...");
    let all: Value = serde_json::from_slice(&download_bytes(client, crate::util::JAVA_RUNTIME_URL)?)
        .context("Format indeks Java runtime tidak dikenal")?;

    let platform = java_platform_key();
    let lang = crate::lang::current();
    let platforms = all
        .as_object()
        .ok_or_else(|| anyhow!(lang.java_index_empty))?;
    let platform_meta = platforms
        .get(&platform)
        .ok_or_else(|| anyhow!(lang.no_java_for_platform.replace("{}", &platform)))?;

    let preferred = category_for_major(required_major);
    let categories = platform_meta
        .as_object()
        .ok_or_else(|| anyhow!(lang.invalid_java_runtime_data))?;

    let category = if categories.contains_key(preferred) {
        preferred
    } else {
        categories
            .keys()
            .find(|k| k.starts_with("java-runtime") || k == &"jre-legacy")
            .map(|s| s.as_str())
            .ok_or_else(|| anyhow!(lang.no_matching_runtime))?
    };

    let list = categories
        .get(category)
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!(lang.invalid_runtime_data.replace("{}", category)))?;
    let manifest_url = list
        .first()
        .and_then(|v| v.get("manifest"))
        .and_then(|m| m.get("url"))
        .and_then(|u| u.as_str())
        .ok_or_else(|| anyhow!(lang.no_manifest_for.replace("{}", category)))?
        .to_string();

    let target = root.join("runtime").join(format!("{category}-{platform}"));
    if target.join("bin").join(java_binary_name()).is_file() {
        reporter.log(
            lang.java_runtime_installed
                .replace("{}", &target.display().to_string()),
        );
        return Ok(target);
    }

    reporter.log(
        lang.downloading_java
            .replace("{}", category)
            .replacen("{}", &required_major.to_string(), 1),
    );
    let manifest: Value = serde_json::from_slice(&download_bytes(client, &manifest_url)?)
        .context(lang.invalid_runtime_manifest)?;

    let files = manifest
        .get("files")
        .and_then(|f| f.as_object())
        .ok_or_else(|| anyhow!(lang.runtime_manifest_no_files))?;

    let mut file_entries: Vec<(String, String, Option<String>, Option<u64>)> = Vec::new();
    for (rel_path, entry) in files {
        let etype = entry.get("type").and_then(|t| t.as_str()).unwrap_or("file");
        match etype {
            "file" => {
                let raw = entry.get("downloads").and_then(|d| d.get("raw"));
                let url = raw.and_then(|r| r.get("url")).and_then(|u| u.as_str());
                let sha1 = raw.and_then(|r| r.get("sha1")).and_then(|s| s.as_str());
                let size = raw.and_then(|r| r.get("size")).and_then(|s| s.as_u64());
                if let Some(url) = url {
                    file_entries.push((rel_path.clone(), url.to_string(), sha1.map(|s| s.to_string()), size));
                }
            }
            "directory" => {
                let dir = target.join(rel_path);
                let _ = std::fs::create_dir_all(&dir);
            }
            _ => {}
        }
    }

    let total: u64 = file_entries.iter().map(|(_, _, _, s)| s.unwrap_or(0)).sum();
    let mut done: u64 = 0;
    let n = file_entries.len();
    reporter.progress("Mengunduh Java runtime", 0, total.max(1));

    for (i, (rel_path, url, sha1, size)) in file_entries.iter().enumerate() {
        let dest = target.join(rel_path);
        reporter.log(format!("  [{}/{}] {}", i + 1, n, rel_path));
        download_file(
            client,
            url,
            &dest,
            sha1.as_deref(),
            *size,
            reporter,
            "Mengunduh Java runtime",
        )?;
        done += size.unwrap_or(0);
        reporter.progress("Mengunduh Java runtime", done, total.max(1));
    }

    if !target.join("bin").join(java_binary_name()).is_file() {
        bail!("Java runtime diunduh tetapi java tidak ditemukan di dalamnya.");
    }

    Ok(target)
}