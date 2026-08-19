#![allow(dead_code)]
use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::Value;

use crate::util::{arch_native, arch_rule, os_rule_name};

#[derive(Debug, Deserialize, Clone)]
pub struct VersionManifest {
    pub latest: Latest,
    pub versions: Vec<ManifestVersion>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Latest {
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ManifestVersion {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    pub release_time: Option<String>,
}

impl VersionManifest {
    pub fn from_remote(client: &reqwest::blocking::Client) -> Result<Self> {
        let resp = client
            .get(crate::util::VERSION_MANIFEST_URL)
            .send()
            .context(crate::lang::current().failed_fetch_versions)?;
        let text = resp
            .text()
            .context(crate::lang::current().failed_read_versions)?;
        Ok(serde_json::from_str(&text).context("Format daftar versi tidak dikenal")?)
    }
}

/// Client mod / loader families the launcher can install on top of a vanilla
/// release: Fabric (meta.fabricmc.net), Quilt (meta.quiltmc.org) and Forge
/// (maven.minecraftforge.net). The complete per-version lists come from the
/// scraped client-mod catalog (see tools/scrape-mod-catalog.ps1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderKind {
    Fabric,
    Quilt,
    Forge,
}

impl LoaderKind {
    pub fn base_url(&self) -> &'static str {
        match self {
            LoaderKind::Fabric => "https://meta.fabricmc.net/v2",
            LoaderKind::Quilt => "https://meta.quiltmc.org/v3",
            LoaderKind::Forge => "https://maven.minecraftforge.net",
        }
    }

    /// URL of the loader profile JSON for (mc, loader).
    pub fn profile_url(&self, mc: &str, loader: &str) -> String {
        match self {
            LoaderKind::Fabric | LoaderKind::Quilt => format!(
                "{}/versions/loader/{}/{}/profile/json",
                self.base_url(),
                mc,
                loader
            ),
            LoaderKind::Forge => format!(
                "{}/net/minecraftforge/forge/{}-{}/forge-{}-{}.json",
                self.base_url(),
                mc,
                loader,
                mc,
                loader
            ),
        }
    }

    /// Version id installed in the versions folder for (mc, loader).
    pub fn version_id(&self, mc: &str, loader: &str) -> String {
        match self {
            LoaderKind::Fabric => format!("fabric-loader-{}-{}", loader, mc),
            LoaderKind::Quilt => format!("quilt-loader-{}-{}", loader, mc),
            LoaderKind::Forge => format!("{}-forge-{}", mc, loader),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct LoaderListEntry {
    pub loader: LoaderMeta,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LoaderMeta {
    pub version: String,
    pub stable: Option<bool>,
}

/// Fetch the loader versions available for a Minecraft version, newest stable
/// first (stable entries before beta/alpha builds, then by version number).
pub fn fetch_loader_list(
    client: &reqwest::blocking::Client,
    kind: LoaderKind,
    mc: &str,
) -> Result<Vec<LoaderMeta>> {
    let url = format!("{}/versions/loader/{}", kind.base_url(), mc);
    let text = client
        .get(&url)
        .send()
        .context(crate::lang::current().failed_fetch_loaders)?
        .text()
        .context(crate::lang::current().failed_read_versions)?;
    let entries: Vec<LoaderListEntry> = serde_json::from_str(&text)
        .context(crate::lang::current().failed_fetch_loaders)?;
    let mut metas: Vec<LoaderMeta> = entries.into_iter().map(|e| e.loader).collect();
    metas.sort_by(|a, b| {
        let sa = a.stable.unwrap_or(false);
        let sb = b.stable.unwrap_or(false);
        sb.cmp(&sa)
            .then_with(|| crate::util::cmp_version(&b.version, &a.version))
    });
    Ok(metas)
}

/// Full scraped client-mod catalog (all Fabric/Quilt/Forge versions), built by
/// tools/scrape-mod-catalog.ps1 and hosted next to the launcher releases.
/// The launcher reads it first and falls back to the live metadata endpoints.
#[derive(Debug, Deserialize)]
pub struct LoaderCatalog {
    pub versions: std::collections::HashMap<String, LoaderFamilyCatalog>,
}

#[derive(Debug, Deserialize)]
pub struct LoaderFamilyCatalog {
    #[serde(default)]
    pub loaders: std::collections::HashMap<String, Vec<String>>,
}

const MOD_CATALOG_URL: &str =
    "https://stievenw.github.io/HanaLauncher-portal/client-mod-catalog.json";

/// Loader lists (fabric, quilt, forge) for a Minecraft version, newest first.
/// For each family, uses the scraped catalog when the version is present and
/// falls back to the live metadata endpoint (meta.fabricmc.net,
/// meta.quiltmc.org, maven.minecraftforge.net) otherwise - one stale family
/// never breaks the others, and versions still get a correct, official list.
pub fn fetch_loader_lists(
    client: &reqwest::blocking::Client,
    mc: &str,
) -> Result<(Vec<LoaderMeta>, Vec<LoaderMeta>, Vec<LoaderMeta>)> {
    let cat = fetch_catalog(client).ok();
    let fam_meta = |name: &str| {
        cat.as_ref()
            .and_then(|c| c.versions.get(name))
            .and_then(|f| f.loaders.get(mc))
            .map(|versions| {
                let mut metas: Vec<LoaderMeta> = versions
                    .iter()
                    .map(|v| LoaderMeta {
                        version: v.clone(),
                        stable: Some(!v.contains("beta")),
                    })
                    .collect();
                metas.sort_by(|a, b| crate::util::cmp_version(&b.version, &a.version));
                metas
            })
    };
    let fabric = match fam_meta("fabric") {
        Some(metas) => metas,
        None => fetch_loader_list(client, LoaderKind::Fabric, mc)?,
    };
    let quilt = match fam_meta("quilt") {
        Some(metas) => metas,
        None => fetch_loader_list(client, LoaderKind::Quilt, mc)?,
    };
    let forge = match fam_meta("forge") {
        Some(metas) => metas,
        None => fetch_forge_list(client, mc)?,
    };
    Ok((fabric, quilt, forge))
}

fn fetch_catalog(client: &reqwest::blocking::Client) -> Result<LoaderCatalog> {
    let text = client
        .get(MOD_CATALOG_URL)
        .send()
        .context(crate::lang::current().failed_fetch_loaders)?
        .text()
        .context(crate::lang::current().failed_read_versions)?;
serde_json::from_str(&text).context(crate::lang::current().failed_fetch_loaders)
}

/// Forge loader list for a Minecraft version, parsed from the Maven
/// maven-metadata.xml (entries look like "1.21.1-52.0.1").
pub fn fetch_forge_list(
    client: &reqwest::blocking::Client,
    mc: &str,
) -> Result<Vec<LoaderMeta>> {
    let url = format!(
        "{}/net/minecraftforge/forge/maven-metadata.xml",
        LoaderKind::Forge.base_url()
    );
    let text = client
        .get(&url)
        .send()
        .context(crate::lang::current().failed_fetch_loaders)?
        .text()
        .context(crate::lang::current().failed_read_versions)?;
    let mut metas = Vec::new();
    for cap in text.split("<version>").skip(1) {
        let version = cap.split("</version>").next().unwrap_or("");
        if let Some(build) = version.strip_prefix(&format!("{mc}-")) {
            metas.push(LoaderMeta {
                version: build.to_string(),
                stable: Some(true),
            });
        }
    }
    metas.sort_by(|a, b| crate::util::cmp_version(&b.version, &a.version));
    Ok(metas)
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub id: String,
    pub main_class: String,
    pub minecraft_arguments: Option<String>,
    pub arguments: Option<Arguments>,
    pub asset_index: AssetIndex,
    pub downloads: VersionDownloads,
    pub libraries: Vec<Library>,
    pub java_version: Option<JavaVersion>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub inherits_from: Option<String>,
}

impl Version {
    pub fn asset_index_name(&self) -> &str {
        &self.asset_index.id
    }

    pub fn required_java_major(&self) -> u32 {
        // Versions that ship a `javaVersion` field pin the runtime (e.g. 1.17+
        // -> 17/21/25). Pre-1.17 versions use the LaunchWrapper (and old LWJGL)
        // which only runs on Java 8, so fall back to 8 when the field is absent.
        self.java_version.as_ref().map(|j| j.major_version).unwrap_or(8)
    }

    pub fn type_label(&self) -> &str {
        self.kind.as_deref().unwrap_or("release")
    }

    /// Whether the version uses the authlib session system (1.6+). Older
    /// versions read the username/token straight from the launch wrapper and
    /// cannot use authlib-injector.
    pub fn uses_authlib(&self) -> bool {
        self.asset_index.id != "pre-1.6"
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct JavaVersion {
    pub component: Option<String>,
    pub major_version: u32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Arguments {
    pub game: Vec<ArgValue>,
    pub jvm: Vec<ArgValue>,
}

#[derive(Debug, Clone)]
pub struct ResolvedArgs {
    pub jvm: Vec<String>,
    pub game: Vec<String>,
}

impl Arguments {
    pub fn resolve(&self, ctx: &RuleContext) -> ResolvedArgs {
        ResolvedArgs {
            jvm: self.jvm.iter().flat_map(|a| a.to_strings(ctx)).collect(),
            game: self.game.iter().flat_map(|a| a.to_strings(ctx)).collect(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum ArgValue {
    Str(String),
    Rules(RulesValue),
}

impl ArgValue {
    pub fn to_strings(&self, ctx: &RuleContext) -> Vec<String> {
        match self {
            ArgValue::Str(s) => vec![s.clone()],
            ArgValue::Rules(r) => {
                if r.applies(ctx) {
                    match &r.value {
                        Value::String(s) => vec![s.clone()],
                        Value::Array(items) => items
                            .iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect(),
                        _ => vec![],
                    }
                } else {
                    vec![]
                }
            }
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct RulesValue {
    pub rules: Vec<Rule>,
    pub value: Value,
}

impl RulesValue {
    pub fn applies(&self, ctx: &RuleContext) -> bool {
        eval_rules(&self.rules, ctx)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Rule {
    pub action: String,
    pub os: Option<OsRule>,
    pub features: Option<serde_json::Map<String, Value>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OsRule {
    pub name: Option<String>,
    pub arch: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Default)]
pub struct RuleContext {
    pub os_name: String,
    pub arch: String,
    pub has_custom_resolution: bool,
    pub is_demo_user: bool,
}

impl RuleContext {
    pub fn current() -> Self {
        Self {
            os_name: os_rule_name().to_string(),
            arch: arch_rule().to_string(),
            has_custom_resolution: false,
            is_demo_user: false,
        }
    }

    fn matches_os(&self, os: &OsRule) -> bool {
        if let Some(name) = &os.name {
            if *name != self.os_name {
                return false;
            }
        }
        if let Some(arch) = &os.arch {
            if *arch != self.arch {
                return false;
            }
        }
        if let Some(version) = &os.version {
            // Rarely used by Mojang libraries; accept if our OS name hints match.
            let _ = version;
        }
        true
    }

    fn matches_features(&self, features: &serde_json::Map<String, Value>) -> bool {
        for (key, value) in features {
            let want = value.as_bool().unwrap_or(false);
            let have = match key.as_str() {
                "has_custom_resolution" => self.has_custom_resolution,
                "is_demo_user" => self.is_demo_user,
                // Unsupported features (quick play etc.) never match.
                _ => false,
            };
            if want && !have {
                return false;
            }
        }
        true
    }
}

pub fn eval_rules(rules: &[Rule], ctx: &RuleContext) -> bool {
    // Mojang semantics: nothing is allowed unless a matching rule says so.
    let mut allowed = false;
    for rule in rules {
        let mut match_this = true;
        if let Some(os) = &rule.os {
            if !ctx.matches_os(os) {
                match_this = false;
            }
        }
        if let Some(features) = &rule.features {
            if !ctx.matches_features(features) {
                match_this = false;
            }
        }
        if match_this {
            allowed = rule.action == "allow";
        }
    }
    allowed
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AssetIndex {
    pub id: String,
    pub url: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub total_size: Option<u64>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct VersionDownloads {
    pub client: Artifact,
    pub server: Option<Artifact>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Library {
    pub name: String,
    pub rules: Option<Vec<Rule>>,
    pub natives: Option<HashMap<String, String>>,
    pub downloads: Option<LibraryDownloads>,
    pub url: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LibraryDownloads {
    pub artifact: Option<Artifact>,
    pub classifiers: Option<HashMap<String, Artifact>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Artifact {
    pub path: Option<String>,
    pub url: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ParsedLibName {
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub classifier: Option<String>,
}

pub fn parse_lib_name(name: &str) -> Result<ParsedLibName> {
    let mut parts: Vec<&str> = name.split(':').collect();
    if parts.len() < 3 {
        return Err(anyhow!("Nama library tidak valid: {name}"));
    }
    let classifier = parts.pop();
    let version = parts.pop().unwrap_or("");
    let artifact = parts.pop().unwrap_or("");
    let group = parts.join(":");
    Ok(ParsedLibName {
        group,
        artifact: artifact.to_string(),
        version: version.to_string(),
        classifier: classifier
            .filter(|c| !c.is_empty())
            .map(|s| s.to_string()),
    })
}

pub fn library_path_from_name(name: &str) -> String {
    let p = match parse_lib_name(name) {
        Ok(p) => p,
        Err(_) => return name.replace('.', "/") + ".jar",
    };
    let base = format!(
        "{}/{}/{}/{}-{}",
        p.group.replace('.', "/"),
        p.artifact,
        p.version,
        p.artifact,
        p.version
    );
    match &p.classifier {
        Some(c) => format!("{base}-{c}.jar"),
        None => format!("{base}.jar"),
    }
}

impl Library {
    pub fn is_allowed(&self, ctx: &RuleContext) -> bool {
        match &self.rules {
            Some(rules) => eval_rules(rules, ctx),
            None => true,
        }
    }

    /// Resolve the artifact to download for this library on the current platform.
    /// Returns `(artifact, natives: bool)`.
    pub fn artifact(&self) -> Option<(Artifact, bool)> {
        let downloads = self.downloads.as_ref()?;

        if let Some(natives) = &self.natives {
            let native_key = natives.get(os_rule_name())?;
            let key = native_key.replace("${arch}", arch_native());
            let classifier = downloads
                .classifiers
                .as_ref()
                .and_then(|c| c.get(&key).cloned());
            return classifier.map(|a| (a, true));
        }

        downloads.artifact.clone().map(|a| (a, false))
    }

    pub fn resolved_url(&self) -> Option<String> {
        self.artifact().map(|(a, _)| a.url)
    }
}

/// Asset index file (`assets/indexes/<name>.json`).
#[derive(Debug, Deserialize, Clone)]
pub struct AssetIndexJson {
    pub objects: HashMap<String, AssetObject>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

