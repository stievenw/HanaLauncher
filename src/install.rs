#![allow(dead_code)]
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;

use crate::download::{download_bytes, download_file, download_with_fallback};
use crate::minecraft::{
    parse_lib_name, Arguments, AssetIndex, JavaVersion, Library, LoaderKind,
    RuleContext, Version, VersionDownloads, VersionManifest,
};
use crate::util::{AUTH_LIB_INJECTOR_FALLBACK_URL, AUTH_LIB_INJECTOR_RELEASE_URL};
use crate::worker::Reporter;

pub fn version_json_path(root: &Path, id: &str) -> PathBuf {
    root.join("versions").join(id).join(format!("{id}.json"))
}

pub fn version_dir(root: &Path, id: &str) -> PathBuf {
    root.join("versions").join(id)
}

pub fn client_jar_path(root: &Path, id: &str) -> PathBuf {
    root.join("versions").join(id).join(format!("{id}.jar"))
}

pub fn natives_dir(root: &Path, id: &str) -> PathBuf {
    root.join("versions").join(id).join(format!("{id}-natives"))
}

pub fn assets_index_path(root: &Path, name: &str) -> PathBuf {
    root.join("assets").join("indexes").join(format!("{name}.json"))
}

/// For legacy versions (pre-1.7.10) the asset index must be *unpacked* into a
/// folder the old game reads directly:
/// - `pre-1.6` indexes map to `assets/resources/`
/// - `legacy` (1.6.x – 1.7.10) indexes map to `assets/virtual/legacy/`
/// Modern indexes return `None` (the game reads `assets/objects/` itself).
pub fn unpack_dir_for_index(root: &Path, index_id: &str) -> Option<PathBuf> {
    match index_id {
        "pre-1.6" => Some(root.join("assets").join("resources")),
        "legacy" => Some(root.join("assets").join("virtual").join(index_id)),
        _ => None,
    }
}

pub fn library_path(root: &Path, lib: &Library) -> Result<PathBuf> {
    if let Some(artifact) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()) {
        if let Some(path) = &artifact.path {
            return Ok(root.join("libraries").join(path));
        }
    }
    Ok(root.join("libraries").join(parse_lib_name(&lib.name)?.group.replace('.', "/")))
}

pub fn library_file_path(root: &Path, lib: &Library) -> Result<PathBuf> {
    let name = parse_lib_name(&lib.name)?;
    let base = format!(
        "{}/{}/{}/{}-{}",
        name.group.replace('.', "/"),
        name.artifact,
        name.version,
        name.artifact,
        name.version
    );
    let rel = match &name.classifier {
        Some(c) => format!("{base}-{c}.jar"),
        None => format!("{base}.jar"),
    };
    Ok(root.join("libraries").join(rel))
}

pub fn scan_installed(root: &Path) -> Vec<String> {
    let dir = root.join("versions");
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir()
                && p.file_name()
                    .map(|n| n.to_string_lossy().ends_with(".json"))
                    .unwrap_or(false)
            {
                continue;
            }
            if p.is_dir() {
                let id = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if !id.is_empty() && p.join(format!("{id}.json")).exists() {
                    out.push(id);
                }
            }
        }
    }
    out.sort();
    out
}

pub fn version_is_installed(root: &Path, id: &str) -> bool {
    version_json_path(root, id).exists() && client_jar_path(root, id).exists()
}

/// Newest stable "release" version that is fully installed, if any. Used by the
/// built-in "latest" instance to decide whether a newer release is available.
pub fn newest_installed_release(root: &Path, manifest: &VersionManifest) -> Option<String> {
    let versions_dir = root.join("versions");
    let mut best: Option<String> = None;
    for entry in std::fs::read_dir(&versions_dir).ok()?.flatten() {
        let id = entry.file_name().to_string_lossy().into_owned();
        let is_release = manifest
            .versions
            .iter()
            .any(|v| v.id == id && v.kind == "release");
        if !is_release {
            continue;
        }
        let dir = entry.path();
        if !dir.join(format!("{id}.json")).is_file() || !dir.join(format!("{id}.jar")).is_file() {
            continue;
        }
        let newer = best
            .as_deref()
            .map(|b| crate::util::cmp_version(&id, b) == std::cmp::Ordering::Greater)
            .unwrap_or(true);
        if newer {
            best = Some(id);
        }
    }
    best
}

/// Check that the artifact file for a library exists and, when its checksum
/// is known, matches it (i.e. it is neither missing nor corrupt).
pub fn library_valid(root: &Path, lib: &Library) -> bool {
    let path = match lib.artifact() {
        Some((artifact, _)) => match &artifact.path {
            Some(p) => root.join("libraries").join(p),
            None => library_file_path(root, lib).unwrap_or_default(),
        },
        None => library_file_path(root, lib).unwrap_or_default(),
    };
    if !path.is_file() {
        return false;
    }
    if let Some((artifact, _)) = lib.artifact() {
        if let Some(sha1) = &artifact.sha1 {
            return crate::download::sha1_of_path(&path)
                .map(|h| h.eq_ignore_ascii_case(sha1))
                .unwrap_or(false);
        }
    }
    true
}

/// Full pre-launch verification: version JSON, client jar, every allowed
/// library, the natives marker (legacy versions) and the completed assets.
/// Returns `false` when anything is missing or corrupt, so the caller can run
/// an incremental repair that only re-downloads the broken pieces.
pub fn verify_version_installed(root: &Path, version: &Version) -> bool {
    let client_jar = client_jar_path(root, &version.id);
    if !version_json_path(root, &version.id).exists() || !client_jar.is_file() {
        return false;
    }
    if let Some(sha1) = &version.downloads.client.sha1 {
        if !crate::download::sha1_of_path(&client_jar)
            .map(|h| h.eq_ignore_ascii_case(sha1))
            .unwrap_or(false)
        {
            return false;
        }
    }
    let ctx = RuleContext::current();
    for lib in &version.libraries {
        if lib.is_allowed(&ctx) && !library_valid(root, lib) {
            return false;
        }
    }
    if version.libraries.iter().any(|l| l.natives.is_some())
        && !natives_dir(root, &version.id).join(".extracted").exists()
    {
        return false;
    }
    assets_index_path(root, &version.asset_index.id)
        .with_extension("complete")
        .exists()
        && unpack_dir_for_index(root, &version.asset_index.id)
            .map(|d| d.join(".complete").exists())
            .unwrap_or(true)
}

/// Load a version JSON, downloading it (and the manifest) if needed.
pub fn load_or_fetch_version(client: &Client, root: &Path, id: &str) -> Result<Version> {
    let local = version_json_path(root, id);
    if local.exists() {
        if let Ok(text) = fs::read_to_string(&local) {
            if let Ok(v) = serde_json::from_str::<Version>(&text) {
                return Ok(v);
            }
        }
    }

    let lang = crate::lang::current();
    let manifest: VersionManifest =
        VersionManifest::from_remote(client).context(lang.failed_fetch_versions)?;
    let entry = manifest
        .versions
        .iter()
        .find(|v| v.id == id)
        .ok_or_else(|| anyhow!(lang.version_not_in_manifest.replace("{}", &id)))?;
    let bytes = download_bytes(client, &entry.url)
        .with_context(|| lang.failed_download_version.replace("{}", &id))?;
    let version: Version = serde_json::from_slice(&bytes)
        .with_context(|| lang.failed_parse_version.replace("{}", &id))?;
    if let Some(parent) = local.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&local, &bytes)?;
    Ok(version)
}

/// Download a Fabric/Quilt loader profile JSON into the local versions folder
/// and return the version id it declares (e.g. `fabric-loader-0.16.14-1.21.4`).
pub fn fetch_loader_profile(
    client: &Client,
    root: &Path,
    kind: LoaderKind,
    mc: &str,
    loader: &str,
) -> Result<String> {
    let url = format!(
        "{}/versions/loader/{}/{}/profile/json",
        kind.base_url(),
        mc,
        loader
    );
    let bytes = download_bytes(client, &url)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .context("Profil loader tidak valid (JSON tidak dikenal)")?;
    let id = value
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Profil loader tidak memiliki id"))?
        .to_string();
    let dir = version_dir(root, &id);
    fs::create_dir_all(&dir)?;
    fs::write(version_json_path(root, &id), &bytes)?;
    Ok(id)
}

/// Resolve a version and all of its parents into a single, fully materialized
/// `Version`. Client mod profiles (Fabric/Quilt) only carry `id`,
/// `inheritsFrom`, `mainClass`, extra `arguments` and their own `libraries`;
/// everything else (assetIndex, downloads, javaVersion, full argument set)
/// is merged in from the vanilla parent chain.
pub fn resolve_version(client: &Client, root: &Path, id: &str) -> Result<Version> {
    let lang = crate::lang::current();
    let mut chain: Vec<serde_json::Value> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut cur = id.to_string();

    loop {
        if !seen.insert(cur.clone()) {
            break;
        }
        let local = version_json_path(root, &cur);
        let raw: serde_json::Value = if local.exists() {
            serde_json::from_str(&fs::read_to_string(&local)?)
                .with_context(|| lang.failed_parse_version.replace("{}", &cur))?
        } else {
            let manifest = VersionManifest::from_remote(client)?;
            let entry = manifest
                .versions
                .iter()
                .find(|v| v.id == cur)
                .ok_or_else(|| anyhow!(lang.version_not_in_manifest.replace("{}", &cur)))?;
            let bytes = download_bytes(client, &entry.url)
                .with_context(|| lang.failed_download_version.replace("{}", &cur))?;
            if let Some(parent) = local.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&local, &bytes)?;
            serde_json::from_slice(&bytes)
                .with_context(|| lang.failed_parse_version.replace("{}", &cur))?
        };
        let inherits = raw
            .get("inheritsFrom")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        chain.push(raw);
        match inherits {
            Some(parent) => cur = parent,
            None => break,
        }
    }

    // chain[0] = the requested version, chain[last] = the root (a full vanilla
    // JSON). Start from the root and overlay each closer version.
    let mut merged: Version =
        serde_json::from_value(chain.last().expect("chain kosong").clone())?;
    for raw in chain.iter().rev().skip(1) {
        if let Some(s) = raw.get("mainClass").and_then(|v| v.as_str()) {
            merged.main_class = s.to_string();
        }
        if let Some(s) = raw.get("id").and_then(|v| v.as_str()) {
            merged.id = s.to_string();
        }
        if let Some(s) = raw.get("type").and_then(|v| v.as_str()) {
            merged.kind = Some(s.to_string());
        }
        if let Some(a) = raw.get("arguments") {
            if let Ok(args) = serde_json::from_value::<Arguments>(a.clone()) {
                let base = merged
                    .arguments
                    .get_or_insert_with(|| Arguments { game: Vec::new(), jvm: Vec::new() });
                base.game.extend(args.game);
                base.jvm.extend(args.jvm);
            }
        }
        if let Some(s) = raw.get("minecraftArguments").and_then(|v| v.as_str()) {
            merged.minecraft_arguments = Some(s.to_string());
        }
        if let Some(l) = raw.get("libraries") {
            if let Ok(libs) = serde_json::from_value::<Vec<Library>>(l.clone()) {
                for lib in libs {
                    if !merged.libraries.iter().any(|x| x.name == lib.name) {
                        merged.libraries.push(lib);
                    }
                }
            }
        }
        if let Some(aj) = raw.get("assetIndex") {
            if let Ok(a) = serde_json::from_value::<AssetIndex>(aj.clone()) {
                merged.asset_index = a;
            }
        }
        if let Some(d) = raw.get("downloads") {
            if let Ok(dd) = serde_json::from_value::<VersionDownloads>(d.clone()) {
                merged.downloads = dd;
            }
        }
        if let Some(j) = raw.get("javaVersion") {
            if let Ok(jj) = serde_json::from_value::<JavaVersion>(j.clone()) {
                merged.java_version = Some(jj);
            }
        }
    }
    merged.inherits_from = None;
    Ok(merged)
}

/// Load a locally installed version JSON only (never contacts the network).
pub fn load_local_version(root: &Path, id: &str) -> Result<Version> {
    let local = version_json_path(root, id);
    let text = fs::read_to_string(&local)
        .with_context(|| crate::lang::current().failed_parse_version.replace("{}", id))?;
    serde_json::from_str::<Version>(&text)
        .with_context(|| crate::lang::current().failed_parse_version.replace("{}", id))
}

/// Ensure the authlib-injector jar is present under the minecraft root.
pub fn ensure_authlib_injector(client: &Client, root: &Path, reporter: &Reporter) -> Result<PathBuf> {
    let dest = root.join("authlib-injector.jar");
    if dest.exists() {
        return Ok(dest);
    }
    reporter.log("Mengunduh authlib-injector untuk Ely.by...");

    let mut candidates: Vec<String> = Vec::new();
    match download_bytes(client, AUTH_LIB_INJECTOR_RELEASE_URL) {
        Ok(bytes) => {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if let Some(assets) = value.get("assets").and_then(|a| a.as_array()) {
                    for asset in assets {
                        if let Some(name) = asset.get("name").and_then(|n| n.as_str()) {
                            if name.ends_with(".jar") && !name.contains("sources") {
                                if let Some(url) = asset.get("browser_download_url").and_then(|u| u.as_str()) {
                                    candidates.push(url.to_string());
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
        Err(_) => {}
    }
    candidates.push(AUTH_LIB_INJECTOR_FALLBACK_URL.to_string());

    download_with_fallback(client, &candidates, &dest, None, reporter, "authlib-injector")
        .context(crate::lang::current().failed_download_authlib)?;
    Ok(dest)
}

fn download_library(client: &Client, root: &Path, lib: &Library, reporter: &Reporter) -> Result<()> {
    let (artifact, is_native) = match lib.artifact() {
        Some(a) => a,
        None => {
            // Legacy library without `downloads` info - derive path/URL from the name.
            if lib.natives.is_some() {
                return Ok(());
            }
            let dest = library_file_path(root, lib)?;
            let base = lib.url.as_deref().unwrap_or("https://libraries.minecraft.net/");
            let rel = dest
                .strip_prefix(&root.join("libraries"))
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let url = format!("{}/{}", base.trim_end_matches('/'), rel);
            return download_file(client, &url, &dest, None, None, reporter, "Library");
        }
    };
    if is_native {
        return Ok(());
    }

    let dest = match &artifact.path {
        Some(p) => root.join("libraries").join(p),
        None => library_file_path(root, lib)?,
    };
    let url = if artifact.url.is_empty() {
        let base = lib.url.as_deref().unwrap_or("https://libraries.minecraft.net/");
        let rel = dest.strip_prefix(&root.join("libraries")).unwrap_or(Path::new(""));
        format!("{}{}", base, rel.to_string_lossy())
    } else {
        artifact.url.clone()
    };
    download_file(
        client,
        &url,
        &dest,
        artifact.sha1.as_deref(),
        artifact.size,
        reporter,
        "Library",
    )
}

fn download_natives(client: &Client, root: &Path, lib: &Library, natives_dir: &Path, reporter: &Reporter) -> Result<()> {
    let (artifact, is_native) = lib
        .artifact()
        .ok_or_else(|| anyhow!("Library {} tidak memiliki classifier native", lib.name))?;
    if !is_native {
        return Ok(());
    }
    let tmp = root.join("libraries").join(".natives-tmp.jar");
    download_file(
        client,
        &artifact.url,
        &tmp,
        artifact.sha1.as_deref(),
        artifact.size,
        reporter,
        "Native",
    )?;
    extract_natives(&tmp, natives_dir)?;
    let _ = fs::remove_file(&tmp);
    Ok(())
}

fn extract_natives(jar: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    let lang = crate::lang::current();
    let file = File::open(jar).context(lang.failed_open_native_jar)?;
    let mut archive = zip::ZipArchive::new(file).context(lang.invalid_archive)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();
        if entry.is_dir() || name.starts_with("META-INF/") {
            continue;
        }
        let lower = name.to_lowercase();
        let keep = lower.ends_with(".dll")
            || lower.ends_with(".so")
            || lower.ends_with(".dylib")
            || lower.ends_with(".jnilib")
            || lower.ends_with(".properties")
            || lower.ends_with(".dat")
            || lower.ends_with(".cfg");
        if !keep {
            continue;
        }
        // Strip any leading directory components; keep only the file name to be safe.
        let file_name = Path::new(&name)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        if file_name.is_empty() {
            continue;
        }
        let out_path = dest.join(&file_name);
        let mut out = BufWriter::new(File::create(&out_path)?);
        std::io::copy(&mut entry, &mut out)?;
        out.flush()?;
    }
    Ok(())
}

/// Download everything required for a version and return its parsed JSON.
pub fn install_version(
    client: &Client,
    root: &Path,
    version_id: &str,
    reporter: &Reporter,
) -> Result<Version> {
    let version = resolve_version(client, root, version_id)?;
    reporter.log(crate::lang::current().installing_version_log.replace("{}", &version.id));
    let ctx = RuleContext::current();

    // 1) Libraries
    let libs: Vec<&Library> = version.libraries.iter().filter(|l| l.is_allowed(&ctx)).collect();
    let mut done = 0usize;
    let total = libs.len();
    for lib in &libs {
        reporter.log(format!("  Library [{}/{}] {}", done + 1, total, lib.name));
        download_library(client, root, lib, reporter)?;
        done += 1;
    }

    // 2) Client jar
    let client_jar = client_jar_path(root, &version.id);
    let cd = &version.downloads.client;
    reporter.log(crate::lang::current().downloading_client_jar);
    download_file(
        client,
        &cd.url,
        &client_jar,
        cd.sha1.as_deref(),
        cd.size,
        reporter,
        "Client JAR",
    )?;

    // 3) Natives
    let natives = natives_dir(root, &version.id);
    let marker = natives.join(".extracted");
    if !marker.exists() {
        reporter.log(crate::lang::current().extracting_natives);
        let _ = fs::remove_dir_all(&natives);
        for lib in &libs {
            if lib.natives.is_some() {
                download_natives(client, root, lib, &natives, reporter)?;
            }
        }
        let _ = fs::write(&marker, b"ok");
    }

    // 4) Assets index + objects
    ensure_assets(client, root, &version, reporter)?;

    // 5) authlib-injector
    ensure_authlib_injector(client, root, reporter)?;

    reporter.log("Instalasi selesai.");
    Ok(version)
}

fn ensure_assets(client: &Client, root: &Path, version: &Version, reporter: &Reporter) -> Result<()> {
    let index = &version.asset_index;
    let index_path = assets_index_path(root, &index.id);
    let objects_dir = root.join("assets").join("objects");
    let unpack_dir = unpack_dir_for_index(root, &index.id);
    let needs_unpack = unpack_dir
        .as_ref()
        .map(|d| !d.join(".complete").exists())
        .unwrap_or(false);
    // Fast path: a previous run already verified/downloaded everything.
    let marker = assets_index_path(root, &index.id).with_extension("complete");
    if marker.exists() && !needs_unpack {
        return Ok(());
    }
    reporter.log(
        crate::lang::current()
            .checking_assets
            .replace("{}", &index.id),
    );
    download_file(
        client,
        &index.url,
        &index_path,
        index.sha1.as_deref(),
        index.size,
        reporter,
        "Asset Index",
    )?;

    let text = fs::read_to_string(&index_path)?;
    let parsed: crate::minecraft::AssetIndexJson =
        serde_json::from_str(&text).context(crate::lang::current().invalid_asset_index)?;

    let mut pending: Vec<(String, u64)> = parsed
        .objects
        .iter()
        .map(|(_, obj)| (obj.hash.clone(), obj.size))
        .collect();
    let total: u64 = pending.iter().map(|(_, s)| *s).sum();
    let mut done: u64 = 0;

    pending.retain(|(hash, _)| {
        let path = objects_dir.join(&hash[..2]).join(hash);
        let exists = path.is_file() && crate::download::sha1_of_path(&path).map(|h| h == *hash).unwrap_or(false);
        if !exists {
            true
        } else {
            done += 0;
            false
        }
    });

    let n = pending.len();
    for (i, (hash, size)) in pending.iter().enumerate() {
        let url = format!(
            "https://resources.download.minecraft.net/{}/{}",
            &hash[..2],
            hash
        );
        let dest = objects_dir.join(&hash[..2]).join(hash);
        if i % 50 == 0 {
            reporter.log(
                crate::lang::current()
                    .asset_progress
                    .replace("{}", &(i + 1).to_string())
                    .replacen("{}", &n.to_string(), 1),
            );
        }
        download_file(client, &url, &dest, Some(hash), Some(*size), reporter, &crate::lang::current().asset_stage)?;
        done += size;
        if n > 0 {
            reporter.progress(&crate::lang::current().asset_stage, done.min(total.max(1)), total.max(1));
        }
    }
    // Unpack legacy indexes so pre-1.7.10 games can read files directly
    // (they have no hashed-objects loader).
    if let Some(dir) = &unpack_dir {
        let _ = fs::create_dir_all(dir);
        for (key, obj) in &parsed.objects {
            let src = objects_dir.join(&obj.hash[..2]).join(&obj.hash);
            let dst = dir.join(key);
            if dst.is_file()
                && dst.metadata().map(|m| m.len()).unwrap_or(0) == obj.size
            {
                continue;
            }
            if let Some(parent) = dst.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::copy(&src, &dst);
        }
        let _ = fs::write(dir.join(".complete"), b"ok");
    }
    // Mark the asset set as fully downloaded so later plays skip the scan.
    let _ = fs::write(&marker, b"ok");
    Ok(())
}

/// Helper to pretty-print remaining bytes for reporting.
#[allow(dead_code)]
fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

pub fn write_text(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = BufWriter::new(File::create(path)?);
    f.write_all(text.as_bytes())?;
    f.flush()?;
    Ok(())
}