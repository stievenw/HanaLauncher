mod ui;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel as new_channel, Receiver, Sender};
use std::time::Instant;

use eframe::egui::{self, TextureHandle};

use crate::config::{save_config, Account, Config, Installation};
use crate::minecraft::ManifestVersion;
use crate::worker::{LaunchDecision, LaunchRequest, TaskEvent};

#[allow(dead_code)]
#[cfg(windows)]
mod win_probe {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    static DESCENDANTS: Mutex<Vec<u32>> = Mutex::new(Vec::new());
    static PROBE_FOUND: AtomicBool = AtomicBool::new(false);

    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;

    #[repr(C)]
    struct ProcessEntry32W {
        dw_size: u32,
        cnt_usage: u32,
        th32_process_id: u32,
        th32_default_heap_id: usize,
        th32_module_id: u32,
        cnt_threads: u32,
        th32_parent_process_id: u32,
        pc_pri_class_base: i32,
        dw_flags: u32,
        sz_exe_file: [u16; 260],
    }

    #[allow(non_snake_case)]
    unsafe extern "system" {
        fn EnumWindows(
            lpEnumFunc: Option<unsafe extern "system" fn(isize, isize) -> i32>,
            lParam: isize,
        ) -> i32;
        fn GetWindowThreadProcessId(hwnd: *const std::ffi::c_void, pid: *mut u32) -> u32;
        fn IsWindowVisible(hwnd: *const std::ffi::c_void) -> i32;
        fn CreateToolhelp32Snapshot(dwFlags: u32, th32ProcessID: u32) -> isize;
        fn Process32FirstW(hSnapshot: isize, lppe: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(hSnapshot: isize, lppe: *mut ProcessEntry32W) -> i32;
        fn CloseHandle(hObject: isize) -> i32;
    }

    fn collect_descendants(root: u32) -> Vec<u32> {
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == -1 {
                return vec![root];
            }
            let mut e: ProcessEntry32W = std::mem::zeroed();
            e.dw_size = std::mem::size_of::<ProcessEntry32W>() as u32;
            let mut any = false;
            if Process32FirstW(snap, &mut e) != 0 {
                any = true;
                loop {
                    children
                        .entry(e.th32_parent_process_id)
                        .or_default()
                        .push(e.th32_process_id);
                    if Process32NextW(snap, &mut e) == 0 {
                        break;
                    }
                }
            }
            CloseHandle(snap);
            if !any {
                return vec![root];
            }
        }
        let mut out = Vec::new();
        let mut stack = vec![root];
        while let Some(p) = stack.pop() {
            if out.contains(&p) {
                continue;
            }
            out.push(p);
            if let Some(kids) = children.get(&p) {
                stack.extend(kids.iter().copied());
            }
        }
        out
    }

    unsafe extern "system" fn enum_proc(hwnd: isize, _lparam: isize) -> i32 {
        unsafe {
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd as *const std::ffi::c_void, &mut pid);
            let is_game_window =
                DESCENDANTS.lock().map(|d| d.contains(&pid)).unwrap_or(false);
            if is_game_window
                && IsWindowVisible(hwnd as *const std::ffi::c_void) != 0
            {
                PROBE_FOUND.store(true, Ordering::SeqCst);
                return 0;
            }
        }
        1
    }

    pub fn game_has_visible_window(pid: u32) -> bool {
        if let Ok(mut d) = DESCENDANTS.lock() {
            *d = collect_descendants(pid);
        }
        PROBE_FOUND.store(false, Ordering::SeqCst);
        unsafe {
            EnumWindows(Some(enum_proc), 0);
        }
        PROBE_FOUND.load(Ordering::SeqCst)
    }
}

// ── Types ───────────────────────────────────────────────────────────────────

#[derive(PartialEq, Clone, Copy)]
pub(crate) enum Page {
    Home,
    Versions,
    Installations,
    Accounts,
    Settings,
    Console,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ToastKind {
    Info,
    Ok,
    Error,
}

pub(crate) struct Toast {
    pub msg: String,
    pub kind: ToastKind,
    pub born: Instant,
}

pub(crate) enum VersionAction {
    Select(String),
    Install(String),
    Repair(String),
    DeleteData(String),
}

/// Draft state for the "delete installation" confirmation dialog.
pub(crate) struct DeleteDraft {
    pub idx: usize,
    pub name: String,
    pub also_folder: bool,
    pub deletable: bool,
}

/// Draft state for the "create / edit installation" dialog.
pub(crate) struct InstallationDraft {
    pub editing: Option<usize>,
    pub is_latest: bool,
    pub name: String,
    pub version_id: Option<String>,
    pub version_search: String,
    pub memory_mb: u32,
    pub java_path: Option<String>,
    pub download_java: bool,
    pub width: u32,
    pub height: u32,
    pub extra_jvm_args: String,
    pub authlib_url: String,
}

impl InstallationDraft {
    pub fn new() -> Self {
        Self {
            editing: None,
            is_latest: false,
            name: String::new(),
            version_id: None,
            version_search: String::new(),
            memory_mb: 2048,
            java_path: None,
            download_java: true,
            width: 854,
            height: 480,
            extra_jvm_args: String::new(),
            authlib_url: "ely.by".to_string(),
        }
    }

    pub fn from_installation(idx: usize, inst: &Installation) -> Self {
        Self {
            editing: Some(idx),
            is_latest: inst.is_latest,
            name: inst.name.clone(),
            version_id: inst.version_id.clone(),
            version_search: String::new(),
            memory_mb: inst.memory_mb,
            java_path: inst.java_path.clone(),
            download_java: inst.download_java,
            width: inst.width,
            height: inst.height,
            extra_jvm_args: inst.extra_jvm_args.clone(),
            authlib_url: inst.authlib_url.clone(),
        }
    }
}

// ── HanaApp ─────────────────────────────────────────────────────────────────

pub struct HanaApp {
    pub(crate) cfg: Config,
    pub(crate) tx: Sender<TaskEvent>,
    pub(crate) rx: Receiver<TaskEvent>,
    pub(crate) root: PathBuf,

    pub(crate) versions: Vec<ManifestVersion>,
    pub(crate) versions_loaded: bool,
    pub(crate) latest_release: Option<String>,
    pub(crate) installed: HashSet<String>,
    pub(crate) search: String,
    pub(crate) versions_tab_downloaded: bool,

    pub(crate) task_active: bool,
    pub(crate) task_stage: String,
    pub(crate) task_progress: Option<(u64, u64)>,

    pub(crate) page: Page,
    pub(crate) running_pid: Option<u32>,
    pub(crate) console: String,
    pub(crate) console_scroll: bool,

    pub(crate) avatars: HashMap<String, TextureHandle>,
    pub(crate) toasts: Vec<Toast>,

    pub(crate) pw_username: String,
    pub(crate) pw_password: String,
    pub(crate) pw_show: bool,
    pub(crate) need_2fa: bool,
    pub(crate) twofa_input: String,
    pub(crate) device_code: Option<(String, String)>,
    pub(crate) offline_name: String,

    pub(crate) installation_dialog: Option<InstallationDraft>,
    pub(crate) delete_dialog: Option<DeleteDraft>,
    pub(crate) version_delete_confirm: Option<String>,
    pub(crate) warn_existing: bool,

    pub(crate) version_choice: Option<(String, String)>,
    pub(crate) decision_tx: Option<Sender<LaunchDecision>>,

    pub(crate) bg: Option<egui::TextureHandle>,
    pub(crate) logo: Option<egui::TextureHandle>,
}

impl HanaApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        brand: String,
        channel: String,
        warn_existing: bool,
    ) -> Self {
        let ctx = &cc.egui_ctx;

        let mut visuals = egui::Visuals::light();
        visuals.panel_fill = ui::BG_BOTTOM;
        visuals.extreme_bg_color = ui::BG_BOTTOM;
        visuals.faint_bg_color = egui::Color32::from_rgb(255, 244, 228);
        visuals.window_fill = ui::GLASS;
        visuals.window_stroke = egui::Stroke::new(1.0, ui::BORDER);
        visuals.window_corner_radius = egui::CornerRadius::same(10);
        visuals.window_shadow = egui::epaint::Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: egui::Color32::from_black_alpha(45),
        };
        visuals.popup_shadow = visuals.window_shadow.clone();
        visuals.selection.bg_fill = ui::ACCENT;
        visuals.selection.stroke.color = ui::TEXT;
        visuals.code_bg_color = egui::Color32::from_rgb(255, 244, 228);
        visuals.hyperlink_color = ui::ACCENT;
        visuals.text_cursor.stroke.color = ui::ACCENT;
        visuals.widgets.noninteractive.bg_fill = egui::Color32::TRANSPARENT;
        visuals.widgets.noninteractive.fg_stroke.color = ui::TEXT;
        visuals.widgets.inactive.bg_fill = ui::GLASS;
        visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(255, 244, 228);
        visuals.widgets.inactive.fg_stroke.color = ui::TEXT;
        visuals.widgets.hovered.bg_fill = ui::HOVER;
        visuals.widgets.hovered.weak_bg_fill = ui::HOVER;
        visuals.widgets.active.bg_fill = ui::PRIMARY_BTN;
        visuals.widgets.active.fg_stroke.color = ui::PRIMARY_TEXT;
        visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);
        visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);
        visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);
        visuals.override_text_color = Some(ui::TEXT);
        visuals.widgets.open.fg_stroke.color = ui::TEXT;
        visuals.widgets.open.bg_fill = ui::GLASS_SOLID;
        visuals.widgets.open.weak_bg_fill = ui::GLASS_SOLID;

        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let mut style = (*ctx.style_of(theme)).clone();
            style.spacing.item_spacing = egui::Vec2::new(6.0, 6.0);
            style.spacing.button_padding = egui::Vec2::new(9.0, 4.0);
            style.visuals = visuals.clone();
            ctx.set_style_of(theme, style);
        }
        ctx.set_visuals(visuals);

        let bg = crate::assets::background_jpg()
            .and_then(|bytes| image::load_from_memory(&bytes).ok())
            .map(|img| {
                let img = img.to_rgba8();
                let (w, h) = img.dimensions();
                let img = if w.max(h) > 1600 {
                    let scale = 1600.0 / (w.max(h) as f32);
                    let nw = ((w as f32 * scale) as u32).max(1);
                    let nh = ((h as f32 * scale) as u32).max(1);
                    image::imageops::resize(
                        &img,
                        nw,
                        nh,
                        image::imageops::FilterType::Triangle,
                    )
                } else {
                    img
                };
                let (w, h) = img.dimensions();
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [w as usize, h as usize],
                    img.as_raw(),
                );
                ctx.load_texture(
                    "bg_sunflowers",
                    color_image,
                    egui::TextureOptions::LINEAR,
                )
            });

        let icon_png = crate::assets::icon_png();
        if let Some(bytes) = &icon_png {
            if let Ok(icon) = image::load_from_memory(bytes) {
                let rgba = icon.to_rgba8();
                let (w, h) = rgba.dimensions();
                ctx.send_viewport_cmd(egui::ViewportCommand::Icon(Some(
                    std::sync::Arc::new(egui::IconData {
                        rgba: rgba.into_raw(),
                        width: w,
                        height: h,
                    }),
                )));
            }
        }
        let logo = icon_png
            .as_deref()
            .and_then(|bytes| image::load_from_memory(bytes).ok())
            .map(|icon| {
                let rgba = icon.to_rgba8();
                let (w, h) = rgba.dimensions();
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [w as usize, h as usize],
                    rgba.as_raw(),
                );
                ctx.load_texture(
                    "logo_sunflower",
                    color_image,
                    egui::TextureOptions::LINEAR,
                )
            });

        let mut cfg = crate::config::load_config();
        crate::lang::set_current(cfg.t());
        cfg.brand = brand;
        cfg.channel = channel;
        ui::apply_font_mode(ctx, &cfg.font_mode);

        let root = crate::config::Config::launcher_root();
        for sub in ["versions", "libraries", "assets", "runtime", "logs"] {
            let _ = std::fs::create_dir_all(root.join(sub));
        }
        let _ = std::fs::create_dir_all(&root);

        let (tx, rx) = new_channel();

        let mut app = Self {
            cfg,
            tx,
            rx,
            root,
            versions: Vec::new(),
            versions_loaded: false,
            latest_release: None,
            installed: HashSet::new(),
            search: String::new(),
            versions_tab_downloaded: false,
            task_active: false,
            task_stage: String::new(),
            task_progress: None,
            page: Page::Home,
            running_pid: None,
            console: String::new(),
            console_scroll: true,
            avatars: HashMap::new(),
            toasts: Vec::new(),
            pw_username: String::new(),
            pw_password: String::new(),
            pw_show: false,
            need_2fa: false,
            twofa_input: String::new(),
            device_code: None,
            offline_name: String::new(),

            installation_dialog: None,
            delete_dialog: None,
            version_delete_confirm: None,
            warn_existing,
            version_choice: None,
            decision_tx: None,
            bg,
            logo,
        };
        app.refresh_installed();

        if let Some(acc) = app.cfg.active_account() {
            let uuid = acc.uuid.clone();
            let (_, drx) = new_channel::<LaunchDecision>();
            crate::tasks::fetch_avatar(app.tx.clone(), drx, uuid);
        }

        app
    }

    // ── Events ──────────────────────────────────────────────────────────────

    pub(crate) fn drain_events(&mut self, ctx: &egui::Context) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                TaskEvent::Progress { stage, current, total } => {
                    self.task_stage = stage;
                    self.task_progress = Some((current, total));
                }
                TaskEvent::Log(msg) => {
                    self.push_console(&msg);
                    self.task_stage = msg.clone();
                }
                TaskEvent::Error(msg) => {
                    self.task_active = false;
                    self.task_progress = None;
                    self.device_code = None;
                    self.refresh_installed();
                    self.toast(ToastKind::Error, msg.clone());
                    self.push_console(&format!("[ERROR] {msg}"));
                }
                TaskEvent::Done(_summary) => {
                    self.task_active = false;
                    self.task_progress = None;
                    self.refresh_installed();
                }
                TaskEvent::AccountAdded(account) => {
                    self.device_code = None;
                    self.add_or_replace_account(*account);
                }
                TaskEvent::AvatarReady { uuid, width, height, rgba } => {
                    let img = egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba,
                    );
                    let tex = ctx.load_texture(
                        format!("avatar-{uuid}"),
                        img,
                        egui::TextureOptions::LINEAR,
                    );
                    self.avatars.insert(uuid, tex);
                }
                TaskEvent::VersionList { latest, versions } => {
                    let had_selection = self
                        .cfg
                        .active_installation()
                        .and_then(|i| i.version_id.clone());
                    self.latest_release = latest;
                    self.versions = versions;
                    self.versions_loaded = true;
                    self.refresh_installed();
                    if had_selection.is_none() {
                        let is_latest = self
                            .cfg
                            .active_installation()
                            .map(|i| i.is_latest)
                            .unwrap_or(true);
                        if !is_latest {
                            if let Some(id) = self
                                .versions
                                .iter()
                                .find(|v| self.version_visible(v))
                                .map(|v| v.id.clone())
                            {
                                if let Some(inst) = self.cfg.active_installation_mut() {
                                    inst.version_id = Some(id.clone());
                                }
                                let _ = save_config(&self.cfg);
                                self.push_console(
                                    &self.cfg.t().version_picked.replace("{}", &id),
                                );
                            }
                        }
                    }
                }
                TaskEvent::GameStarted(pid) => {
                    self.running_pid = Some(pid);
                    self.task_active = false;
                    self.toast(
                        ToastKind::Ok,
                        self.cfg.t().game_running.replace("{}", &pid.to_string()),
                    );
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                }
                TaskEvent::GameOutput(line) => self.push_console(&line),
                TaskEvent::GameExited(code) => {
                    self.running_pid = None;
                    self.toast(
                        if code == 0 { ToastKind::Ok } else { ToastKind::Error },
                        self.cfg.t().game_exited.replace("{}", &code.to_string()),
                    );
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                }
                TaskEvent::JavaReady(dir) => {
                    let java = dir
                        .join("bin")
                        .join(crate::java::java_binary_name());
                    if java.exists() {
                        if let Some(inst) = self.cfg.active_installation_mut() {
                            inst.java_path =
                                Some(java.to_string_lossy().into_owned());
                            inst.download_java = false;
                        }
                        self.toast(ToastKind::Ok, "Java runtime terpasang.");
                        let _ = save_config(&self.cfg);
                    }
                }
                TaskEvent::NeedsTwoFactor => {
                    self.need_2fa = true;
                    self.toast(ToastKind::Info, self.cfg.t().account_uses_2fa);
                }
                TaskEvent::NeedVersionChoice { newest, current } => {
                    self.version_choice = Some((newest, current));
                    self.toast(ToastKind::Info, self.cfg.t().update_available);
                }
                TaskEvent::DeviceCodeRequired { code, verification_uri } => {
                    self.device_code = Some((code, verification_uri));
                    self.toast(ToastKind::Info, self.cfg.t().enter_verify_code);
                }
            }
        }
    }

    // ── State helpers ────────────────────────────────────────────────────────

    pub(crate) fn installation_root(&self) -> PathBuf {
        let dir = self.cfg.launcher_dir();
        if dir.as_os_str().is_empty() {
            self.root.clone()
        } else {
            dir
        }
    }

    pub(crate) fn refresh_installed(&mut self) {
        let root = self.installation_root();
        let mut set = HashSet::new();
        let dir = root.join("versions");
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let id = e.file_name().to_string_lossy().into_owned();
                if crate::install::version_is_installed(&root, &id) {
                    set.insert(id);
                }
            }
        }
        self.installed = set;
    }

    pub(crate) fn refresh_active_root(&mut self) {
        self.refresh_installed();
    }

    pub(crate) fn add_or_replace_account(&mut self, account: Account) {
        let is_update = self
            .cfg
            .accounts
            .iter()
            .any(|a| a.uuid == account.uuid);
        if let Some(idx) = self
            .cfg
            .accounts
            .iter()
            .position(|a| a.uuid == account.uuid)
        {
            self.cfg.accounts[idx] = account;
        } else {
            self.cfg.accounts.push(account);
        }
        self.cfg.active_account_index = Some(self.cfg.accounts.len() - 1);
        let _ = save_config(&self.cfg);
        let uuid = self
            .cfg
            .active_account()
            .map(|a| a.uuid.clone())
            .unwrap_or_default();
        if !self.avatars.contains_key(&uuid) {
            let (_, drx) = new_channel::<LaunchDecision>();
            crate::tasks::fetch_avatar(self.tx.clone(), drx, uuid);
        }
        let msg = if is_update {
            self.cfg.t().token_updated.replace(
                "{}",
                &self
                    .cfg
                    .active_account()
                    .map(|a| a.username.clone())
                    .unwrap_or_default(),
            )
        } else {
            self.cfg.t().account_added.to_string()
        };
        self.toast(ToastKind::Ok, msg);
    }

    pub(crate) fn remove_account(&mut self, idx: usize) {
        if idx >= self.cfg.accounts.len() {
            return;
        }
        self.cfg.accounts.remove(idx);
        if let Some(active) = self.cfg.active_account_index {
            if active >= self.cfg.accounts.len() {
                self.cfg.active_account_index =
                    if self.cfg.accounts.is_empty() { None } else { Some(0) };
            }
        }
        let _ = save_config(&self.cfg);
        self.toast(ToastKind::Ok, self.cfg.t().account_removed);
    }

    pub(crate) fn toast(&mut self, kind: ToastKind, msg: impl Into<String>) {
        self.toasts.push(Toast {
            msg: msg.into(),
            kind,
            born: Instant::now(),
        });
        if self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
    }

    fn redact_sensitive(&self, line: &str) -> String {
        let mut out = line.to_string();
        for acc in &self.cfg.accounts {
            if !acc.access_token.is_empty() {
                out = out.replace(&acc.access_token, "****");
            }
            if let Some(rt) = &acc.refresh_token {
                if !rt.is_empty() {
                    out = out.replace(rt, "****");
                }
            }
        }
        out
    }

    pub(crate) fn push_console(&mut self, line: &str) {
        let redacted = self.redact_sensitive(line);
        self.console.push_str(&redacted);
        self.console.push('\n');
        if self.console.len() > 400_000 {
            self.console = self.console.split_off(self.console.len() - 300_000);
        }
        self.console_scroll = true;
    }

    pub(crate) fn busy(&self) -> bool {
        self.task_active || self.game_running()
    }

    pub(crate) fn start(
        &mut self,
        task: impl FnOnce(Sender<TaskEvent>, Receiver<LaunchDecision>) + Send + 'static,
    ) {
        if self.task_active {
            self.toast(ToastKind::Error, self.cfg.t().task_busy);
            return;
        }
        self.task_active = true;
        self.task_stage = self.cfg.t().starting.to_string();
        self.task_progress = None;
        let tx = self.tx.clone();
        let (decision_tx, decision_rx) = new_channel::<LaunchDecision>();
        self.decision_tx = Some(decision_tx);
        std::thread::spawn(move || task(tx, decision_rx));
    }

    pub(crate) fn game_running(&self) -> bool {
        self.running_pid.is_some()
    }

    pub(crate) fn play_clicked(&mut self) {
        let t = self.cfg.t();
        if self.game_running() {
            if let Some(pid) = self.running_pid {
                crate::tasks::kill_game(pid);
                self.running_pid = None;
                self.toast(ToastKind::Info, t.stop_game);
            }
            return;
        }
        let Some(inst) = self.cfg.active_installation().cloned() else {
            self.toast(ToastKind::Error, t.no_installation_selected);
            return;
        };
        let Some(version_id) = self.installation_resolved_version(&inst) else {
            self.toast(ToastKind::Error, t.version_unavailable);
            return;
        };

        if !inst.is_latest && !self.installed.contains(&version_id) {
            self.toast(ToastKind::Info, t.installing_version);
            let root = self.cfg.launcher_dir();
            self.start(move |tx, drx| {
                crate::tasks::install_version(tx, drx, version_id, root)
            });
            return;
        }

        let Some(account) = self.cfg.active_account().cloned() else {
            self.toast(ToastKind::Error, t.login_required);
            return;
        };
        if account.account_type == crate::config::ACCOUNT_TYPE_OFFLINE
            && !self.has_valid_online()
        {
            self.toast(ToastKind::Error, t.offline_locked);
            return;
        }
        let req = LaunchRequest {
            config: self.cfg.clone(),
            account,
            version: Some(version_id.clone()),
        };
        let root = self.cfg.launcher_dir();
        self.start(move |tx, drx| {
            crate::tasks::launch_game(tx, drx, req, root);
        });
    }

    pub(crate) fn start_refresh_versions(&mut self) {
        self.versions_loaded = true;
        self.start(|tx, drx| crate::tasks::refresh_versions(tx, drx));
    }

    pub(crate) fn active_version_installed(&self) -> bool {
        self.active_resolved_version()
            .as_ref()
            .map(|id| self.installed.contains(id))
            .unwrap_or(false)
    }

    pub(crate) fn latest_stable(&self) -> Option<String> {
        if let Some(id) = &self.latest_release {
            return Some(id.clone());
        }
        self.versions
            .iter()
            .filter(|v| v.kind == "release")
            .max_by(|a, b| a.release_time.cmp(&b.release_time))
            .map(|v| v.id.clone())
    }

    pub(crate) fn installation_resolved_version(&self, inst: &Installation) -> Option<String> {
        if inst.is_latest {
            self.latest_stable()
        } else {
            inst.version_id.clone()
        }
    }

    pub(crate) fn active_resolved_version(&self) -> Option<String> {
        self.cfg
            .active_installation()
            .and_then(|i| self.installation_resolved_version(i))
    }

    pub(crate) fn installation_display_name(&self, inst: &Installation) -> String {
        if inst.is_latest {
            self.cfg.t().latest_installation.to_string()
        } else {
            inst.name.clone()
        }
    }

    pub(crate) fn version_visible(&self, v: &ManifestVersion) -> bool {
        self.cfg.show_all_versions || v.kind == "release"
    }

    pub(crate) fn visible_versions(&self) -> Vec<&ManifestVersion> {
        self.versions
            .iter()
            .filter(|v| self.version_visible(v))
            .collect()
    }

    pub(crate) fn manifest_version(&self, id: &str) -> Option<&ManifestVersion> {
        self.versions.iter().find(|v| v.id == id)
    }

    pub(crate) fn display_version_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for v in self.visible_versions() {
            ids.push(v.id.clone());
            seen.insert(v.id.clone());
        }
        let mut installed: Vec<String> = self.installed.iter().cloned().collect();
        installed.sort();
        for id in installed {
            if seen.insert(id.clone()) {
                ids.push(id);
            }
        }
        ids
    }

    pub(crate) fn has_valid_online(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.cfg.accounts.iter().any(|a| {
            a.account_type != crate::config::ACCOUNT_TYPE_OFFLINE
                && a.expires_at.map(|e| e > now).unwrap_or(true)
        })
    }

    pub(crate) fn update_toasts(&mut self) {
        let now = Instant::now();
        self.toasts.retain(|t| now.duration_since(t.born).as_secs() < 5);
    }

    // ── Drawing (delegates to ui module) ─────────────────────────────────────

    fn draw(&mut self, root_ui: &mut egui::Ui) {
        ui::draw(self, root_ui);
    }
}

// ── eframe::App ─────────────────────────────────────────────────────────────

impl eframe::App for HanaApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events(ctx);
        if !self.versions_loaded && !self.task_active {
            self.start_refresh_versions();
        }
        self.update_toasts();
        if self.task_active || self.running_pid.is_some() || !self.toasts.is_empty() {
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}
