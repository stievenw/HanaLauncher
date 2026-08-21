use super::super::{HanaApp, ToastKind};
use super::*;
use crate::worker::LaunchDecision;
use eframe::egui::{self, RichText};

// ── Warn: already running ───────────────────────────────────────────────────

pub(super) fn warn_existing(app: &HanaApp, ctx: &egui::Context) {
    let t = app.cfg.t();
    let mut close = false;
    egui::Window::new("HanaLauncher")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
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
}

// ── Version choice dialog ───────────────────────────────────────────────────

pub(super) fn version_choice(app: &HanaApp, ctx: &egui::Context) -> Option<LaunchDecision> {
    let (newest, current) = app.version_choice.clone()?;
    let t = app.cfg.t();
    let mut chosen: Option<LaunchDecision> = None;
    egui::Window::new(t.update_available)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
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
    chosen
}

// ── Two-factor authentication dialog ────────────────────────────────────────

pub(super) fn two_factor(app: &mut HanaApp, ctx: &egui::Context) {
    if !app.need_2fa {
        return;
    }
    let t = app.cfg.t();
    let mut close = false;
    let mut submit = false;
    egui::Modal::new(egui::Id::new("twofa_modal")).show(ctx, |ui| {
        ui.set_width(320.0);
        ui.label(RichText::new(t.twofa_title).size(15.0).strong());
        ui.add_space(6.0);
        ui.label(t.twofa_hint);
        ui.add(
            egui::TextEdit::singleline(&mut app.twofa_input)
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
        let code = app.twofa_input.trim().to_string();
        if code.is_empty() {
            app.toast(ToastKind::Error, t.twofa_empty);
        } else {
            let username = app.pw_username.clone();
            let password = app.pw_password.clone();
            app.start(move |tx, drx| {
                crate::tasks::login_password(tx, drx, username, password, Some(code));
            });
            app.need_2fa = false;
            app.twofa_input.clear();
        }
    }
    if close {
        app.need_2fa = false;
        app.twofa_input.clear();
    }
}

// ── Device code (OAuth) dialog ──────────────────────────────────────────────

pub(super) fn device_code(app: &mut HanaApp, ctx: &egui::Context) {
    let Some((code, verification_uri)) = app.device_code.clone() else {
        return;
    };
    let t = app.cfg.t();
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::new("oauth_modal"))
        .show(ctx, |ui| {
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
                        app.toast(ToastKind::Error, t.cannot_open_browser);
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
    let esc = modal.is_top_modal
        && !modal.any_popup_open
        && ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
        });
    if close || esc {
        app.device_code = None;
        if let Some(tx) = app.decision_tx.take() {
            let _ = tx.send(LaunchDecision::Cancel);
        }
        app.toast(ToastKind::Info, t.login_cancelled);
    }
}
