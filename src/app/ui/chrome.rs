use super::super::{HanaApp, Page, ToastKind};
use super::*;
use eframe::egui::{self, Align2, Color32, RichText, Vec2};

// ── Background ──────────────────────────────────────────────────────────────

pub(super) fn paint_background(app: &HanaApp, ctx: &egui::Context) {
    let painter = ctx.layer_painter(egui::LayerId::background());
    let rect = ctx.input(|i| i.viewport_rect());
    match &app.bg {
        Some(bg) => {
            let iw = bg.size_vec2().x;
            let ih = bg.size_vec2().y;
            let scale = (rect.width() / iw).max(rect.height() / ih).max(0.001);
            let dw = iw * scale;
            let dh = ih * scale;
            let offset = egui::vec2((rect.width() - dw) * 0.5, (rect.height() - dh) * 0.5);
            let draw_rect = egui::Rect::from_min_size(rect.min + offset, egui::vec2(dw, dh));
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

// ── Titlebar ────────────────────────────────────────────────────────────────

pub(super) fn paint_titlebar(
    _app: &mut HanaApp,
    ctx: &egui::Context,
    root_ui: &mut egui::Ui,
) {
    let bar_h = 36.0;
    egui::Panel::top("titlebar")
        .frame(
            egui::Frame::default()
                .fill(PRIMARY_BTN)
                .inner_margin(egui::Margin::ZERO),
        )
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
            if let Some(logo) = &_app.logo {
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
                Align2::LEFT_CENTER,
                "HanaLauncher",
                egui::FontId::proportional(14.0),
                PRIMARY_TEXT,
            );

            // Window control buttons
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
}

// ── Sidebar ─────────────────────────────────────────────────────────────────

pub(super) fn paint_sidebar(app: &mut HanaApp, root_ui: &mut egui::Ui) {
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
                });
            });

            ui.add_space(14.0);
            let t = app.cfg.t();
            let pages = [
                (t.nav_home, Page::Home),
                (t.nav_versions, Page::Versions),
                (t.nav_installations, Page::Installations),
                (t.nav_accounts, Page::Accounts),
                (t.nav_settings, Page::Settings),
                (t.nav_console, Page::Console),
            ];
            for (label, page) in pages {
                let selected = app.page == page;
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
                if ui
                    .add_sized(Vec2::new(150.0, 30.0), button)
                    .clicked()
                {
                    app.page = page;
                }
            }

            let version_name = app
                .active_resolved_version()
                .unwrap_or_else(|| "-".to_string());
            let inst_memory = app
                .cfg
                .active_installation()
                .map(|i| i.memory_mb)
                .unwrap_or(2048);
            let installed = app.active_version_installed();

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
}

// ── Top bar ─────────────────────────────────────────────────────────────────

pub(super) fn paint_top_bar(app: &HanaApp, root_ui: &mut egui::Ui) {
    egui::Panel::top("top_bar")
        .frame(egui::Frame::default().fill(GLASS_NAV))
        .show(root_ui, |ui| {
            ui.horizontal(|ui| {
                let t = app.cfg.t();
                let title = match app.page {
                    Page::Home => t.nav_home,
                    Page::Versions => t.nav_versions,
                    Page::Installations => t.nav_installations,
                    Page::Accounts => t.nav_accounts,
                    Page::Settings => t.nav_settings,
                    Page::Console => t.nav_console,
                };
                ui.label(RichText::new(title).size(16.0).strong());
                if app.busy() {
                    ui.add_space(10.0);
                    ui.add(egui::Spinner::new().size(14.0).color(ACCENT));
                    ui.label(RichText::new(&app.task_stage).size(12.0).color(TEXT_WEAK));
                }
            });
        });
}

// ── Status bar ──────────────────────────────────────────────────────────────

pub(super) fn paint_status_bar(app: &HanaApp, root_ui: &mut egui::Ui) {
    egui::Panel::bottom("status_bar")
        .frame(egui::Frame::default().fill(GLASS_NAV))
        .show(root_ui, |ui| {
            if app.task_active {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(13.0).color(ACCENT));
                    ui.label(RichText::new(&app.task_stage).size(11.0).color(TEXT_WEAK));
                    if let Some((current, total)) = app.task_progress {
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
            } else if let Some(pid) = app.running_pid {
                ui.label(
                    RichText::new(app.cfg.t().game_running.replace("{}", &pid.to_string()))
                        .size(11.0)
                        .color(OK_GREEN),
                );
            } else {
                let t = app.cfg.t();
                ui.label(RichText::new(t.home_ready).size(11.0).color(TEXT_WEAK));
            }
        });
}

// ── Toasts ──────────────────────────────────────────────────────────────────

pub(super) fn paint_toasts(app: &HanaApp, ctx: &egui::Context) {
    egui::Area::new(egui::Id::new("toasts"))
        .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-16.0, -52.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let t = app.cfg.t();
            for t_toast in &app.toasts {
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
                                    RichText::new(&t_toast.msg)
                                        .size(13.0)
                                        .color(TEXT),
                                );
                            });
                        });
                    });
                ui.add_space(6.0);
            }
        });
}

// ── Resize edges ────────────────────────────────────────────────────────────

pub(super) fn handle_resize(ctx: &egui::Context) {
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
                let resp = ui.interact(
                    srect,
                    egui::Id::new(("resize_edge", name)),
                    egui::Sense::drag(),
                );
                if resp.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
                }
            }
        });
}
