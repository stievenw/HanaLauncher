use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel as new_channel, Receiver, Sender};
use std::time::Instant;

use eframe::egui::{
    self, Align2, Color32, RichText, TextureHandle, TextureOptions, Vec2,
};

use crate::config::{save_config, Account, Config, Instance};
use crate::minecraft::ManifestVersion;
use crate::worker::{LaunchDecision, LaunchRequest, TaskEvent};

/// Switch the whole launcher between the Monogram pixel font and the OS default.
/// The TTF files ship in the external `resources/` folder; when they are missing
/// the OS default is used regardless of the selected mode.
fn apply_font_mode(ctx: &egui::Context, mode: &crate::config::FontMode) {
    let mut fonts = egui::FontDefinitions::default();
    if *mode == crate::config::FontMode::Monogram {
        let regular = crate::assets::monogram_ttf();
        let italic = crate::assets::monogram_italic_ttf();
        if let (Some(regular), Some(italic)) = (regular, italic) {
            fonts.font_data.insert(
                "monogram".to_owned(),
                egui::FontData::from_owned(regular).into(),
            );
            fonts.font_data.insert(
                "monogram_italic".to_owned(),
                egui::FontData::from_owned(italic).into(),
            );
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                if let Some(list) = fonts.families.get_mut(&family) {
                    list.insert(0, "monogram_italic".to_owned());
                    list.insert(0, "monogram".to_owned());
                }
            }
        }
    }
    ctx.set_fonts(fonts);
    ctx.request_repaint();
}

/// Detects whether a launched Minecraft process owns a visible window by
/// walking the whole descendant process tree (the Java shim spawns the real
/// JVM as a child). Kept for diagnostics; the launcher now hides immediately.
#[cfg(windows)]
#[allow(dead_code)]
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

    /// Every descendant pid of `root`, including `root` itself. The game is
    /// spawned through `javapath\java.exe` (or another launcher shim) which
    /// starts the real JVM as a child process, so the Minecraft window is
    /// owned by a *grandchild* pid, not by the pid the launcher spawned.
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
            let is_game_window = DESCENDANTS.lock().map(|d| d.contains(&pid)).unwrap_or(false);
            if is_game_window && IsWindowVisible(hwnd as *const std::ffi::c_void) != 0 {
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

const ACCENT: Color32 = Color32::from_rgb(255, 130, 45);
/// Subtle "selected / active" highlight: the normal glass colour, slightly
/// darkened - not the bright orange accent.
const ACCENT_SOFT: Color32 = Color32::from_rgba_premultiplied(243, 229, 206, 236);
const ACCENT_SOFT_STROKE: Color32 = Color32::from_rgb(203, 168, 122);
const PRIMARY_BTN: Color32 = Color32::from_rgb(228, 100, 22);
const PRIMARY_TEXT: Color32 = Color32::from_rgb(62, 44, 30);
const OK_GREEN: Color32 = Color32::from_rgb(46, 150, 70);
const WARN_YELLOW: Color32 = Color32::from_rgb(206, 132, 0);
const ERR_RED: Color32 = Color32::from_rgb(214, 60, 48);
const BG_TOP: Color32 = Color32::from_rgb(112, 154, 216);
const BG_BOTTOM: Color32 = Color32::from_rgb(255, 197, 122);
const GLASS: Color32 = Color32::from_rgba_premultiplied(255, 249, 240, 236);
const GLASS_NAV: Color32 = Color32::from_rgba_premultiplied(255, 249, 240, 198);
const GLASS_SOLID: Color32 = Color32::from_rgb(255, 249, 240);
const HOVER: Color32 = Color32::from_rgb(250, 227, 198);
const BORDER: Color32 = Color32::from_rgb(226, 190, 150);
const TEXT: Color32 = Color32::from_rgb(62, 44, 30);
const TEXT_WEAK: Color32 = Color32::from_rgb(142, 112, 86);

fn paint_vgrad(painter: &egui::Painter, rect: egui::Rect, top: Color32, bottom: Color32) {
    use egui::epaint::{Vertex, WHITE_UV};
    let mesh = egui::Mesh {
        vertices: vec![
            Vertex { pos: rect.left_top(), uv: WHITE_UV, color: top },
            Vertex { pos: rect.right_top(), uv: WHITE_UV, color: top },
            Vertex { pos: rect.left_bottom(), uv: WHITE_UV, color: bottom },
            Vertex { pos: rect.right_bottom(), uv: WHITE_UV, color: bottom },
        ],
        indices: vec![0, 1, 2, 2, 1, 3],
        texture_id: egui::TextureId::default(),
    };
    painter.add(egui::Shape::mesh(mesh));
}

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Home,
    Versions,
    Instances,
    Accounts,
    Settings,
    Console,
}

#[derive(Clone, Copy, PartialEq)]
enum ToastKind {
    Info,
    Ok,
    Error,
}

struct Toast {
    msg: String,
    kind: ToastKind,
    born: Instant,
}

fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::default()
        .fill(GLASS)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(10.0)
        .shadow(egui::epaint::Shadow {
            offset: [0, 2],
            blur: 12,
            spread: 0,
            color: Color32::from_black_alpha(18),
        })
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, add);
}

/// Formats a Unix timestamp as a local date-time string (`YYYY-MM-DD HH:MM`).
fn format_expiry_date(unix: i64) -> String {
    use chrono::{DateTime, Local};
    DateTime::from_timestamp(unix, 0)
        .map(|u| u.with_timezone(&Local))
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| Local::now().format("%Y-%m-%d %H:%M").to_string())
}

/// Builds a compact countdown like `2 hari 3 jam 10 mnt` for the given seconds.
fn format_time_left(secs: i64, t: &crate::lang::Lang) -> String {
    if secs <= 0 {
        return format!("0 {}", t.unit_sec);
    }
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    let parts: Vec<String> = [d, h, m, s]
        .iter()
        .zip([t.unit_day, t.unit_hour, t.unit_min, t.unit_sec])
        .filter_map(|(v, unit)| (*v > 0).then(|| format!("{} {}", v, unit)))
        .collect();
    if parts.is_empty() {
        format!("0 {}", t.unit_sec)
    } else {
        parts.join(" ")
    }
}

enum VersionAction {
    Select(String),
    Install(String),
    DeleteData(String),
}

/// Compact local timestamp used for console log filenames, e.g.
/// `2026-08-19-14-30-25`.
fn now_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (hh, mm, ss) = (rem / 3_600, (rem % 3_600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}-{hh:02}-{mm:02}-{ss:02}")
}

/// Convert days since the Unix epoch to a (year, month, day) civil date using
/// Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    (y, mo as u32, d as u32)
}

/// Turn whatever the user picked in the folder dialog into a path usable as a
/// Build the instance-specific sub folder for a custom game dir. The user
/// picks a base location; the instance gets its own folder named after it so
/// multiple instances can share one base without mixing saves.
fn pick_folder_with_instance_subdir(base: std::path::PathBuf, instance_name: &str) -> PathBuf {
    let slug = if instance_name.trim().is_empty() {
        "instance".to_string()
    } else {
        instance_name
            .trim()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>()
    };
    base.join(slug)
}

/// Open a folder in the platform file manager (Explorer on Windows).
fn open_in_explorer(path: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .is_ok()
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .is_ok()
    }
}

/// Draft state for the "delete instance" confirmation dialog.
struct DeleteDraft {
    idx: usize,
    name: String,
    /// True when the "delete folder too" checkbox is ticked.
    also_folder: bool,
    /// True when the instance's game folder may be deleted (Custom mode only).
    deletable: bool,
}

/// Draft state for the "create / edit instance" dialog.
struct InstDraft {
    editing: Option<usize>,
    is_latest: bool,
    name: String,
    version_id: Option<String>,
    version_search: String,
    memory_mb: u32,
    java_path: Option<String>,
    download_java: bool,
    width: u32,
    height: u32,
    extra_jvm_args: String,
    authlib_url: String,
    game_dir_mode: crate::config::GameDirMode,
    game_dir: Option<String>,
}

impl InstDraft {
    fn new() -> Self {
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
            game_dir_mode: crate::config::GameDirMode::Launcher,
            game_dir: None,
        }
    }

    fn from_instance(idx: usize, inst: &Instance) -> Self {
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
            game_dir_mode: inst.game_dir_mode.clone(),
            game_dir: inst.game_dir.clone(),
        }
    }
}

pub struct HanaApp {
    cfg: Config,
    tx: Sender<TaskEvent>,
    rx: Receiver<TaskEvent>,
    root: PathBuf,

    versions: Vec<ManifestVersion>,
    versions_loaded: bool,
    latest_release: Option<String>,
    installed: HashSet<String>,
    /// Custom clients found in the local versions folder (not in the manifest).
    custom_versions: Vec<ManifestVersion>,
    /// Client mods (Fabric/Quilt) state for the "custom clients" section.
    loaders_mc: String,
    fabric_loaders: Vec<crate::minecraft::LoaderMeta>,
    quilt_loaders: Vec<crate::minecraft::LoaderMeta>,
    search: String,
    versions_tab_downloaded: bool,

    task_active: bool,
    task_stage: String,
    task_progress: Option<(u64, u64)>,

    page: Page,
    running_pid: Option<u32>,
    console: String,
    console_scroll: bool,

    avatars: HashMap<String, TextureHandle>,
    toasts: Vec<Toast>,

    pw_username: String,
    pw_password: String,
    pw_show: bool,
    need_2fa: bool,
    twofa_input: String,
    device_code: Option<(String, String)>,
    offline_name: String,

    inst_dialog: Option<InstDraft>,
    delete_dialog: Option<DeleteDraft>,
    warn_existing: bool,

    version_choice: Option<(String, String)>,
    decision_tx: Option<Sender<LaunchDecision>>,

    bg: Option<egui::TextureHandle>,
    logo: Option<egui::TextureHandle>,
}

impl HanaApp {
    pub fn new(cc: &eframe::CreationContext<'_>, brand: String, channel: String, warn_existing: bool) -> Self {
        let ctx = &cc.egui_ctx;

        let mut visuals = egui::Visuals::light();
        visuals.panel_fill = BG_BOTTOM;
        visuals.extreme_bg_color = BG_BOTTOM;
        visuals.faint_bg_color = Color32::from_rgb(255, 244, 228);
        visuals.window_fill = GLASS;
        visuals.window_stroke = egui::Stroke::new(1.0, BORDER);
        visuals.window_corner_radius = egui::CornerRadius::same(10);
        visuals.window_shadow = egui::epaint::Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(45),
        };
        visuals.popup_shadow = visuals.window_shadow.clone();
        visuals.selection.bg_fill = ACCENT;
        visuals.selection.stroke.color = TEXT;
        visuals.code_bg_color = Color32::from_rgb(255, 244, 228);
        visuals.hyperlink_color = ACCENT;
        visuals.text_cursor.stroke.color = ACCENT;
        visuals.widgets.noninteractive.bg_fill = Color32::TRANSPARENT;
        visuals.widgets.noninteractive.fg_stroke.color = TEXT;
        visuals.widgets.inactive.bg_fill = GLASS;
        visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(255, 244, 228);
        visuals.widgets.inactive.fg_stroke.color = TEXT;
        visuals.widgets.hovered.bg_fill = HOVER;
        visuals.widgets.hovered.weak_bg_fill = HOVER;
        visuals.widgets.active.bg_fill = PRIMARY_BTN;
        visuals.widgets.active.fg_stroke.color = PRIMARY_TEXT;
        visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);
        visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);
        visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);
        visuals.override_text_color = Some(TEXT);
        visuals.widgets.open.fg_stroke.color = TEXT;
        visuals.widgets.open.bg_fill = GLASS_SOLID;
        visuals.widgets.open.weak_bg_fill = GLASS_SOLID;

        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let mut style = (*ctx.style_of(theme)).clone();
            style.spacing.item_spacing = Vec2::new(6.0, 6.0);
            style.spacing.button_padding = Vec2::new(9.0, 4.0);
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
                image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle)
            } else {
                img
            };
            let (w, h) = img.dimensions();
            let color_image =
                egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw());
            ctx.load_texture("bg_sunflowers", color_image, egui::TextureOptions::LINEAR)
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
            let color_image =
                egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba.as_raw());
            ctx.load_texture("logo_sunflower", color_image, egui::TextureOptions::LINEAR)
        });

        let mut cfg = crate::config::load_config();
        crate::lang::set_current(cfg.t());
        cfg.brand = brand;
        cfg.channel = channel;
        apply_font_mode(ctx, &cfg.font_mode);

        let root = cfg.data_root_for();
        // Create the standard launcher folders right away so the install
        // directory looks like a normal launcher instead of an empty folder.
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
            custom_versions: Vec::new(),
            loaders_mc: String::new(),
            fabric_loaders: Vec::new(),
            quilt_loaders: Vec::new(),
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

            inst_dialog: None,
            delete_dialog: None,
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

    // ------------------------------------------------------------------ events

    fn drain_events(&mut self, ctx: &egui::Context) {
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
                    let img =
                        egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
                    let tex = ctx.load_texture(
                        format!("avatar-{uuid}"),
                        img,
                        TextureOptions::LINEAR,
                    );
                    self.avatars.insert(uuid, tex);
                }
                TaskEvent::VersionList { latest, versions } => {
                    let had_selection = self
                        .cfg
                        .active_instance()
                        .and_then(|i| i.version_id.clone());
                    self.latest_release = latest;
                    self.versions = versions;
                    self.versions_loaded = true;
                    self.refresh_installed();
                    // Only auto-select a version for ordinary (non-latest) instances.
                    if had_selection.is_none() {
                        let is_latest = self
                            .cfg
                            .active_instance()
                            .map(|i| i.is_latest)
                            .unwrap_or(true);
                        if !is_latest {
                            if let Some(id) = self
                                .versions
                                .iter()
                                .find(|v| self.version_visible(v))
                                .map(|v| v.id.clone())
                            {
                                if let Some(inst) = self.cfg.active_instance_mut() {
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
                TaskEvent::Loaders { mc, fabric, quilt } => {
                    self.loaders_mc = mc;
                    self.fabric_loaders = fabric;
                    self.quilt_loaders = quilt;
                }
                TaskEvent::GameStarted(pid) => {
                    self.running_pid = Some(pid);
                    self.task_active = false;
                    self.toast(
                        ToastKind::Ok,
                        self.cfg.t().game_running.replace("{}", &pid.to_string()),
                    );
                    // Hide the launcher immediately so the user isn't stuck
                    // watching the (slow) JVM boot. GameExited brings the
                    // launcher back if the game crashes.
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                }
                TaskEvent::GameOutput(line) => self.push_console(&line),
                TaskEvent::GameExited(code) => {
                    self.running_pid = None;
                    self.toast(
                        if code == 0 {
                            ToastKind::Ok
                        } else {
                            ToastKind::Error
                        },
                        self.cfg.t().game_exited.replace("{}", &code.to_string()),
                    );
                    // Bring the launcher window back after the game closes.
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                }
                TaskEvent::JavaReady(dir) => {
                    let java = dir.join("bin").join(crate::java::java_binary_name());
                    if java.exists() {
                        if let Some(inst) = self.cfg.active_instance_mut() {
                            inst.java_path = Some(java.to_string_lossy().into_owned());
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

    fn refresh_installed(&mut self) {
        let mut set = HashSet::new();
        let dir = self.root.join("versions");
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let id = e.file_name().to_string_lossy().into_owned();
                if crate::install::version_is_installed(&self.root, &id) {
                    set.insert(id);
                }
            }
        }
        self.installed = set;
        self.scan_custom_versions();
    }

    /// Re-resolve the launcher data root after the user changed the data-folder
    /// setting, recreate the standard folders and refresh what is installed.
    fn apply_data_root(&mut self) {
        self.root = self.cfg.data_root_for();
        for sub in ["versions", "libraries", "assets", "runtime", "logs"] {
            let _ = std::fs::create_dir_all(self.root.join(sub));
        }
        let _ = std::fs::create_dir_all(&self.root);
        self.refresh_installed();
    }

    /// Find custom clients (modded / client-mod versions) that live in the
    /// launcher's `versions/` folder but are NOT part of the official Mojang
    /// manifest. A version counts as a "custom client" only when it is fully
    /// installed (valid JSON + client jar present), so the dropdown never lists
    /// broken/partial folders.
    fn scan_custom_versions(&mut self) {
        let manifest_ids: HashSet<String> = self.versions.iter().map(|v| v.id.clone()).collect();
        let mut custom: Vec<ManifestVersion> = Vec::new();
        let dir = self.root.join("versions");
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let id = e.file_name().to_string_lossy().into_owned();
                if manifest_ids.contains(&id) {
                    continue;
                }
                if !crate::install::version_is_installed(&self.root, &id) {
                    continue;
                }
                if let Ok(v) = crate::install::load_local_version(&self.root, &id) {
                    if v.main_class.is_empty() {
                        continue;
                    }
                } else {
                    continue;
                }
                custom.push(ManifestVersion {
                    id,
                    kind: "custom".to_string(),
                    url: String::new(),
                    release_time: None,
                });
            }
        }
        custom.sort_by(|a, b| a.id.cmp(&b.id));
        self.custom_versions = custom;
    }

    fn add_or_replace_account(&mut self, account: Account) {
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

    fn remove_account(&mut self, idx: usize) {
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

    fn toast(&mut self, kind: ToastKind, msg: impl Into<String>) {
        self.toasts.push(Toast { msg: msg.into(), kind, born: Instant::now() });
        if self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
    }

    /// Replace every saved access/refresh token with a placeholder so secrets
    /// never appear in the console or UI.
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

    fn push_console(&mut self, line: &str) {
        let redacted = self.redact_sensitive(line);
        self.console.push_str(&redacted);
        self.console.push('\n');
        if self.console.len() > 400_000 {
            self.console = self.console.split_off(self.console.len() - 300_000);
        }
        self.console_scroll = true;
    }

    /// True when a background task is running and important buttons should be locked.
    fn busy(&self) -> bool {
        self.task_active || self.game_running()
    }

    fn start(
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

    fn game_running(&self) -> bool {
        self.running_pid.is_some()
    }

    fn play_clicked(&mut self) {
        let t = self.cfg.t();
        if self.game_running() {
            if let Some(pid) = self.running_pid {
                crate::tasks::kill_game(pid);
                self.running_pid = None;
                self.toast(ToastKind::Info, t.stop_game);
            }
            return;
        }
        let Some(inst) = self.cfg.active_instance().cloned() else {
            self.toast(ToastKind::Error, t.no_instance_selected);
            return;
        };
        let Some(version_id) = self.instance_resolved_version(&inst) else {
            self.toast(ToastKind::Error, t.version_unavailable);
            return;
        };

        // Not installed yet -> the button acts as "PASANG" (except for the built-in
        // "latest" instance, which picks its version during launch and asks
        // the user whether to update first).
        if !inst.is_latest && !self.installed.contains(&version_id) {
            self.toast(ToastKind::Info, t.installing_version);
            let root = self.root.clone();
            self.start(move |tx, drx| crate::tasks::install_version(tx, drx, version_id, root));
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
        let req = LaunchRequest { config: self.cfg.clone(), account, version: Some(version_id.clone()) };
        let root = self.root.clone();
        self.start(move |tx, drx| {
            crate::tasks::launch_game(tx, drx, req, root);
        });
    }

    fn start_refresh_versions(&mut self) {
        self.versions_loaded = true;
        self.start(|tx, drx| crate::tasks::refresh_versions(tx, drx));
    }

    fn active_version_installed(&self) -> bool {
        self.active_resolved_version()
            .as_ref()
            .map(|id| self.installed.contains(id))
            .unwrap_or(false)
    }

    /// Newest stable release (from the manifest, or derived from the loaded list).
    fn latest_stable(&self) -> Option<String> {
        if let Some(id) = &self.latest_release {
            return Some(id.clone());
        }
        self.versions
            .iter()
            .filter(|v| v.kind == "release")
            .max_by(|a, b| a.release_time.cmp(&b.release_time))
            .map(|v| v.id.clone())
    }

    /// The version an instance resolves to. The built-in "latest" instance
    /// always resolves to the newest stable release and ignores its version_id.
    fn instance_resolved_version(&self, inst: &Instance) -> Option<String> {
        if inst.is_latest {
            self.latest_stable()
        } else {
            inst.version_id.clone()
        }
    }

    fn active_resolved_version(&self) -> Option<String> {
        self.cfg
            .active_instance()
            .and_then(|i| self.instance_resolved_version(i))
    }

    /// Display name for an instance (the built-in one shows a localized label).
    fn instance_display_name(&self, inst: &Instance) -> String {
        if inst.is_latest {
            self.cfg.t().latest_instance.to_string()
        } else {
            inst.name.clone()
        }
    }

    /// Only show stable releases unless the user enables all versions in settings.
    fn version_visible(&self, v: &ManifestVersion) -> bool {
        if v.kind == "custom" {
            return self.cfg.show_custom_clients;
        }
        self.cfg.show_all_versions || v.kind == "release"
    }

    /// Every version that may be shown right now: official manifest versions
    /// (filtered by the "show all" switch) plus local custom clients (filtered
    /// by the "show custom clients" switch).
    fn visible_versions(&self) -> Vec<&ManifestVersion> {
        let mut out: Vec<&ManifestVersion> = self
            .versions
            .iter()
            .filter(|v| self.version_visible(v))
            .collect();
        out.extend(self.custom_versions.iter().filter(|v| self.version_visible(v)));
        out
    }

    // ------------------------------------------------------------------ drawing

    fn draw(&mut self, root_ui: &mut egui::Ui) {
        let ctx = root_ui.ctx().clone();

        // Another HanaLauncher instance is already running: only show a
        // small warning dialog; closing it closes this window too.
        if self.warn_existing {
            let t = self.cfg.t();
            let mut close = false;
            egui::Window::new("HanaLauncher")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(&ctx, |ui| {
                    ui.set_width(300.0);
                    ui.label(RichText::new(t.already_running).size(14.0));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            close = true;
                        }
                    });
                });
            if close {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }

        // A newer stable release is available for the "latest" instance.
        if let Some((newest, current)) = self.version_choice.clone() {
            let t = self.cfg.t();
            let mut chosen: Option<LaunchDecision> = None;
            egui::Window::new(t.update_available)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(&ctx, |ui| {
                    ui.set_width(360.0);
                    ui.label(
                        RichText::new(t.update_available_body.replace("{}", &newest)).size(14.0),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(t.installed_current.replace("{}", &current))
                            .size(12.0)
                            .color(TEXT_WEAK),
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                true,
                                egui::Button::new(t.continue_playing.replace("{}", &current)),
                            )
                            .clicked()
                        {
                            chosen = Some(LaunchDecision::PlayVersion(current.clone()));
                        }
                        if ui
                            .add_enabled(true, egui::Button::new(t.update_now))
                            .clicked()
                        {
                            chosen = Some(LaunchDecision::PlayVersion(newest.clone()));
                        }
                        if ui.button(t.cancel).clicked() {
                            chosen = Some(LaunchDecision::Cancel);
                        }
                    });
                });
            if let Some(decision) = chosen {
                if let Some(tx) = &self.decision_tx {
                    let _ = tx.send(decision);
                }
                self.version_choice = None;
            }
        }

        // -------------------------------------- full-window background image
        {
            let painter = ctx.layer_painter(egui::LayerId::background());
            let rect = ctx.input(|i| i.viewport_rect());
            match &self.bg {
                Some(bg) => {
                    let iw = bg.size_vec2().x;
                    let ih = bg.size_vec2().y;
                    let scale = (rect.width() / iw).max(rect.height() / ih).max(0.001);
                    let dw = iw * scale;
                    let dh = ih * scale;
                    let offset = egui::vec2((rect.width() - dw) * 0.5, (rect.height() - dh) * 0.5);
                    let draw_rect =
                        egui::Rect::from_min_size(rect.min + offset, egui::vec2(dw, dh));
                    painter.image(
                        bg.id(),
                        draw_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                None => paint_vgrad(&painter, rect, BG_TOP, BG_BOTTOM),
            }
        }

        // ------------------------------------------------------ custom titlebar
        let bar_h = 36.0;
        egui::Panel::top("titlebar")
            .frame(egui::Frame::default().fill(PRIMARY_BTN).inner_margin(egui::Margin::ZERO))
            .exact_size(bar_h)
            .resizable(false)
            .show_separator_line(false)
            .show(root_ui, |ui| {
                ui.set_min_height(bar_h);
                let full = ui.max_rect();
                let drag = ui.interact(
                    full,
                    egui::Id::new("titlebar_drag"),
                    egui::Sense::click_and_drag(),
                );
                if drag.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if drag.double_clicked() {
                    let maxed = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maxed));
                }

                let painter = ui.painter();
                if let Some(logo) = &self.logo {
                    painter.image(
                        logo.id(),
                        egui::Rect::from_min_size(
                            full.min + egui::vec2(10.0, (bar_h - 20.0) * 0.5),
                            egui::vec2(20.0, 20.0),
                        ),
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                painter.text(
                    egui::pos2(full.left() + 38.0, full.center().y),
                    egui::Align2::LEFT_CENTER,
                    "HanaLauncher",
                    egui::FontId::proportional(14.0),
                    PRIMARY_TEXT,
                );

                // Window control buttons (drawn with painter so no font glyphs needed).
                let btn_w = 46.0;
                let close_r = egui::Rect::from_min_size(
                    full.right_top() + egui::vec2(-btn_w, 0.0),
                    egui::vec2(btn_w, bar_h),
                );
                let max_r = egui::Rect::from_min_size(
                    close_r.left_top() + egui::vec2(-btn_w, 0.0),
                    egui::vec2(btn_w, bar_h),
                );
                let min_r = egui::Rect::from_min_size(
                    max_r.left_top() + egui::vec2(-btn_w, 0.0),
                    egui::vec2(btn_w, bar_h),
                );
                let hover_bg = Color32::from_rgba_unmultiplied(255, 255, 255, 40);

                let min_resp = ui.interact(min_r, egui::Id::new("win_min"), egui::Sense::click());
                if min_resp.clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
                if min_resp.hovered() {
                    painter.rect_filled(min_r, 0.0, hover_bg);
                }
                let my = min_r.center().y;
                painter.line_segment(
                    [
                        egui::pos2(min_r.center().x - 6.0, my),
                        egui::pos2(min_r.center().x + 6.0, my),
                    ],
                    egui::Stroke::new(1.6, PRIMARY_TEXT),
                );

                let maxed = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                let max_resp = ui.interact(max_r, egui::Id::new("win_max"), egui::Sense::click());
                if max_resp.clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maxed));
                }
                if max_resp.hovered() {
                    painter.rect_filled(max_r, 0.0, hover_bg);
                }
                let mcx = max_r.center().x;
                let mcy = max_r.center().y;
                if maxed {
                    painter.rect_stroke(
                        egui::Rect::from_center_size(
                            egui::pos2(mcx - 2.5, mcy - 2.5),
                            egui::vec2(9.0, 9.0),
                        ),
                        egui::CornerRadius::ZERO,
                        egui::Stroke::new(1.4, PRIMARY_TEXT),
                        egui::StrokeKind::Inside,
                    );
                    painter.rect_stroke(
                        egui::Rect::from_center_size(
                            egui::pos2(mcx + 2.5, mcy + 2.5),
                            egui::vec2(9.0, 9.0),
                        ),
                        egui::CornerRadius::ZERO,
                        egui::Stroke::new(1.4, PRIMARY_TEXT),
                        egui::StrokeKind::Inside,
                    );
                } else {
                    painter.rect_stroke(
                        egui::Rect::from_center_size(egui::pos2(mcx, mcy), egui::vec2(10.0, 10.0)),
                        egui::CornerRadius::ZERO,
                        egui::Stroke::new(1.4, PRIMARY_TEXT),
                        egui::StrokeKind::Inside,
                    );
                }

                let close_resp =
                    ui.interact(close_r, egui::Id::new("win_close"), egui::Sense::click());
                if close_resp.clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if close_resp.hovered() {
                    painter.rect_filled(
                        close_r,
                        0.0,
                        Color32::from_rgba_unmultiplied(214, 60, 48, 80),
                    );
                }
                let ccx = close_r.center().x;
                let ccy = close_r.center().y;
                painter.line_segment(
                    [
                        egui::pos2(ccx - 5.0, ccy - 5.0),
                        egui::pos2(ccx + 5.0, ccy + 5.0),
                    ],
                    egui::Stroke::new(1.6, PRIMARY_TEXT),
                );
                painter.line_segment(
                    [
                        egui::pos2(ccx - 5.0, ccy + 5.0),
                        egui::pos2(ccx + 5.0, ccy - 5.0),
                    ],
                    egui::Stroke::new(1.6, PRIMARY_TEXT),
                );
            });

        // ---------------------------------------------------------- sidebar
        egui::Panel::left("nav")
            .frame(egui::Frame::default().fill(GLASS_NAV))
            .default_size(172.0)
            .min_size(172.0)
            .max_size(172.0)
            .resizable(false)
            .show_separator_line(true)
            .show(root_ui, |ui| {
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("HanaLauncher")
                                .size(18.0)
                                .strong()
                                .color(TEXT),
                        );
                        ui.label(
                            RichText::new("By StievenW")
                                .size(11.0)
                                .color(TEXT_WEAK),
                        );
                    });
                });

                ui.add_space(14.0);
                let t = self.cfg.t();
                let pages = [
                    (t.nav_home, Page::Home),
                    (t.nav_versions, Page::Versions),
                    (t.nav_instances, Page::Instances),
                    (t.nav_accounts, Page::Accounts),
                    (t.nav_settings, Page::Settings),
                    (t.nav_console, Page::Console),
                ];
                for (label, page) in pages {
                    let selected = self.page == page;
                    let text = RichText::new(label)
                        .size(14.0)
                        .color(if selected { ACCENT } else { TEXT });
                    let button = egui::Button::new(text)
                        .fill(if selected { ACCENT_SOFT } else { Color32::TRANSPARENT })
                        .stroke(if selected {
                            egui::Stroke::new(1.0, ACCENT)
                        } else {
                            egui::Stroke::NONE
                        })
                        .corner_radius(6.0)
                        .min_size(Vec2::new(150.0, 30.0));
                    if ui.add_sized(Vec2::new(150.0, 30.0), button).clicked() {
                        self.page = page;
                    }
                }

                let version_name = self
                    .active_resolved_version()
                    .unwrap_or_else(|| "-".to_string());
                let inst_memory = self
                    .cfg
                    .active_instance()
                    .map(|i| i.memory_mb)
                    .unwrap_or(2048);
                let installed = self.active_version_installed();

                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.add_space(8.0);
                    egui::Frame::default()
                        .fill(GLASS_NAV)
                        .stroke(egui::Stroke::new(1.0, BORDER))
                        .corner_radius(8.0)
                        .inner_margin(egui::Margin::symmetric(10, 10))
                        .show(ui, |ui| {
                            ui.set_width(148.0);
                            ui.label(RichText::new(t.active_version).size(9.0).color(TEXT_WEAK));
                            ui.add_space(2.0);
                            ui.label(RichText::new(&version_name).size(13.0).strong());
                            if installed {
                                ui.label(RichText::new(t.installed).size(10.0).color(OK_GREEN));
                            } else {
                                ui.label(RichText::new(t.not_installed).size(10.0).color(WARN_YELLOW));
                            }
                            ui.add_space(6.0);
                            ui.separator();
                            ui.add_space(6.0);
                            ui.label(RichText::new(t.memory).size(9.0).color(TEXT_WEAK));
                            ui.add_space(2.0);
                            ui.label(RichText::new(format!("{inst_memory} MB")).size(13.0).strong());
                        });
                });
            });

        // ---------------------------------------------------------- top bar
        egui::Panel::top("top_bar")
            .frame(egui::Frame::default().fill(GLASS_NAV))
            .show(root_ui, |ui| {
                ui.horizontal(|ui| {
                    let t = self.cfg.t();
                    let title = match self.page {
                        Page::Home => t.nav_home,
                        Page::Versions => t.nav_versions,
                        Page::Instances => t.nav_instances,
                        Page::Accounts => t.nav_accounts,
                        Page::Settings => t.nav_settings,
                        Page::Console => t.nav_console,
                    };
                    ui.label(RichText::new(title).size(16.0).strong());
                    if self.busy() {
                        ui.add_space(10.0);
                        ui.add(egui::Spinner::new().size(14.0).color(ACCENT));
                        ui.label(RichText::new(&self.task_stage).size(12.0).color(TEXT_WEAK));
                    }
                });
            });

        // ---------------------------------------------------------- status bar
        egui::Panel::bottom("status_bar")
            .frame(egui::Frame::default().fill(GLASS_NAV))
            .show(root_ui, |ui| {
                if self.task_active {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(13.0).color(ACCENT));
                        ui.label(RichText::new(&self.task_stage).size(11.0).color(TEXT_WEAK));
                        if let Some((current, total)) = self.task_progress {
                            let frac = if total > 0 {
                                (current as f32 / total as f32).min(1.0)
                            } else {
                                0.0
                            };
                            ui.add(
                                egui::ProgressBar::new(frac)
                                    .desired_width(240.0)
                                    .show_percentage()
                                    .text(format!(
                                        "{} / {}",
                                        crate::util::fmt_bytes(current),
                                        crate::util::fmt_bytes(total)
                                    )),
                            );
                        }
                    });
                } else if let Some(pid) = self.running_pid {
                    ui.label(
                        RichText::new(self.cfg.t().game_running.replace("{}", &pid.to_string()))
                            .size(11.0)
                            .color(OK_GREEN),
                    );
                } else {
                    let t = self.cfg.t();
                    ui.label(RichText::new(t.home_ready).size(11.0).color(TEXT_WEAK));
                }
            });

        // ---------------------------------------------------------- content
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(Color32::TRANSPARENT))
            .show(root_ui, |ui| {
                ui.add_space(6.0);
                match self.page {
                    Page::Home => self.ui_home(ui),
                    Page::Versions => self.ui_versions(ui),
                    Page::Instances => self.ui_instances(ui),
                    Page::Accounts => self.ui_accounts(ui),
                    Page::Settings => self.ui_settings(ui),
                    Page::Console => self.ui_console(ui),
                }
            });

        self.handle_resize(&ctx);

        // Toasts
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-16.0, -52.0))
            .order(egui::Order::Foreground)
            .show(&ctx, |ui| {
                let t = self.cfg.t();
                for t_toast in &self.toasts {
                    let color = match t_toast.kind {
                        ToastKind::Ok => OK_GREEN,
                        ToastKind::Error => ERR_RED,
                        ToastKind::Info => ACCENT,
                    };
                    let title = match t_toast.kind {
                        ToastKind::Ok => t.toast_ok,
                        ToastKind::Error => t.toast_err,
                        ToastKind::Info => t.toast_info,
                    };
                    egui::Frame::default()
                        .fill(GLASS_SOLID)
                        .stroke(egui::Stroke::new(1.0, BORDER))
                        .corner_radius(8.0)
                        .shadow(egui::epaint::Shadow {
                            offset: [0, 2],
                            blur: 12,
                            spread: 0,
                            color: Color32::from_black_alpha(30),
                        })
                        .inner_margin(egui::Margin::symmetric(12, 10))
                        .show(ui, |ui| {
                            ui.set_width(300.0);
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(
                                    Vec2::new(8.0, 8.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().circle_filled(rect.center(), 4.0, color);
                                ui.add_space(8.0);
                                ui.vertical(|ui| {
                                    ui.label(
                                        RichText::new(title)
                                            .size(12.0)
                                            .strong()
                                            .color(color),
                                    );
                                    ui.label(
                                        RichText::new(&t_toast.msg).size(13.0).color(TEXT),
                                    );
                                });
                            });
                        });
                    ui.add_space(6.0);
                }
            });

        // 2FA dialog
        if self.need_2fa {
            let t = self.cfg.t();
            let mut close = false;
            let mut submit = false;
            egui::Modal::new(egui::Id::new("twofa_modal")).show(&ctx, |ui| {
                ui.set_width(320.0);
                ui.label(RichText::new(t.twofa_title).size(15.0).strong());
                ui.add_space(6.0);
                ui.label(t.twofa_hint);
                ui.add(
                    egui::TextEdit::singleline(&mut self.twofa_input)
                        .hint_text("000000")
                        .desired_width(220.0),
                );
                ui.horizontal(|ui| {
                    if ui.button(t.submit).clicked() {
                        submit = true;
                    }
                    if ui.button(t.cancel).clicked() {
                        close = true;
                    }
                });
            });
            if submit {
                let code = self.twofa_input.trim().to_string();
                if code.is_empty() {
                    self.toast(ToastKind::Error, t.twofa_empty);
                } else {
                    let username = self.pw_username.clone();
                    let password = self.pw_password.clone();
                    self.start(move |tx, drx| {
                        crate::tasks::login_password(tx, drx, username, password, Some(code));
                    });
                    self.need_2fa = false;
                    self.twofa_input.clear();
                }
            }
            if close {
                self.need_2fa = false;
                self.twofa_input.clear();
            }
        }

        // Device-code (OAuth) dialog
        if let Some((code, verification_uri)) = self.device_code.clone() {
            let t = self.cfg.t();
            let mut close = false;
            // A proper modal: dims + blocks the rest of the UI. It only closes
            // through the Cancel button / Escape, never by clicking outside.
            let modal = egui::Modal::new(egui::Id::new("oauth_modal"))
                .show(&ctx, |ui| {
                    ui.set_width(400.0);
                    ui.label(RichText::new(t.verify_win_title).size(15.0).strong());
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(t.login_ely_account)
                            .color(TEXT_WEAK)
                            .size(11.0),
                    );
                    ui.add_space(6.0);
                    ui.label(t.verify_page);
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(&code)
                                .monospace()
                                .size(28.0)
                                .color(ACCENT),
                        );
                        if ui.button(t.copy).clicked() {
                            ctx.copy_text(code.clone());
                        }
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button(t.open_verify_page).clicked() {
                            if webbrowser::open(&verification_uri).is_err() {
                                self.toast(ToastKind::Error, t.cannot_open_browser);
                            }
                        }
                        if ui.button(t.cancel).clicked() {
                            close = true;
                        }
                    });
                    ui.add_space(4.0);
                    ui.label(RichText::new(t.oauth_auto).color(TEXT_WEAK).size(12.0));
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.hyperlink_to(
                            RichText::new(t.register_ely).size(11.0).color(ACCENT),
                            crate::util::ELY_REGISTER_URL,
                        );
                    });
                });
            // Block outside clicks; only Cancel button or Escape closes it.
            let esc = modal.is_top_modal
                && !modal.any_popup_open
                && ctx.input_mut(|i| {
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                });
            if close || esc {
                self.device_code = None;
                // Tell the running login-oauth task to stop, otherwise the
                // loading spinner stays forever.
                if let Some(tx) = self.decision_tx.take() {
                    let _ = tx.send(LaunchDecision::Cancel);
                }
                self.toast(ToastKind::Info, t.login_cancelled);
            }
        }

        self.update_toasts();
    }

    fn handle_resize(&self, ctx: &egui::Context) {
        let maxed = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        if maxed {
            return;
        }
        let rect = ctx.input(|i| i.viewport_rect());
        let edge = 7.0;
        let corner = 16.0;
        let strips: [(&str, egui::Rect, egui::ResizeDirection); 8] = [
            (
                "n",
                egui::Rect::from_min_size(
                    rect.min + egui::vec2(corner, 0.0),
                    egui::vec2(rect.width() - 2.0 * corner, edge),
                ),
                egui::ResizeDirection::North,
            ),
            (
                "s",
                egui::Rect::from_min_size(
                    rect.left_bottom() + egui::vec2(corner, -edge),
                    egui::vec2(rect.width() - 2.0 * corner, edge),
                ),
                egui::ResizeDirection::South,
            ),
            (
                "w",
                egui::Rect::from_min_size(
                    rect.min + egui::vec2(0.0, corner),
                    egui::vec2(edge, rect.height() - 2.0 * corner),
                ),
                egui::ResizeDirection::West,
            ),
            (
                "e",
                egui::Rect::from_min_size(
                    rect.right_top() + egui::vec2(-edge, corner),
                    egui::vec2(edge, rect.height() - 2.0 * corner),
                ),
                egui::ResizeDirection::East,
            ),
            (
                "nw",
                egui::Rect::from_min_size(rect.min, egui::vec2(corner, corner)),
                egui::ResizeDirection::NorthWest,
            ),
            (
                "ne",
                egui::Rect::from_min_size(
                    rect.right_top() + egui::vec2(-corner, 0.0),
                    egui::vec2(corner, corner),
                ),
                egui::ResizeDirection::NorthEast,
            ),
            (
                "sw",
                egui::Rect::from_min_size(
                    rect.left_bottom() + egui::vec2(0.0, -corner),
                    egui::vec2(corner, corner),
                ),
                egui::ResizeDirection::SouthWest,
            ),
            (
                "se",
                egui::Rect::from_min_size(
                    rect.right_bottom() + egui::vec2(-corner, -corner),
                    egui::vec2(corner, corner),
                ),
                egui::ResizeDirection::SouthEast,
            ),
        ];
        egui::Area::new(egui::Id::new("resize_edges"))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                for (name, srect, dir) in strips {
                    let resp = ui.interact(srect, egui::Id::new(("resize_edge", name)), egui::Sense::drag());
                    if resp.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
                    }
                }
            });
    }

    fn update_toasts(&mut self) {
        let now = Instant::now();
        self.toasts.retain(|t| now.duration_since(t.born).as_secs() < 5);
    }

    // ------------------------------------------------------------- pages

    fn ui_home(&mut self, ui: &mut egui::Ui) {
        let t = self.cfg.t();

        // ----- active instance -----
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(t.nav_instances.to_uppercase()).size(9.0).color(TEXT_WEAK));
                    ui.add_space(2.0);
                    let inst_name = self
                        .cfg
                        .active_instance()
                        .map(|i| self.instance_display_name(i))
                        .unwrap_or_else(|| "-".to_string());
                    ui.label(RichText::new(&inst_name).size(18.0).strong());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(t.manage).clicked() {
                        self.page = Page::Instances;
                    }
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(t.choose_instance).size(12.0).color(TEXT_WEAK));
                let active = self
                    .cfg
                    .active_instance
                    .clone()
                    .unwrap_or_default();
                let mut new_active = active.clone();
                egui::ComboBox::from_id_salt("home_instance")
                    .selected_text(
                        RichText::new(
                            self.cfg
                                .active_instance()
                                .map(|i| self.instance_display_name(i))
                                .unwrap_or_else(|| "-".to_string()),
                        )
                        .size(14.0)
                        .strong()
                        .color(TEXT),
                    )
                    .width(240.0)
                    .show_ui(ui, |ui| {
                        for inst in &self.cfg.instances {
                            ui.selectable_value(
                                &mut new_active,
                                inst.name.clone(),
                                self.instance_display_name(inst),
                            );
                        }
                    });
                if new_active != active {
                    self.cfg.active_instance = Some(new_active);
                    let _ = save_config(&self.cfg);
                }
            });
        });

        ui.add_space(10.0);

        // ----- resolved version -----
        let sel = self
            .active_resolved_version()
            .unwrap_or_else(|| "-".to_string());
        let is_latest = self
            .cfg
            .active_instance()
            .map(|i| i.is_latest)
            .unwrap_or(false);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(t.mc_version).size(9.0).color(TEXT_WEAK));
                    ui.add_space(2.0);
                    ui.label(RichText::new(&sel).size(15.0).strong().color(TEXT));
                    ui.add_space(4.0);
                    if is_latest {
                        ui.label(RichText::new(t.latest_hint).size(11.0).color(TEXT_WEAK));
                    } else if sel == "-" {
                        ui.label(RichText::new(t.create_instance_hint).size(11.0).color(TEXT_WEAK));
                    }
                });
                ui.add_space(14.0);
                ui.separator();
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(t.status).size(9.0).color(TEXT_WEAK));
                    ui.add_space(2.0);
                    let status = if self.active_version_installed() {
                        RichText::new(t.installed).color(OK_GREEN)
                    } else if sel == "-" {
                        RichText::new(t.not_installed).color(TEXT_WEAK)
                    } else {
                        RichText::new(t.not_installed).color(WARN_YELLOW)
                    };
                    ui.label(status.size(13.0).strong());
                });
            });
        });

        ui.add_space(10.0);

        // ----- play / install (single button) -----
        let installed = self.active_version_installed();
        card(ui, |ui| {
            ui.horizontal(|ui| {
                let play_text = if self.game_running() {
                    t.stop
                } else if !installed {
                    t.install
                } else {
                    t.play
                };
                let has_version = self.active_resolved_version().is_some();
                let play_enabled = self.game_running()
                    || (has_version
                        && (!installed || self.cfg.active_account().is_some()));
                let play = egui::Button::new(
                    RichText::new(play_text)
                        .size(18.0)
                        .strong()
                        .color(PRIMARY_TEXT),
                )
                    .fill(if self.game_running() { ERR_RED } else { PRIMARY_BTN })
                    .corner_radius(10.0)
                    .min_size(Vec2::new(160.0, 46.0));
                if ui
                    .add_enabled(play_enabled && !self.task_active, play)
                    .clicked()
                {
                    self.play_clicked();
                }

                if self.busy() {
                    ui.label(
                        RichText::new(t.waiting_task)
                            .color(TEXT_WEAK)
                            .size(12.0),
                    );
                } else if self.cfg.active_instance().is_none() {
                    ui.label(
                        RichText::new(t.no_instance_selected)
                            .color(WARN_YELLOW)
                            .size(12.0),
                    );
                } else if !has_version {
                    ui.label(
                        RichText::new(t.need_version_hint)
                            .color(WARN_YELLOW)
                            .size(12.0),
                    );
                } else if !installed && self.cfg.active_account().is_none() {
                    ui.label(
                        RichText::new(t.need_login_hint)
                            .color(WARN_YELLOW)
                            .size(12.0),
                    );
                }
            });
        });

        ui.add_space(10.0);

        // ----- active account -----
        ui.label(RichText::new(t.active_account).size(9.0).color(TEXT_WEAK));
        ui.add_space(4.0);
        card(ui, |ui| match self.cfg.active_account() {
            Some(acc) => {
                ui.horizontal(|ui| {
                    if let Some(tex) = self.avatars.get(&acc.uuid) {
                        ui.image((tex.id(), Vec2::new(38.0, 38.0)));
                    } else {
                        ui.allocate_ui(Vec2::new(38.0, 38.0), |ui| {
                            ui.centered_and_justified(|ui| {
                                ui.label(
                                    RichText::new(
                                        acc.username.chars().next().unwrap_or('?').to_string(),
                                    )
                                    .size(16.0)
                                    .strong(),
                                );
                            });
                        });
                    }
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&acc.username).size(15.0).strong());
                        let ty = if acc.account_type == crate::config::ACCOUNT_TYPE_ELY_OAUTH {
                            t.ely_oauth
                        } else if acc.account_type == crate::config::ACCOUNT_TYPE_ELY_PASSWORD {
                            t.ely_password
                        } else {
                            t.offline_account
                        };
                        ui.label(RichText::new(ty).size(11.0).color(TEXT_WEAK));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(t.manage).clicked() {
                            self.page = Page::Accounts;
                        }
                    });
                });
            }
            None => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(t.no_active_account).color(TEXT_WEAK));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(t.sign_in).clicked() {
                            self.page = Page::Accounts;
                        }
                    });
                });
            }
        });
    }

    fn ui_versions(&mut self, ui: &mut egui::Ui) {
        let t = self.cfg.t();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.versions_tab_downloaded, false, t.tab_all);
            ui.selectable_value(&mut self.versions_tab_downloaded, true, t.tab_downloaded);
            ui.separator();
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text(t.search_versions)
                    .desired_width(220.0),
            );
            if ui
                .add_enabled(
                    !self.task_active,
                    egui::Button::new(RichText::new(t.reload).size(13.0)),
                )
                .clicked()
            {
                self.start_refresh_versions();
            }
            if self.task_active {
                ui.add(egui::Spinner::new().size(14.0).color(ACCENT));
            }
        });
        if !self.cfg.show_all_versions {
            ui.label(
                RichText::new(t.releases_only_hint)
                    .color(TEXT_WEAK)
                    .size(11.0),
            );
        }

        if self.versions_tab_downloaded {
            self.ui_versions_downloaded(ui);
            return;
        }

        if self.versions.is_empty() {
            ui.add_space(30.0);
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new(if self.task_active {
                        t.loading_versions
                    } else {
                        t.empty_versions
                    })
                    .color(TEXT_WEAK),
                );
            });
            return;
        }

        let filter = self.search.to_lowercase();
        ui.add_space(4.0);

        let list = self.visible_versions();
        let rows: Vec<usize> = (0..list.len())
            .filter(|&i| {
                let v = list[i];
                filter.is_empty() || v.id.to_lowercase().contains(&filter)
            })
            .collect();

        const ROW_H: f32 = 50.0;
        let mut action: Option<VersionAction> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(false)
            .show_rows(ui, ROW_H, rows.len(), |ui, range| {
                for i in range {
                    let idx = rows[i];
                    let v = list[idx];
                    let installed = self.installed.contains(&v.id);
                    let is_latest_active = self
                        .cfg
                        .active_instance()
                        .map(|i| i.is_latest)
                        .unwrap_or(true);
                    let is_selected = if is_latest_active {
                        self.active_resolved_version().as_deref() == Some(v.id.as_str())
                    } else {
                        self.cfg
                            .active_instance()
                            .and_then(|inst| inst.version_id.as_deref())
                            == Some(v.id.as_str())
                    };

                    egui::Frame::default()
                        .fill(if is_selected { GLASS_SOLID } else { GLASS })
                        .stroke(egui::Stroke::new(
                            1.0,
                            if is_selected { ACCENT_SOFT_STROKE } else { BORDER },
                        ))
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(&v.id).size(14.0).strong());
                                    let kind_color = match v.kind.as_str() {
                                        "release" => OK_GREEN,
                                        "snapshot" => WARN_YELLOW,
                                        _ => Color32::from_gray(170),
                                    };
                                    ui.label(RichText::new(&v.kind).size(10.0).color(kind_color));
                                });
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if installed {
                                            if is_selected {
                                                ui.label(
                                                    RichText::new(t.installed)
                                                        .color(OK_GREEN)
                                                        .size(11.0),
                                                );
                                            } else if ui
                                                .add_enabled(
                                                    !self.task_active && !is_latest_active,
                                                    egui::Button::new(t.use_version),
                                                )
                                                .clicked()
                                            {
                                                action = Some(VersionAction::Select(v.id.clone()));
                                            }
                                        } else if ui
                                            .add_enabled(
                                                !self.task_active,
                                                egui::Button::new(t.pick_version),
                                            )
                                            .clicked()
                                        {
                                            action = Some(VersionAction::Install(v.id.clone()));
                                        }
                                    },
                                );
                            });
                        });
                }
            });

        if self.cfg.show_custom_clients {
            ui.add_space(10.0);
            card(ui, |ui| {
                ui.label(RichText::new(t.client_mods).size(9.0).color(TEXT_WEAK));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(t.client_mods_mc);
                    let releases: Vec<String> = self
                        .versions
                        .iter()
                        .filter(|v| v.kind == "release")
                        .map(|v| v.id.clone())
                        .collect();
                    let mut sel = if self.loaders_mc.is_empty() {
                        self.latest_stable().unwrap_or_default()
                    } else {
                        self.loaders_mc.clone()
                    };
                    egui::ComboBox::from_id_salt("client_mods_mc")
                        .selected_text(RichText::new(&sel).size(13.0).color(TEXT))
                        .width(170.0)
                        .show_ui(ui, |ui| {
                            for id in &releases {
                                ui.selectable_value(&mut sel, id.clone(), id);
                            }
                        });
                    if sel != self.loaders_mc {
                        self.loaders_mc = sel;
                        self.fabric_loaders.clear();
                        self.quilt_loaders.clear();
                    }
                    if ui
                        .add_enabled(!self.task_active, egui::Button::new(t.client_mods_load))
                        .clicked()
                    {
                        let mc = self.loaders_mc.clone();
                        self.start(move |tx, drx| crate::tasks::refresh_loaders(tx, drx, mc));
                    }
                    if self.task_active {
                        ui.add(egui::Spinner::new().size(14.0).color(ACCENT));
                    }
                });

                let mut install_targets: Vec<(crate::minecraft::LoaderKind, String, String)> =
                    Vec::new();
                let mc = self.loaders_mc.clone();
                const MAX: usize = 8;
                let mut loader_section =
                    |ui: &mut egui::Ui,
                     name: &str,
                     kind: crate::minecraft::LoaderKind,
                     metas: &[crate::minecraft::LoaderMeta]| {
                        if metas.is_empty() {
                            return;
                        }
                        ui.add_space(6.0);
                        ui.label(RichText::new(name).strong().size(12.5));
                        let prefix = if kind == crate::minecraft::LoaderKind::Fabric {
                            "fabric-loader"
                        } else {
                            "quilt-loader"
                        };
                        for meta in metas.iter().take(MAX) {
                            let id = format!("{prefix}-{}-{}", meta.version, mc);
                            let installed = self.installed.contains(&id);
                            ui.horizontal(|ui| {
                                let label = if meta.stable == Some(false) {
                                    format!("{}  [beta]", meta.version)
                                } else {
                                    meta.version.clone()
                                };
                                ui.label(RichText::new(label).size(12.5).color(TEXT));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if installed {
                                            ui.label(
                                                RichText::new(t.installed)
                                                    .color(OK_GREEN)
                                                    .size(11.0),
                                            );
                                        } else if ui
                                            .add_enabled(
                                                !self.task_active,
                                                egui::Button::new(t.pick_version),
                                            )
                                            .clicked()
                                        {
                                            install_targets
                                                .push((kind, mc.clone(), meta.version.clone()));
                                        }
                                    },
                                );
                            });
                        }
                    };

                loader_section(ui, "Fabric", crate::minecraft::LoaderKind::Fabric, &self.fabric_loaders);
                loader_section(ui, "Quilt", crate::minecraft::LoaderKind::Quilt, &self.quilt_loaders);

                for (kind, mc, loader) in install_targets {
                    let root = self.root.clone();
                    self.start(move |tx, drx| {
                        crate::tasks::install_custom_client(tx, drx, kind, mc, loader, root);
                    });
                }
            });
        }

        if let Some(action) = action {
            match action {
                VersionAction::Select(id) => {
                    if let Some(inst) = self.cfg.active_instance_mut() {
                        inst.version_id = Some(id);
                    }
                    let _ = save_config(&self.cfg);
                }
                VersionAction::Install(id) => {
                    if let Some(inst) = self.cfg.active_instance_mut() {
                        if inst.is_editable() {
                            inst.version_id = Some(id.clone());
                        }
                    }
                    let _ = save_config(&self.cfg);
                    let root = self.root.clone();
                    self.start(move |tx, drx| crate::tasks::install_version(tx, drx, id, root));
                }
                VersionAction::DeleteData(id) => {
                    let dir = crate::install::version_dir(&self.root, &id);
                    if dir.exists() {
                        let _ = std::fs::remove_dir_all(&dir);
                    }
                    self.refresh_installed();
                    self.toast(ToastKind::Ok, t.version_deleted);
                }
            }
        }
    }

    fn ui_versions_downloaded(&mut self, ui: &mut egui::Ui) {
        let t = self.cfg.t();
        ui.add_space(4.0);
        if self.installed.is_empty() {
            ui.add_space(30.0);
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new(t.empty_downloaded).color(TEXT_WEAK));
            });
            return;
        }
        let mut installed_ids: Vec<String> = self.installed.iter().cloned().collect();
        installed_ids.sort_by(|a, b| b.cmp(a));
        let mut action: Option<VersionAction> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for id in installed_ids {
                    let is_selected = self.active_resolved_version().as_deref() == Some(id.as_str());
                    let is_latest = self.latest_stable().as_deref() == Some(id.as_str());
                    egui::Frame::default()
                        .fill(if is_selected { ACCENT_SOFT } else { GLASS })
                        .stroke(egui::Stroke::new(1.0, BORDER))
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&id).size(14.0).strong());
                                if is_latest {
                                    ui.label(
                                        RichText::new(t.latest_badge)
                                            .size(9.0)
                                            .color(ACCENT)
                                            .strong(),
                                    );
                                }
                                if is_selected {
                                    ui.label(
                                        RichText::new(t.installed).color(OK_GREEN).size(11.0),
                                    );
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .add_enabled(
                                                !self.task_active,
                                                egui::Button::new(t.delete_data),
                                            )
                                            .clicked()
                                        {
                                            action = Some(VersionAction::DeleteData(id.clone()));
                                        }
                                    },
                                );
                            });
                        });
                    ui.add_space(4.0);
                }
            });
        if let Some(action) = action {
            match action {
                VersionAction::DeleteData(id) => {
                    let dir = crate::install::version_dir(&self.root, &id);
                    if dir.exists() {
                        let _ = std::fs::remove_dir_all(&dir);
                    }
                    self.refresh_installed();
                    self.toast(ToastKind::Ok, t.version_deleted);
                }
                _ => {}
            }
        }
    }

    fn ui_instances(&mut self, ui: &mut egui::Ui) {
        let t = self.cfg.t();
        let busy = self.busy();

        ui.horizontal(|ui| {
            ui.label(RichText::new(t.instances_title).size(9.0).color(TEXT_WEAK));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !self.task_active,
                        egui::Button::new(RichText::new(t.new_instance).size(13.0)),
                    )
                    .clicked()
                {
                    let mut d = InstDraft::new();
                    d.version_id = self
                        .cfg
                        .active_instance()
                        .and_then(|i| i.version_id.clone());
                    self.inst_dialog = Some(d);
                }
            });
        });
        ui.add_space(4.0);

        if self.cfg.instances.is_empty() {
            card(ui, |ui| {
                ui.label(RichText::new(t.no_instances).color(TEXT_WEAK));
                ui.label(RichText::new(t.no_instances_hint).color(TEXT_WEAK).size(11.0));
            });
        } else {
            let mut select_idx: Option<usize> = None;
            let mut edit_idx: Option<usize> = None;
            let mut delete_idx: Option<usize> = None;
            let mut delete_data: Option<String> = None;
            let mut open_folder_idx: Option<usize> = None;

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (i, inst) in self.cfg.instances.iter().enumerate() {
                        let is_active =
                            self.cfg.active_instance.as_deref() == Some(inst.name.as_str());
                        let ver = self
                            .instance_resolved_version(inst)
                            .unwrap_or_else(|| "-".to_string());
                        let inst_ok = self
                            .instance_resolved_version(inst)
                            .as_ref()
                            .map(|id| self.installed.contains(id))
                            .unwrap_or(false);

                        egui::Frame::default()
                            .fill(if is_active { ACCENT_SOFT } else { GLASS })
                            .stroke(egui::Stroke::new(
                                1.0,
                                if is_active { ACCENT_SOFT_STROKE } else { BORDER },
                            ))
                            .corner_radius(8.0)
                            .inner_margin(egui::Margin::symmetric(12, 9))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(self.instance_display_name(inst))
                                                    .size(14.0)
                                                    .strong(),
                                            );
                                            if inst.is_latest {
                                                ui.label(
                                                    RichText::new(t.latest_badge)
                                                        .size(9.0)
                                                        .color(ACCENT)
                                                        .strong(),
                                                );
                                            }
                                            if is_active {
                                                ui.label(
                                                    RichText::new(t.active_badge)
                                                        .size(9.0)
                                                        .color(ACCENT)
                                                        .strong(),
                                                );
                                            }
                                        });
                                        let status = if inst_ok {
                                            t.installed
                                        } else {
                                            t.not_installed
                                        };
                                        ui.label(
                                            RichText::new(format!(
                                                "{ver}  -  {status}  -  {} MB",
                                                inst.memory_mb
                                            ))
                                            .color(TEXT_WEAK)
                                            .size(11.0),
                                        );
                                    });
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .add_enabled(
                                                    !busy,
                                                    egui::Button::new(t.open_folder),
                                                )
                                                .clicked()
                                            {
                                                open_folder_idx = Some(i);
                                            }
                                            if inst.is_latest {
                                                if ui
                                                    .add_enabled(
                                                        !busy,
                                                        egui::Button::new(t.delete_data),
                                                    )
                                                    .clicked()
                                                {
                                                    delete_data = Some(ver.clone());
                                                }
                                            } else if ui
                                                .add_enabled(
                                                    !busy,
                                                    egui::Button::new(t.delete),
                                                )
                                                .clicked()
                                            {
                                                delete_idx = Some(i);
                                            }
                                            if ui
                                                .add_enabled(
                                                    !busy,
                                                    egui::Button::new(t.edit),
                                                )
                                                .clicked()
                                            {
                                                edit_idx = Some(i);
                                            }
                                            if !is_active
                                                && ui
                                                    .add_enabled(
                                                        !busy,
                                                        egui::Button::new(t.select),
                                                    )
                                                    .clicked()
                                            {
                                                select_idx = Some(i);
                                            }
                                        },
                                    );
                                });
                            });
                        ui.add_space(5.0);
                    }
                });

            if let Some(idx) = select_idx {
                self.cfg.active_instance = Some(self.cfg.instances[idx].name.clone());
                let _ = save_config(&self.cfg);
                self.toast(ToastKind::Ok, t.instance_updated);
            }
            if let Some(idx) = edit_idx {
                let inst = self.cfg.instances[idx].clone();
                self.inst_dialog = Some(InstDraft::from_instance(idx, &inst));
            }
            if let Some(idx) = delete_idx {
                let inst = &self.cfg.instances[idx];
                self.delete_dialog = Some(DeleteDraft {
                    idx,
                    name: inst.name.clone(),
                    also_folder: false,
                    deletable: inst.game_dir_deletable(),
                });
            }
            if let Some(id) = delete_data {
                if id == "-" {
                    self.toast(ToastKind::Info, t.version_unavailable);
                } else {
                    let dir = crate::install::version_dir(&self.root, &id);
                    if dir.exists() {
                        let _ = std::fs::remove_dir_all(&dir);
                    }
                    self.refresh_installed();
                    self.toast(ToastKind::Ok, t.version_deleted);
                }
            }
            if let Some(idx) = open_folder_idx {
                let inst = &self.cfg.instances[idx];
                let dir = inst.game_dir_for(&self.root);
                let _ = std::fs::create_dir_all(&dir);
                if !open_in_explorer(&dir) {
                    self.toast(
                        ToastKind::Error,
                        t.open_failed.replace("{}", &dir.to_string_lossy()),
                    );
                }
            }
        }

        self.ui_instance_dialog(ui);
        self.ui_delete_dialog(ui);
    }

    fn ui_instance_dialog(&mut self, ui: &mut egui::Ui) {
        let t = self.cfg.t();
        let Some(draft) = self.inst_dialog.take() else {
            return;
        };
        let mut draft = draft;
        let mut open = true;
        let mut save = false;
        let ctx = ui.ctx().clone();

        let title = if draft.editing.is_some() {
            t.edit
        } else {
            t.new_instance
        };
        let modal = egui::Modal::new(egui::Id::new("inst_edit_modal")).show(&ctx, |ui| {
            ui.set_width(440.0);
            ui.label(RichText::new(title).size(15.0).strong());
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(t.instance_name);
                ui.add(
                    egui::TextEdit::singleline(&mut draft.name)
                        .desired_width(240.0),
                );
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(t.mc_version);
                if draft.is_latest {
                    ui.label(
                        RichText::new(t.latest_no_edit)
                            .size(13.0)
                            .color(TEXT_WEAK),
                    );
                } else {
                    let sel = draft.version_id.clone().unwrap_or_default();
                    let mut new_sel = sel.clone();
                    egui::ComboBox::from_id_salt("inst_dialog_version")
                        .selected_text(RichText::new(&sel).size(14.0).color(TEXT))
                        .width(240.0)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show_ui(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut draft.version_search)
                                        .hint_text(t.search_versions)
                                        .desired_width(220.0),
                                );
                            });
                            let filter = draft.version_search.to_lowercase();
                            let list: Vec<&ManifestVersion> = self
                                .visible_versions()
                                .into_iter()
                                .filter(|v| {
                                    filter.is_empty() || v.id.to_lowercase().contains(&filter)
                                })
                                .collect();
                            const ROW_H: f32 = 24.0;
                            egui::ScrollArea::vertical()
                                .max_height(300.0)
                                .show_rows(ui, ROW_H, list.len(), |ui, range| {
                                    for i in range {
                                        let v = list[i];
                                        let kind = if v.kind == "custom" {
                                            t.kind_custom
                                        } else if v.kind == "release" {
                                            "Release"
                                        } else {
                                            v.kind.as_str()
                                        };
                                        let label = RichText::new(format!("{}  [{}]", v.id, kind))
                                            .size(13.0)
                                            .color(TEXT);
                                        ui.selectable_value(&mut new_sel, v.id.clone(), label);
                                    }
                                });
                        });
                    if new_sel != sel {
                        draft.version_id = Some(new_sel);
                        egui::Popup::close_id(
                            ui.ctx(),
                            egui::Id::new("inst_dialog_version").with("popup"),
                        );
                    }
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(t.memory_label);
                ui.add(
                    egui::Slider::new(&mut draft.memory_mb, 1024..=16384)
                        .text("MB")
                        .suffix(" MB"),
                );
            });
            ui.add_space(4.0);
            ui.checkbox(&mut draft.download_java, t.auto_download_java);
            ui.horizontal(|ui| {
                ui.label(t.java_path);
                let path = draft.java_path.get_or_insert_with(|| String::new());
                ui.add(
                    egui::TextEdit::singleline(path).desired_width(240.0),
                );
                if ui.button(t.detect).clicked() {
                    if let Some(p) =
                        crate::java::detect_java(draft.java_path.as_deref(), &self.root)
                    {
                        draft.java_path = Some(p.to_string_lossy().into_owned());
                    }
                }
                if ui.button(t.clear).clicked() {
                    draft.java_path = None;
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(t.resolution);
                ui.add(egui::DragValue::new(&mut draft.width).range(320..=7680));
                ui.label("x");
                ui.add(egui::DragValue::new(&mut draft.height).range(240..=4320));
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(t.extra_jvm_args_label);
                ui.add(
                    egui::TextEdit::singleline(&mut draft.extra_jvm_args)
                        .desired_width(240.0)
                        .hint_text(t.extra_jvm_args_hint),
                );
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(t.authlib_url_label);
                ui.add(
                    egui::TextEdit::singleline(&mut draft.authlib_url)
                        .desired_width(200.0)
                        .hint_text("ely.by"),
                );
            });
            ui.add_space(8.0);
            ui.label(RichText::new(t.game_dir_label).strong().size(13.0));
            ui.horizontal(|ui| {
                let selected = match draft.game_dir_mode {
                    crate::config::GameDirMode::Launcher => t.game_dir_launcher,
                    crate::config::GameDirMode::Original => t.game_dir_original,
                    crate::config::GameDirMode::Custom => t.game_dir_custom,
                };
                let mut new_mode = draft.game_dir_mode.clone();
                egui::ComboBox::from_id_salt("inst_dialog_gamedir")
                    .selected_text(RichText::new(selected).size(13.0).color(TEXT))
                    .width(240.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut new_mode,
                            crate::config::GameDirMode::Launcher,
                            t.game_dir_launcher,
                        );
                        ui.selectable_value(
                            &mut new_mode,
                            crate::config::GameDirMode::Original,
                            t.game_dir_original,
                        );
                        ui.selectable_value(
                            &mut new_mode,
                            crate::config::GameDirMode::Custom,
                            t.game_dir_custom,
                        );
                    });
                if new_mode != draft.game_dir_mode {
                    draft.game_dir_mode = new_mode;
                }
            });
            if draft.game_dir_mode == crate::config::GameDirMode::Custom {
                ui.horizontal(|ui| {
                    ui.label(t.game_dir_path_label);
                    let path = draft.game_dir.get_or_insert_with(String::new);
                    ui.add(
                        egui::TextEdit::singleline(path).desired_width(180.0),
                    );
                    if ui.button(t.browse_folder).clicked() {
                        if let Some(picked) = rfd::FileDialog::new()
                            .set_title(t.browse_folder)
                            .pick_folder()
                        {
                            let custom = pick_folder_with_instance_subdir(picked, &draft.name);
                            draft.game_dir = Some(custom.to_string_lossy().into_owned());
                        }
                    }
                    if ui.button(t.clear).clicked() {
                        draft.game_dir = None;
                    }
                });
            }
            ui.label(
                RichText::new(t.game_dir_hint)
                    .color(TEXT_WEAK)
                    .size(10.5),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(t.save).clicked() {
                    save = true;
                }
                if ui.button(t.cancel).clicked() {
                    open = false;
                }
            });
        });
        // Close only via the Cancel button or Escape - a click on the backdrop
        // must NOT close the dialog (the modal blocks the rest of the UI).
        let esc = modal.is_top_modal
            && !modal.any_popup_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if esc {
            open = false;
        }

        if save {
            let name = draft.name.trim().to_string();
            let is_latest_edit = draft
                .editing
                .map(|idx| self.cfg.instances[idx].is_latest)
                .unwrap_or(false);
            if name == crate::config::LATEST_INSTANCE_KEY && !is_latest_edit {
                self.toast(ToastKind::Error, t.latest_key_reserved);
                self.inst_dialog = Some(draft);
                return;
            }
            let name_taken = name.is_empty()
                || self
                    .cfg
                    .instances
                    .iter()
                    .enumerate()
                    .any(|(i, x)| x.name == name && Some(i) != draft.editing);
            if name_taken {
                self.toast(ToastKind::Error, t.name_taken);
                self.inst_dialog = Some(draft);
                return;
            }
            if let Some(idx) = draft.editing {
                let old_name = self.cfg.instances[idx].name.clone();
                let was_active = self.cfg.active_instance.as_deref() == Some(old_name.as_str());
                let inst = &mut self.cfg.instances[idx];
                inst.name = name.clone();
                if !inst.is_latest {
                    inst.version_id = draft.version_id.clone();
                }
                inst.memory_mb = draft.memory_mb;
                inst.java_path = draft.java_path.clone();
                inst.download_java = draft.download_java;
                inst.width = draft.width;
                inst.height = draft.height;
                inst.extra_jvm_args = draft.extra_jvm_args.clone();
                inst.authlib_url = draft.authlib_url.clone();
                inst.game_dir_mode = draft.game_dir_mode.clone();
                inst.game_dir = draft.game_dir.clone();
                if was_active {
                    self.cfg.active_instance = Some(name);
                }
                self.toast(ToastKind::Ok, t.instance_updated);
            } else {
                let mut inst = Instance::new(name);
                inst.version_id = draft.version_id;
                inst.memory_mb = draft.memory_mb;
                inst.java_path = draft.java_path;
                inst.download_java = draft.download_java;
                inst.width = draft.width;
                inst.height = draft.height;
                inst.extra_jvm_args = draft.extra_jvm_args;
                inst.authlib_url = draft.authlib_url;
                inst.game_dir_mode = draft.game_dir_mode;
                inst.game_dir = draft.game_dir;
                let idx = self.cfg.instances.len();
                self.cfg.instances.push(inst);
                self.cfg.active_instance = Some(self.cfg.instances[idx].name.clone());
                self.toast(ToastKind::Ok, t.instance_created);
            }
            let _ = save_config(&self.cfg);
            // Success -> close the dialog.
            self.inst_dialog = None;
            return;
        }
        if !open {
            self.inst_dialog = None;
        } else {
            // Still open -> keep the draft so the dialog survives to the next frame.
            self.inst_dialog = Some(draft);
        }
    }

    fn ui_delete_dialog(&mut self, ui: &mut egui::Ui) {
        let t = self.cfg.t();
        let Some(mut draft) = self.delete_dialog.take() else {
            return;
        };
        let mut open = true;
        let mut do_delete = false;
        let ctx = ui.ctx().clone();

        let modal = egui::Modal::new(egui::Id::new("inst_delete_modal")).show(&ctx, |ui| {
            ui.set_width(420.0);
            ui.label(RichText::new(t.delete_confirm_title).size(15.0).strong());
            ui.add_space(6.0);
            ui.label(
                RichText::new(t.delete_confirm_body.replace("{}", &draft.name))
                    .size(12.5),
            );
            if draft.deletable {
                ui.add_space(6.0);
                ui.checkbox(&mut draft.also_folder, t.delete_confirm_also_folder);
            } else {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(t.delete_folder_locked)
                        .color(TEXT_WEAK)
                        .size(11.0),
                );
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(t.delete).clicked() {
                    do_delete = true;
                }
                if ui.button(t.cancel).clicked() {
                    open = false;
                }
            });
        });
        if modal.should_close() {
            open = false;
        }

        if do_delete {
            let was_active =
                self.cfg.active_instance.as_deref() == Some(draft.name.as_str());
            if draft.also_folder && draft.deletable {
                let dir = self.cfg.instances[draft.idx].game_dir_for(&self.root);
                if dir.exists() {
                    let _ = std::fs::remove_dir_all(&dir);
                }
                self.toast(ToastKind::Ok, t.folder_deleted);
            }
            self.cfg.instances.remove(draft.idx);
            if was_active {
                self.cfg.normalize_active_instance();
            }
            let _ = save_config(&self.cfg);
            self.toast(ToastKind::Ok, t.instance_deleted);
        } else if !open {
            self.delete_dialog = None;
        } else {
            self.delete_dialog = Some(draft);
        }
    }

    /// Whether at least one Ely.by account with a non-expired token exists.
    /// Offline accounts require this to stay unlocked.
    fn has_valid_online(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.cfg.accounts.iter().any(|a| {
            a.account_type != crate::config::ACCOUNT_TYPE_OFFLINE
                && a.expires_at.map(|e| e > now).unwrap_or(true)
        })
    }

    fn ui_accounts(&mut self, ui: &mut egui::Ui) {
        let t = self.cfg.t();
        let busy = self.busy();

        // ---- Add account (always visible at the top, compact) ----
        ui.label(RichText::new(t.add_account).size(9.0).color(TEXT_WEAK));
        ui.add_space(4.0);

        let online_ok = self.has_valid_online();
        let any_busy = self.task_active;

        card(ui, |ui| {
            ui.horizontal(|ui| {
                // LEFT: Ely.by OAuth login
                ui.vertical(|ui| {
                    ui.add_space(2.0);
                    ui.label(RichText::new(t.login_with_ely).strong().size(12.5));
                    ui.label(
                        RichText::new(t.login_ely_account)
                            .color(TEXT_WEAK)
                            .size(10.5),
                    );
                    ui.add_space(5.0);
                    if ui
                        .add_enabled(
                            !any_busy,
                            egui::Button::new(
                                RichText::new(t.ely_oauth)
                                    .size(13.0)
                                    .strong()
                                    .color(PRIMARY_TEXT),
                            )
                            .fill(PRIMARY_BTN)
                            .corner_radius(6.0),
                        )
                        .clicked()
                    {
                        self.start(|tx, drx| crate::tasks::login_oauth(tx, drx));
                    }
                    ui.add_space(4.0);
                    ui.hyperlink_to(
                        RichText::new(t.register_ely).size(10.5).color(ACCENT),
                        crate::util::ELY_REGISTER_URL,
                    );
                });
                ui.separator();
                // RIGHT: Add offline account. Requires a valid Ely.by account.
                ui.vertical(|ui| {
                    ui.add_space(2.0);
                    ui.label(RichText::new(t.offline_account).strong().size(12.5));
                    ui.label(
                        RichText::new(t.offline_desc)
                            .color(TEXT_WEAK)
                            .size(10.5),
                    );
                    ui.add_space(5.0);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.offline_name)
                                .hint_text(t.offline_name)
                                .desired_width(150.0),
                        );
                        if ui
                            .add_enabled(
                                online_ok && !any_busy,
                                egui::Button::new(t.add_offline),
                            )
                            .clicked()
                        {
                            let name = self.offline_name.trim().to_string();
                            let valid = (3..=16).contains(&name.len())
                                && name
                                    .chars()
                                    .all(|c| c.is_alphanumeric() || c == '_');
                            if !valid {
                                self.toast(ToastKind::Error, t.offline_name_invalid);
                            } else {
                                self.offline_name.clear();
                                self.add_or_replace_account(crate::auth::offline_account(&name));
                            }
                        }
                    });
                    if !online_ok {
                        ui.label(
                            RichText::new(t.offline_locked)
                                .color(ERR_RED)
                                .size(10.5),
                        );
                    }
                });
            });
        });

        ui.add_space(8.0);

        card(ui, |ui| {
            egui::CollapsingHeader::new(
                RichText::new(t.password_title).strong().size(13.0),
            )
            .default_open(false)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.pw_username)
                        .hint_text(t.email_or_username)
                        .desired_width(300.0),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.pw_password)
                        .hint_text(t.password)
                        .password(!self.pw_show)
                        .desired_width(300.0),
                );
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&mut self.pw_show, t.show_password)
                        .changed()
                    {
                        let _ = self.pw_show;
                    }
                    if ui
                        .add_enabled(!self.task_active, egui::Button::new(t.sign_in))
                        .clicked()
                    {
                        let username = self.pw_username.trim().to_string();
                        let password = self.pw_password.clone();
                        if username.is_empty() || password.is_empty() {
                            self.toast(ToastKind::Error, t.fill_credentials);
                        } else {
                            self.start(move |tx, drx| {
                                crate::tasks::login_password(tx, drx, username, password, None);
                            });
                        }
                    }
                });
            });
        });

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        // ---- Saved accounts ----
        if self.cfg.accounts.is_empty() {
            card(ui, |ui| {
                ui.label(RichText::new(t.no_saved_accounts).color(TEXT_WEAK));
            });
            ui.add_space(10.0);
        } else {
            ui.label(RichText::new(t.saved_accounts).size(9.0).color(TEXT_WEAK));
            ui.add_space(4.0);

            let n = self.cfg.accounts.len();
            let mut remove_idx: Option<usize> = None;
            let mut activate_idx: Option<usize> = None;

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for i in 0..n {
                        let is_active = self.cfg.active_account_index == Some(i);
                        let acc = self.cfg.accounts[i].clone();
                        let offline_locked = !online_ok
                            && acc.account_type == crate::config::ACCOUNT_TYPE_OFFLINE;

                        egui::Frame::default()
                            .fill(if is_active && !offline_locked { ACCENT_SOFT } else { GLASS })
                            .stroke(egui::Stroke::new(
                                1.0,
                                if is_active && !offline_locked {
                                    ACCENT_SOFT_STROKE
                                } else {
                                    BORDER
                                },
                            ))
                            .corner_radius(8.0)
                            .inner_margin(egui::Margin::symmetric(10, 7))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    if let Some(tex) = self.avatars.get(&acc.uuid) {
                                        ui.image((tex.id(), Vec2::new(34.0, 34.0)));
                                    } else {
                                        ui.allocate_ui(Vec2::new(34.0, 34.0), |ui| {
                                            ui.centered_and_justified(|ui| {
                                                ui.label(
                                                    RichText::new(
                                                        acc.username
                                                            .chars()
                                                            .next()
                                                            .unwrap_or('?')
                                                            .to_string(),
                                                    )
                                                    .size(14.0)
                                                    .strong(),
                                                );
                                            });
                                        });
                                    }
                                    ui.add_space(8.0);
                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(&acc.username)
                                                    .size(13.5)
                                                    .strong(),
                                            );
                                            if is_active {
                                                ui.label(
                                                    RichText::new(t.active_badge)
                                                        .size(9.0)
                                                        .color(ACCENT)
                                                        .strong(),
                                                );
                                            }
                                            let ty = if acc.account_type
                                                == crate::config::ACCOUNT_TYPE_ELY_OAUTH
                                            {
                                                t.ely_oauth
                                            } else if acc.account_type
                                                == crate::config::ACCOUNT_TYPE_ELY_PASSWORD
                                            {
                                                t.ely_password
                                            } else {
                                                t.offline_account
                                            };
                                            ui.label(
                                                RichText::new(ty)
                                                    .color(TEXT_WEAK)
                                                    .size(10.5),
                                            );
                                        });
                                        if offline_locked {
                                            ui.label(
                                                RichText::new(t.offline_locked)
                                                    .color(ERR_RED)
                                                    .size(10.5),
                                            );
                                        }
                                        if let Some(exp) = acc.expires_at {
                                            let now = std::time::SystemTime::now()
                                                .duration_since(std::time::UNIX_EPOCH)
                                                .unwrap_or_default()
                                                .as_secs() as i64;
                                            let left = exp - now;
                                            let detail = t
                                                .token_expires_on
                                                .replace("{}", &format_expiry_date(exp))
                                                .replacen(
                                                    "{}",
                                                    &format_time_left(left.max(0), t),
                                                    1,
                                                );
                                            let color = if left <= 0 {
                                                ERR_RED
                                            } else if left < 86400 {
                                                Color32::from_rgb(235, 180, 60)
                                            } else {
                                                TEXT_WEAK
                                            };
                                            ui.label(
                                                RichText::new(detail)
                                                    .color(color)
                                                    .size(11.0),
                                            );
                                        }
                                    });
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .add_enabled(!busy, egui::Button::new(t.remove))
                                                .clicked()
                                            {
                                                remove_idx = Some(i);
                                            }
                                            if !is_active
                                                && !offline_locked
                                                && ui
                                                    .add_enabled(
                                                        !busy,
                                                        egui::Button::new(t.activate),
                                                    )
                                                    .clicked()
                                            {
                                                activate_idx = Some(i);
                                            }
                                        },
                                    );
                                });
                            });
                        ui.add_space(5.0);
                    }
                });

            if let Some(idx) = activate_idx {
                self.cfg.active_account_index = Some(idx);
                let _ = save_config(&self.cfg);
                let uuid = self.cfg.accounts[idx].uuid.clone();
                let (_, drx) = new_channel::<LaunchDecision>();
                crate::tasks::fetch_avatar(self.tx.clone(), drx, uuid);
                self.toast(
                    ToastKind::Ok,
                    t.account_active
                        .replace("{}", &self.cfg.accounts[idx].username),
                );
            }
            if let Some(idx) = remove_idx {
                self.remove_account(idx);
            }
        }
    }

    fn ui_settings(&mut self, ui: &mut egui::Ui) {
        let t = self.cfg.t();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                card(ui, |ui| {
                    ui.label(RichText::new(t.data_dir_label).size(9.0).color(TEXT_WEAK));
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(t.data_dir_choice);
                        let mut mode = self.cfg.data_dir_mode.clone();
                        egui::ComboBox::from_id_salt("data_dir_mode")
                            .selected_text(RichText::new(match mode {
                                crate::config::DataDirMode::Launcher => t.data_dir_launcher,
                                crate::config::DataDirMode::Original => t.data_dir_original,
                                crate::config::DataDirMode::Custom => t.data_dir_custom,
                            }))
                            .width(250.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut mode,
                                    crate::config::DataDirMode::Launcher,
                                    t.data_dir_launcher,
                                );
                                ui.selectable_value(
                                    &mut mode,
                                    crate::config::DataDirMode::Original,
                                    t.data_dir_original,
                                );
                                ui.selectable_value(
                                    &mut mode,
                                    crate::config::DataDirMode::Custom,
                                    t.data_dir_custom,
                                );
                            });
                        if mode != self.cfg.data_dir_mode {
                            self.cfg.data_dir_mode = mode;
                            self.apply_data_root();
                            let _ = save_config(&self.cfg);
                            self.toast(ToastKind::Info, t.data_dir_changed);
                        }
                    });
                    if self.cfg.data_dir_mode == crate::config::DataDirMode::Custom {
                        ui.horizontal(|ui| {
                            ui.label(t.data_dir_path_label);
                            let path = self.cfg.data_dir.get_or_insert_with(String::new);
                            ui.add(egui::TextEdit::singleline(path).desired_width(300.0));
                        });
                        ui.horizontal(|ui| {
                            if ui.button(t.browse_folder).clicked() {
                                let start = self
                                    .cfg
                                    .data_dir
                                    .clone()
                                    .filter(|p| !p.is_empty())
                                    .map(PathBuf::from)
                                    .unwrap_or_else(|| PathBuf::from("."));
                                if let Some(picked) = rfd::FileDialog::new()
                                    .set_title(t.browse_folder)
                                    .set_directory(start)
                                    .pick_folder()
                                {
                                    self.cfg.data_dir = Some(picked.to_string_lossy().into_owned());
                                    self.cfg.data_dir_mode = crate::config::DataDirMode::Custom;
                                    self.apply_data_root();
                                    let _ = save_config(&self.cfg);
                                    self.toast(ToastKind::Info, t.data_dir_changed);
                                }
                            }
                        });
                    }
                    ui.label(RichText::new(t.data_dir_hint).color(TEXT_WEAK).size(10.5));
                    ui.label(
                        RichText::new(
                            t.data_dir_current.replace("{}", &self.root.to_string_lossy()),
                        )
                        .color(TEXT_WEAK)
                        .size(10.5),
                    );
                });

                ui.add_space(8.0);

                card(ui, |ui| {
                    ui.label(RichText::new(t.vanilla_branding_label).size(9.0).color(TEXT_WEAK));
                    ui.add_space(4.0);
                    if ui
                        .checkbox(&mut self.cfg.vanilla_branding, t.vanilla_branding_label)
                        .changed()
                    {
                        let _ = save_config(&self.cfg);
                        self.toast(ToastKind::Info, t.saved);
                    }
                    ui.label(
                        RichText::new(t.vanilla_branding_hint)
                            .color(TEXT_WEAK)
                            .size(10.5),
                    );
                });

                ui.add_space(8.0);

                card(ui, |ui| {
                    ui.label(RichText::new(t.versions_label).size(9.0).color(TEXT_WEAK));
                ui.add_space(4.0);
                if ui
                    .checkbox(&mut self.cfg.show_all_versions, t.show_all_versions)
                    .changed()
                {
                    let _ = save_config(&self.cfg);
                }
                if ui
                    .checkbox(&mut self.cfg.show_custom_clients, t.show_custom_clients)
                    .changed()
                {
                    let _ = save_config(&self.cfg);
                }
                ui.label(
                    RichText::new(t.custom_clients_hint)
                        .color(TEXT_WEAK)
                        .size(10.5),
                );
            });

            ui.add_space(8.0);

            card(ui, |ui| {
                ui.label(RichText::new(t.language_label).size(9.0).color(TEXT_WEAK));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(t.language_choice);
                    let mut lang = self.cfg.language.clone();
                    egui::ComboBox::from_id_salt("lang_choice")
                        .selected_text(RichText::new(
                            if lang == crate::config::LANG_EN {
                                t.english
                            } else {
                                t.indonesian
                            },
                        ))
                        .width(160.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut lang,
                                crate::config::LANG_ID.to_string(),
                                t.indonesian,
                            );
                            ui.selectable_value(
                                &mut lang,
                                crate::config::LANG_EN.to_string(),
                                t.english,
                            );
                        });
                    if lang != self.cfg.language {
                        self.cfg.language = lang;
                        let _ = save_config(&self.cfg);
                        crate::lang::set_current(self.cfg.t());
                        self.toast(ToastKind::Info, self.cfg.t().saved);
                    }
                });
            });

            ui.add_space(8.0);

            card(ui, |ui| {
                ui.label(RichText::new(t.font_label).size(9.0).color(TEXT_WEAK));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(t.font_choice);
                    let mut font_mode = self.cfg.font_mode.clone();
                    egui::ComboBox::from_id_salt("font_choice")
                        .selected_text(RichText::new(if font_mode == crate::config::FontMode::Monogram {
                            t.font_monogram
                        } else {
                            t.font_system
                        }))
                        .width(160.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut font_mode,
                                crate::config::FontMode::Monogram,
                                t.font_monogram,
                            );
                            ui.selectable_value(
                                &mut font_mode,
                                crate::config::FontMode::System,
                                t.font_system,
                            );
                        });
                    if font_mode != self.cfg.font_mode {
                        self.cfg.font_mode = font_mode;
                        let _ = save_config(&self.cfg);
                        apply_font_mode(ui.ctx(), &self.cfg.font_mode);
                        self.toast(ToastKind::Info, self.cfg.t().saved);
                    }
                });
            });
        });
    }

    fn ui_console(&mut self, ui: &mut egui::Ui) {
        let t = self.cfg.t();
        ui.horizontal(|ui| {
            if ui.button(t.console_clear).clicked() {
                self.console.clear();
            }
            if ui.button(t.console_save).clicked() {
                let logs_dir = self.root.join("logs");
                let _ = std::fs::create_dir_all(&logs_dir);
                let path = logs_dir.join(format!("console-{}.log", now_stamp()));
                if std::fs::write(&path, &self.console).is_ok() {
                    self.toast(
                        ToastKind::Ok,
                        t.log_saved.replace("{}", &path.to_string_lossy()),
                    );
                } else {
                    self.toast(
                        ToastKind::Error,
                        t.save_failed.replace("{}", &path.to_string_lossy()),
                    );
                }
            }
            if ui.button(t.copy).clicked() {
                ui.ctx().copy_text(self.console.clone());
                self.toast(ToastKind::Info, t.log_copied);
            }
            if ui.button(t.open_logs_folder).clicked() {
                let logs_dir = self.root.join("logs");
                let _ = std::fs::create_dir_all(&logs_dir);
                if !open_in_explorer(&logs_dir) {
                    self.toast(
                        ToastKind::Error,
                        t.open_failed.replace("{}", &logs_dir.to_string_lossy()),
                    );
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut self.console_scroll, t.auto_scroll);
            });
        });
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(self.console_scroll)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.console)
                        .code_editor()
                        .desired_rows(30)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                );
            });
    }

    #[allow(dead_code)]
    fn http_client(&self) -> reqwest::blocking::Client {
        reqwest::blocking::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(20))
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new())
    }
}

impl eframe::App for HanaApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events(ctx);
        if !self.versions_loaded && !self.task_active {
            self.start_refresh_versions();
        }
        self.update_toasts();
        // Only repaint continuously while something is happening (progress,
        // spinner or fading toasts). Idle frames cost almost nothing.
        if self.task_active || self.running_pid.is_some() || !self.toasts.is_empty() {
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}
