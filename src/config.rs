#![allow(dead_code)]
use std::path::PathBuf;

use anyhow::Result;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

pub const ACCOUNT_TYPE_ELY_OAUTH: &str = "ely_oauth";
pub const ACCOUNT_TYPE_ELY_PASSWORD: &str = "ely_password";
pub const ACCOUNT_TYPE_OFFLINE: &str = "offline";

pub const LANG_ID: &str = "id";
pub const LANG_EN: &str = "en";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Account {
    /// Player UUID, with dashes.
    pub uuid: String,
    pub username: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub client_token: Option<String>,
    /// One of `ACCOUNT_TYPE_*`.
    pub account_type: String,
    /// Unix timestamp when the access token expires (seconds).
    pub expires_at: Option<i64>,
}

impl Account {
    pub fn uuid_no_dashes(&self) -> String {
        self.uuid.replace('-', "")
    }

    pub fn is_ely(&self) -> bool {
        self.account_type == ACCOUNT_TYPE_ELY_OAUTH || self.account_type == ACCOUNT_TYPE_ELY_PASSWORD
    }
}

/// Key (name) reserved for the built-in "latest stable release" instance.
pub const LATEST_INSTANCE_KEY: &str = "latest";

fn default_true() -> bool {
    true
}

/// Which font is used to render the whole launcher UI.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FontMode {
    /// The pixel-art "Monogram" font (looks like the bundled bitmap font).
    Monogram,
    /// The operating system default font.
    #[default]
    System,
}

/// A playable instance (like the Minecraft Launcher installations).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Instance {
    pub name: String,
    pub version_id: Option<String>,
    #[serde(default)]
    pub is_latest: bool,
    pub memory_mb: u32,
    pub java_path: Option<String>,
    pub download_java: bool,
    pub width: u32,
    pub height: u32,
    pub extra_jvm_args: String,
    pub authlib_url: String,
    /// Whether the instance uses the default game directory (the original
    /// `~/.minecraft` folder) instead of a custom one.
    #[serde(default = "default_true")]
    pub use_default_directory: bool,
    /// Custom game directory path (used when `use_default_directory` is false).
    #[serde(default)]
    pub game_dir: Option<String>,
}

impl Instance {
    pub fn new(name: String) -> Self {
        Self {
            name,
            version_id: None,
            is_latest: false,
            memory_mb: 2048,
            java_path: None,
            download_java: true,
            width: 854,
            height: 480,
            extra_jvm_args: String::new(),
            authlib_url: "ely.by".to_string(),
            use_default_directory: true,
            game_dir: None,
        }
    }

    /// The built-in instance that always resolves to the newest stable release.
    pub fn latest() -> Self {
        Self {
            name: LATEST_INSTANCE_KEY.to_string(),
            is_latest: true,
            ..Self::new(LATEST_INSTANCE_KEY.to_string())
        }
    }

    pub fn is_editable(&self) -> bool {
        !self.is_latest
    }

    /// The single directory that holds everything for this instance: the
    /// Minecraft data (versions/, libraries/, assets/, runtime/) and the game
    /// files (saves, servers, mods, config). "Use default directory" means the
    /// original `~/.minecraft` folder (like the official Minecraft launcher);
    /// otherwise the custom path is used.
    pub fn instance_dir(&self) -> PathBuf {
        if self.use_default_directory {
            original_minecraft_dir()
        } else {
            self.game_dir
                .as_deref()
                .map(PathBuf::from)
                .filter(|p| !p.as_os_str().is_empty())
                .map(writable_dir)
                .unwrap_or_else(original_minecraft_dir)
        }
    }

    /// Whether the game directory can be deleted together with the instance.
    /// The default `.minecraft` folder is shared and never deleted.
    pub fn game_dir_deletable(&self) -> bool {
        !self.use_default_directory
            && self
                .game_dir
                .as_deref()
                .map(|p| !p.is_empty())
                .unwrap_or(false)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    pub accounts: Vec<Account>,
    #[serde(default)]
    pub active_account_index: Option<usize>,

    #[serde(default)]
    pub instances: Vec<Instance>,
    #[serde(default)]
    pub active_instance: Option<String>,

    // When false only stable "release" versions are shown in the UI.
    #[serde(default)]
    pub show_all_versions: bool,

    // Runtime branding (Legacy Launcher style, set from `--brand` / `--channel`).
    #[serde(skip)]
    pub brand: String,
    #[serde(skip)]
    pub channel: String,

    // Defaults used when creating a new instance.
    #[serde(default = "default_memory")]
    pub default_memory_mb: u32,
    #[serde(default = "default_width")]
    pub default_width: u32,
    #[serde(default = "default_height")]
    pub default_height: u32,
    #[serde(default)]
    pub default_extra_jvm_args: String,
    #[serde(default = "default_authlib")]
    pub default_authlib_url: String,
    #[serde(default = "default_dl_java")]
    pub default_download_java: bool,

    // UI language: "id" (default) or "en".
    #[serde(default = "default_lang")]
    pub language: String,

    // Which font is used to render the launcher UI.
    #[serde(default)]
    pub font_mode: FontMode,

    // When true the game is told the launcher is a vanilla one
    // (tlauncher.bootstrap.brand/channel = "vanilla") instead of the
    // brand/channel given via --brand / --channel.
    #[serde(default)]
    pub vanilla_branding: bool,
}

fn default_memory() -> u32 {
    2048
}
fn default_width() -> u32 {
    854
}
fn default_height() -> u32 {
    480
}
fn default_authlib() -> String {
    "ely.by".to_string()
}
fn default_dl_java() -> bool {
    true
}
fn default_lang() -> String {
    LANG_ID.to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            accounts: Vec::new(),
            active_account_index: None,
            instances: Vec::new(),
            active_instance: None,
            show_all_versions: false,
            brand: crate::util::DEFAULT_BRAND.to_string(),
            channel: crate::util::DEFAULT_CHANNEL.to_string(),
            default_memory_mb: default_memory(),
            default_width: default_width(),
            default_height: default_height(),
            default_extra_jvm_args: String::new(),
            default_authlib_url: default_authlib(),
            default_download_java: default_dl_java(),
            language: default_lang(),
            font_mode: FontMode::Monogram,
            vanilla_branding: false,
        }
    }
}

impl Config {
    pub fn active_account(&self) -> Option<&Account> {
        self.active_account_index.and_then(|i| self.accounts.get(i))
    }

    pub fn active_account_mut(&mut self) -> Option<&mut Account> {
        self.active_account_index.and_then(|i| self.accounts.get_mut(i))
    }

    pub fn active_instance(&self) -> Option<&Instance> {
        self.active_instance
            .as_ref()
            .and_then(|n| self.instances.iter().find(|i| &i.name == n))
    }

    pub fn active_instance_mut(&mut self) -> Option<&mut Instance> {
        self.active_instance
            .as_ref()
            .and_then(|n| self.instances.iter_mut().find(|i| &i.name == n))
    }

    /// Pick a valid active instance (or none if there are no instances at all).
    pub fn normalize_active_instance(&mut self) {
        if self.active_instance.is_none()
            || self
                .active_instance
                .as_ref()
                .map_or(true, |n| !self.instances.iter().any(|i| &i.name == n))
        {
            self.active_instance = self.instances.first().map(|i| i.name.clone());
        }
    }

    /// Make sure the built-in "latest" instance exists (at index 0).
    /// Returns true if it was newly inserted (and activates it for first-run/upgrade).
    pub fn ensure_latest(&mut self) -> bool {
        if self.instances.iter().any(|i| i.is_latest) {
            return false;
        }
        let mut latest = Instance::latest();
        latest.memory_mb = self.default_memory_mb;
        latest.java_path = self.instances.first().and_then(|i| i.java_path.clone());
        latest.download_java = self.default_download_java;
        latest.width = self.default_width;
        latest.height = self.default_height;
        latest.extra_jvm_args = self.default_extra_jvm_args.clone();
        latest.authlib_url = self.default_authlib_url.clone();
        self.instances.insert(0, latest);
        self.active_instance = Some(LATEST_INSTANCE_KEY.to_string());
        true
    }

    /// The currently selected language.
    pub fn t(&self) -> &'static crate::lang::Lang {
        crate::lang::Lang::for_code(&self.language)
    }

    /// Brand reported to the game (Legacy Launcher style). "vanilla" when the
    /// user enabled vanilla branding in Settings.
    pub fn effective_brand(&self) -> &str {
        if self.vanilla_branding {
            "vanilla"
        } else {
            &self.brand
        }
    }

    /// Channel reported to the game; "vanilla" when vanilla branding is on.
    pub fn effective_channel(&self) -> &str {
        if self.vanilla_branding {
            "vanilla"
        } else {
            &self.channel
        }
    }

    /// The launcher's own data root (the folder next to the exe, or AppData
    /// when the exe folder is not writable). Used for launcher-wide data
    /// (logs) and as the default for instances that use "Launcher" mode.
    pub fn launcher_root() -> PathBuf {
        minecraft_root().unwrap_or_else(|_| PathBuf::from("."))
    }
}

pub fn data_root() -> anyhow::Result<PathBuf> {
    let dirs = ProjectDirs::from("com", "Hana", "HanaLauncher")
        .ok_or_else(|| anyhow::anyhow!(crate::lang::current().no_data_dir))?;
    Ok(dirs.data_dir().to_path_buf())
}

pub fn config_path() -> Result<PathBuf> {
    Ok(data_root()?.join("config.json"))
}

/// Portable launcher layout: the Minecraft data (versions/, libraries/,
/// assets/, runtime/, game folders and logs/) lives next to the executable,
/// so the installation folder looks like a normal launcher. If the exe folder
/// is not writable (e.g. Program Files without admin rights) the data root
/// falls back to AppData, otherwise version installs fail with
/// "Access is denied (os error 5)".
pub fn minecraft_root() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!(crate::lang::current().no_data_dir))?;
    let dir = dir.to_path_buf();
    let probe = dir.join(".hana-write-test");
    if std::fs::write(&probe, b"").is_ok() {
        let _ = std::fs::remove_file(&probe);
        Ok(dir)
    } else {
        data_root()
    }
}

/// Probe whether `dir` is writable. If it is not (e.g. a protected folder),
/// fall back to the portable launcher root (or AppData) so version installs
/// never fail with "Access is denied".
fn writable_dir(dir: PathBuf) -> PathBuf {
    let probe = dir.join(".hana-write-test");
    if std::fs::write(&probe, b"").is_ok() {
        let _ = std::fs::remove_file(&probe);
        dir
    } else {
        minecraft_root().unwrap_or(dir)
    }
}

/// The folder used by the official Minecraft launcher (`~/.minecraft`).
pub fn original_minecraft_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join(".minecraft");
        }
        if let Some(userprofile) = std::env::var_os("USERPROFILE") {
            return PathBuf::from(userprofile).join(".minecraft");
        }
        PathBuf::from(".minecraft")
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        home.join("Library").join("Application Support").join("minecraft")
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        home.join(".minecraft")
    }
}

pub fn load_config() -> Config {
    let path = match config_path() {
        Ok(p) => p,
        Err(_) => return Config::default(),
    };
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let legacy = serde_json::from_str::<serde_json::Value>(&text).ok();
    let mut cfg = match serde_json::from_str::<Config>(&text) {
        Ok(c) => c,
        Err(_) => Config::default(),
    };
    cfg.migrate(legacy.as_ref());
    cfg.migrate_instance_dirs(legacy.as_ref());
    cfg.ensure_latest();
    cfg.normalize_active_instance();
    let _ = save_config(&cfg);
    cfg
}

impl Config {
    /// Migrate legacy single-version settings into a "Bawaan" instance.
    fn migrate(&mut self, legacy: Option<&serde_json::Value>) {
        // Fresh install: nothing to migrate, `ensure_latest` builds the instance list.
        let Some(legacy) = legacy else {
            return;
        };
        if !self.instances.is_empty() {
            return;
        }
        let get = |key: &str| legacy.get(key).cloned();
        let s = |key: &str| get(key).and_then(|v| v.as_str().map(str::to_string));
        let n = |key: &str| get(key).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let b = |key: &str| get(key).and_then(|v| v.as_bool()).unwrap_or(true);

        let mut inst = Instance::new("Bawaan".to_string());
        inst.version_id = s("selected_version");
        inst.memory_mb = n("memory_mb");
        if inst.memory_mb == 0 {
            inst.memory_mb = 2048;
        }
        inst.java_path = s("java_path");
        inst.download_java = b("download_java");
        inst.width = n("width");
        if inst.width == 0 {
            inst.width = 854;
        }
        inst.height = n("height");
        if inst.height == 0 {
            inst.height = 480;
        }
        inst.extra_jvm_args = s("extra_jvm_args").unwrap_or_default();
        inst.authlib_url = s("authlib_url").unwrap_or_else(|| "ely.by".to_string());

        self.instances = vec![inst];
        self.active_instance = Some("Bawaan".to_string());
        let _ = save_config(self);
    }

    /// Port the old separate game/data folder settings onto the single
    /// "Game Directory" setting. Old configs used per-instance (and, even
    /// older, global) `data_dir_mode`/`game_dir_mode` with `Launcher`,
    /// `Original` or `Custom`; `Custom` keeps its path, everything else uses
    /// the default `.minecraft` directory.
    fn migrate_instance_dirs(&mut self, legacy: Option<&serde_json::Value>) {
        let Some(legacy) = legacy else {
            return;
        };
        let global_mode = legacy
            .get("data_dir_mode")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let global_dir = legacy
            .get("data_dir")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let legacy_insts = legacy.get("instances").and_then(|v| v.as_array());
        for (i, inst) in self.instances.iter_mut().enumerate() {
            if !inst.use_default_directory || inst.game_dir.is_some() {
                continue;
            }
            let li = legacy_insts.and_then(|a| a.get(i)).and_then(|v| v.as_object());
            let li_s = |key: &str| {
                li.and_then(|o| o.get(key))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            };
            let mode = li_s("data_dir_mode")
                .or_else(|| li_s("game_dir_mode"))
                .or(global_mode.clone());
            let dir = li_s("data_dir")
                .or_else(|| li_s("game_dir"))
                .or(global_dir.clone());
            match mode.as_deref() {
                Some("custom") => {
                    if let Some(d) = dir.filter(|d| !d.is_empty()) {
                        inst.use_default_directory = false;
                        inst.game_dir = Some(d);
                    }
                }
                _ => {
                    inst.use_default_directory = true;
                    inst.game_dir = None;
                }
            }
        }
    }
}

pub fn save_config(cfg: &Config) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(cfg)?;
    std::fs::write(&path, text)?;
    Ok(())
}