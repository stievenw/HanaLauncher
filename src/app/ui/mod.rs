mod accounts;
mod chrome;
mod console;
mod dialogs;
mod home;
mod installations;
mod settings;
mod versions;

use super::{HanaApp, Page};
use eframe::egui::{self, Color32};

// ── Constants ───────────────────────────────────────────────────────────────

pub(super) const MONOGRAM_SCALE: f32 = 1.12;

pub(super) const ACCENT: Color32 = Color32::from_rgb(255, 130, 45);
pub(super) const ACCENT_SOFT: Color32 = Color32::from_rgba_premultiplied(243, 229, 206, 236);
pub(super) const ACCENT_SOFT_STROKE: Color32 = Color32::from_rgb(203, 168, 122);
pub(super) const PRIMARY_BTN: Color32 = Color32::from_rgb(228, 100, 22);
pub(super) const PRIMARY_TEXT: Color32 = Color32::from_rgb(62, 44, 30);
pub(super) const OK_GREEN: Color32 = Color32::from_rgb(46, 150, 70);
pub(super) const WARN_YELLOW: Color32 = Color32::from_rgb(206, 132, 0);
pub(super) const ERR_RED: Color32 = Color32::from_rgb(214, 60, 48);
pub(super) const BG_TOP: Color32 = Color32::from_rgb(112, 154, 216);
pub(super) const BG_BOTTOM: Color32 = Color32::from_rgb(255, 197, 122);
pub(super) const GLASS: Color32 = Color32::from_rgba_premultiplied(255, 249, 240, 236);
pub(super) const GLASS_NAV: Color32 = Color32::from_rgba_premultiplied(255, 249, 240, 198);
pub(super) const GLASS_SOLID: Color32 = Color32::from_rgb(255, 249, 240);
pub(super) const HOVER: Color32 = Color32::from_rgb(250, 227, 198);
pub(super) const BORDER: Color32 = Color32::from_rgb(226, 190, 150);
pub(super) const TEXT: Color32 = Color32::from_rgb(62, 44, 30);
pub(super) const TEXT_WEAK: Color32 = Color32::from_rgb(142, 112, 86);

// ── Helpers ─────────────────────────────────────────────────────────────────

pub(super) fn paint_vgrad(
    painter: &egui::Painter,
    rect: egui::Rect,
    top: Color32,
    bottom: Color32,
) {
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

pub(super) fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
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

pub(super) fn format_expiry_date(unix: i64) -> String {
    use chrono::{DateTime, Local};
    DateTime::from_timestamp(unix, 0)
        .map(|u| u.with_timezone(&Local))
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| Local::now().format("%Y-%m-%d %H:%M").to_string())
}

pub(super) fn format_time_left(secs: i64, t: &crate::lang::Lang) -> String {
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

pub(super) fn now_stamp() -> String {
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

pub(super) fn civil_from_days(z: i64) -> (i64, u32, u32) {
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

pub(super) fn open_in_explorer(path: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        std::process::Command::new("explorer").arg(path).spawn().is_ok()
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open").arg(path).spawn().is_ok()
    }
}

pub(super) fn apply_font_mode(ctx: &egui::Context, mode: &crate::config::FontMode) {
    let mut fonts = egui::FontDefinitions::default();
    if *mode == crate::config::FontMode::Monogram {
        let regular = crate::assets::monogram_ttf();
        let italic = crate::assets::monogram_italic_ttf();
        if let (Some(regular), Some(italic)) = (regular, italic) {
            let mut regular = egui::FontData::from_owned(regular);
            regular.tweak.scale = MONOGRAM_SCALE;
            let mut italic = egui::FontData::from_owned(italic);
            italic.tweak.scale = MONOGRAM_SCALE;
            fonts
                .font_data
                .insert("monogram".to_owned(), regular.into());
            fonts
                .font_data
                .insert("monogram_italic".to_owned(), italic.into());
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

// ── Draw orchestration ──────────────────────────────────────────────────────

pub(crate) fn draw(app: &mut HanaApp, root_ui: &mut egui::Ui) {
    let ctx = root_ui.ctx().clone();

    // Pre-layout dialogs (float above everything)
    if app.warn_existing {
        dialogs::warn_existing(app, &ctx);
    }
    if let Some(decision) = dialogs::version_choice(app, &ctx) {
        if let Some(tx) = &app.decision_tx {
            let _ = tx.send(decision);
        }
        app.version_choice = None;
    }

    // Background
    chrome::paint_background(app, &ctx);

    // Titlebar
    chrome::paint_titlebar(app, &ctx, root_ui);

    // Sidebar
    chrome::paint_sidebar(app, root_ui);

    // Top bar
    chrome::paint_top_bar(app, root_ui);

    // Status bar
    chrome::paint_status_bar(app, root_ui);

    // Content
    egui::CentralPanel::default()
        .frame(egui::Frame::default().fill(Color32::TRANSPARENT))
        .show(root_ui, |ui| {
            ui.add_space(6.0);
            match app.page {
                Page::Home => app.ui_home(ui),
                Page::Versions => app.ui_versions(ui),
                Page::Installations => app.ui_installations(ui),
                Page::Accounts => app.ui_accounts(ui),
                Page::Settings => app.ui_settings(ui),
                Page::Console => app.ui_console(ui),
            }
        });

    chrome::handle_resize(&ctx);
    chrome::paint_toasts(app, &ctx);
    dialogs::two_factor(app, &ctx);
    dialogs::device_code(app, &ctx);
    app.update_toasts();
}
